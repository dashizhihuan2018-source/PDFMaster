use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};

use crate::document::{
    MAX_PAGES, PdfMasterError, Result, Session, materialize_page_attributes, page_object_ids,
    serialize, validate_page_index,
};

const MAX_COPIED_OBJECTS: usize = 65_536;
const MAX_CONTAINER_DEPTH: usize = 64;
const MAX_GRAPH_ITEMS: usize = 1_000_000;

impl Session {
    /// Read-only extraction in source-page order, matching the original IndexSet
    /// workflow. Builds a fresh reachable graph, not a page-tree-only deletion.
    /// Reachable resources/metadata are preserved; this is not secure erasure.
    pub fn extract_pages(&self, selected: &[usize]) -> Result<Vec<u8>> {
        if selected.is_empty() {
            return Err(PdfMasterError::InvalidOperation(
                "page selection cannot be empty",
            ));
        }
        if selected.len() > MAX_PAGES {
            return Err(PdfMasterError::Budget("page selection length"));
        }
        let mut ordered = BTreeSet::new();
        for &page in selected {
            validate_page_index(page, self.pages.len())?;
            if !ordered.insert(page) {
                return Err(PdfMasterError::InvalidOperation("duplicate page selection"));
            }
        }
        let all_ids = page_object_ids(&self.document)?;
        let selected_indices: Vec<_> = ordered.into_iter().map(Some).collect();
        let labels =
            crate::page_labels::PageLabelPlan::read(&self.document)?.select(&selected_indices)?;
        let selected_ids: Vec<_> = selected_indices
            .iter()
            .map(|page| all_ids[page.unwrap()])
            .collect();
        let mut source = self.document.clone();
        materialize_page_attributes(&mut source, &selected_ids)?;
        let mut copier = GraphCopy {
            source: &source,
            output: Document::with_version(source.version.clone()),
            page_ids: all_ids.into_iter().collect(),
            selected_ids: selected_ids.iter().copied().collect(),
            map: HashMap::new(),
            queue: VecDeque::new(),
            items: 0,
            name_index: None,
            lookup_work: std::cell::Cell::new(0),
        };
        let pages_id = copier.output.new_object_id();
        let mut kids = Vec::new();
        for id in selected_ids {
            kids.push(copier.reference(id)?);
        }
        let mut catalog = dictionary! {"Type"=>"Catalog", "Pages"=>pages_id};
        // These global entries affect selected-page appearance. Do not carry
        // global Info/Names/attachments/outlines or unrelated document metadata.
        let original_catalog = source.catalog().map_err(malformed)?;
        for key in [b"OutputIntents".as_slice(), b"OCProperties", b"Lang"] {
            if let Ok(value) = original_catalog.get(key) {
                catalog.set(key, copier.object(value, 0, false)?);
            }
        }
        let catalog_id = copier.output.add_object(catalog);
        copier.output.trailer.set("Root", catalog_id);
        copier.output.objects.insert(
            pages_id,
            dictionary! {
                "Type"=>"Pages", "Count"=>kids.len() as i64, "Kids"=>kids,
            }
            .into(),
        );
        while let Some(old_id) = copier.queue.pop_front() {
            let mut object = source
                .objects
                .get(&old_id)
                .ok_or_else(|| PdfMasterError::Malformed("extraction reference is missing".into()))?
                .clone();
            let is_page = copier.selected_ids.contains(&old_id);
            if is_page {
                object.as_dict_mut().map_err(malformed)?.remove(b"Parent");
            }
            let mut copied = copier.object(&object, 0, is_page)?;
            if is_page {
                copied
                    .as_dict_mut()
                    .map_err(malformed)?
                    .set("Parent", pages_id);
            }
            copier.output.objects.insert(copier.map[&old_id], copied);
        }
        labels.write(&mut copier.output)?;
        let bytes = serialize(&copier.output)?;
        // Reopen through the same strict admission/budget boundary as normal
        // documents. This never assigns into the source Session or its history.
        Session::open(&bytes)?;
        Ok(bytes)
    }
}

fn malformed(error: lopdf::Error) -> PdfMasterError {
    PdfMasterError::Malformed(error.to_string())
}

struct GraphCopy<'a> {
    source: &'a Document,
    output: Document,
    page_ids: HashSet<ObjectId>,
    selected_ids: HashSet<ObjectId>,
    map: HashMap<ObjectId, ObjectId>,
    queue: VecDeque<ObjectId>,
    items: usize,
    name_index: Option<HashMap<Vec<u8>, Object>>,
    lookup_work: std::cell::Cell<usize>,
}

impl GraphCopy<'_> {
    fn reference(&mut self, old_id: ObjectId) -> Result<Object> {
        if self.page_ids.contains(&old_id) && !self.selected_ids.contains(&old_id) {
            return Err(PdfMasterError::InvalidOperation(
                "selected-page relationship targets an excluded page",
            ));
        }
        if let Some(id) = self.map.get(&old_id) {
            return Ok(Object::Reference(*id));
        }
        if !self.source.objects.contains_key(&old_id) {
            return Err(PdfMasterError::Malformed(
                "extraction reference is missing".into(),
            ));
        }
        if self.map.len() == MAX_COPIED_OBJECTS {
            return Err(PdfMasterError::Budget("extraction object graph"));
        }
        let id = self.output.new_object_id();
        self.map.insert(old_id, id);
        self.queue.push_back(old_id);
        Ok(Object::Reference(id))
    }

    fn object(&mut self, object: &Object, depth: usize, selected_page: bool) -> Result<Object> {
        if depth > MAX_CONTAINER_DEPTH {
            return Err(PdfMasterError::Budget("extraction container depth"));
        }
        self.items = self
            .items
            .checked_add(1)
            .ok_or(PdfMasterError::Budget("extraction graph items"))?;
        if self.items > MAX_GRAPH_ITEMS {
            return Err(PdfMasterError::Budget("extraction graph items"));
        }
        Ok(match object {
            Object::Reference(id) => self.reference(*id)?,
            Object::Array(values) => Object::Array(
                values
                    .iter()
                    .map(|value| self.object(value, depth + 1, false))
                    .collect::<Result<_>>()?,
            ),
            Object::Dictionary(dictionary) => {
                Object::Dictionary(self.dictionary(dictionary, depth, selected_page)?)
            }
            Object::Stream(stream) => {
                let mut copied = stream.clone();
                copied.dict = self.dictionary(&stream.dict, depth, false)?;
                Object::Stream(copied)
            }
            value => value.clone(),
        })
    }

    fn dictionary(
        &mut self,
        source: &Dictionary,
        depth: usize,
        selected_page: bool,
    ) -> Result<Dictionary> {
        let kind = self.optional_name(source, b"Type")?;
        if matches!(kind, Some(b"Pages" | b"Catalog")) || kind == Some(b"Page") && !selected_page {
            return Err(PdfMasterError::InvalidOperation(
                "extraction cannot copy an external page tree",
            ));
        }
        if source.has(b"StructParents") || source.has(b"StructParent") {
            return Err(PdfMasterError::InvalidOperation(
                "tagged extraction is not yet supported",
            ));
        }
        if self.optional_name(source, b"Subtype")? == Some(b"Widget") {
            return Err(PdfMasterError::InvalidOperation(
                "form widget extraction is not yet supported",
            ));
        }
        let mut result = Dictionary::new();
        for (key, value) in source.iter() {
            let is_destination =
                key == b"Dest" || key == b"D" && self.optional_name(source, b"S")? == Some(b"GoTo");
            if is_destination {
                let destination = self.destination(value)?;
                result.set(key.clone(), self.object(&destination, depth + 1, false)?);
            } else {
                result.set(key.clone(), self.object(value, depth + 1, false)?);
            }
        }
        Ok(result)
    }

    fn optional_name<'a>(
        &'a self,
        dictionary: &'a Dictionary,
        key: &[u8],
    ) -> Result<Option<&'a [u8]>> {
        dictionary
            .get(key)
            .ok()
            .map(|value| self.resolved(value)?.as_name().map_err(malformed))
            .transpose()
    }

    fn resolved<'a>(&'a self, value: &'a Object) -> Result<&'a Object> {
        let mut value = value;
        let mut visited = HashSet::new();
        self.lookup_step(1)?;
        while let Object::Reference(id) = value {
            self.lookup_step(1)?;
            if visited.len() >= MAX_CONTAINER_DEPTH || !visited.insert(*id) {
                return Err(PdfMasterError::Malformed(
                    "cyclic or deep extraction relationship".into(),
                ));
            }
            value = self.source.objects.get(id).ok_or_else(|| {
                PdfMasterError::Malformed("extraction reference is missing".into())
            })?;
        }
        Ok(value)
    }

    // Inline only destinations actually used by selected annotations/actions.
    // Copying the complete global name tree could retain excluded pages.
    fn destination(&mut self, value: &Object) -> Result<Object> {
        let value = self.resolved(value)?;
        let key = match value {
            Object::Array(_) => return self.destination_array(value),
            Object::Name(key) | Object::String(key, _) => key.clone(),
            _ => {
                return Err(PdfMasterError::Malformed(
                    "invalid local destination".into(),
                ));
            }
        };
        if self.name_index.is_none() {
            self.name_index = Some(self.build_name_index()?);
        }
        let found = self
            .name_index
            .as_ref()
            .and_then(|index| index.get(&key))
            .ok_or_else(|| PdfMasterError::Malformed("named destination is missing".into()))?;
        self.destination_array(found)
    }

    fn lookup_step(&self, count: usize) -> Result<()> {
        let work = self.lookup_work.get().saturating_add(count);
        if work > MAX_GRAPH_ITEMS {
            return Err(PdfMasterError::Budget("extraction relationship work"));
        }
        self.lookup_work.set(work);
        Ok(())
    }

    fn build_name_index(&self) -> Result<HashMap<Vec<u8>, Object>> {
        let mut index = HashMap::new();
        let catalog = self.source.catalog().map_err(malformed)?;
        if let Ok(legacy) = catalog.get(b"Dests") {
            let legacy = self.resolved(legacy)?.as_dict().map_err(malformed)?;
            self.lookup_step(legacy.len())?;
            for (key, value) in legacy.iter() {
                index.insert(key.clone(), value.clone());
            }
        }
        let Ok(names) = catalog.get(b"Names") else {
            return Ok(index);
        };
        let names = self.resolved(names)?.as_dict().map_err(malformed)?;
        let Ok(root) = names.get(b"Dests") else {
            return Ok(index);
        };
        let mut queue = VecDeque::from([root]);
        let mut nodes = 0usize;
        let mut entries = 0usize;
        let mut seen = HashSet::new();
        let mut names_seen = HashSet::new();
        while let Some(node) = queue.pop_front() {
            nodes += 1;
            if nodes > MAX_COPIED_OBJECTS {
                return Err(PdfMasterError::Budget("destination name tree"));
            }
            if let Object::Reference(id) = node
                && !seen.insert(*id)
            {
                return Err(PdfMasterError::Malformed(
                    "cyclic destination name tree".into(),
                ));
            }
            let node = self.resolved(node)?.as_dict().map_err(malformed)?;
            if let Ok(values) = node.get(b"Names") {
                let values = self.resolved(values)?.as_array().map_err(malformed)?;
                entries = entries.saturating_add(values.len());
                self.lookup_step(values.len())?;
                if entries > MAX_GRAPH_ITEMS {
                    return Err(PdfMasterError::Budget("destination names"));
                }
                if values.len() % 2 != 0 {
                    return Err(PdfMasterError::Malformed(
                        "unpaired destination name".into(),
                    ));
                }
                for pair in values.chunks_exact(2) {
                    let key = self
                        .resolved(&pair[0])?
                        .as_str()
                        .map_err(malformed)?
                        .to_vec();
                    if !names_seen.insert(key.clone()) {
                        return Err(PdfMasterError::Malformed(
                            "duplicate destination name".into(),
                        ));
                    }
                    // Preserve legacy lookup precedence without copying the
                    // complete destination graph into the output document.
                    index.entry(key).or_insert_with(|| pair[1].clone());
                }
            }
            if let Ok(kids) = node.get(b"Kids") {
                let kids = self.resolved(kids)?.as_array().map_err(malformed)?;
                if queue.len().saturating_add(kids.len()).saturating_add(nodes) > MAX_COPIED_OBJECTS
                {
                    return Err(PdfMasterError::Budget("destination name tree"));
                }
                queue.extend(kids);
            }
        }
        Ok(index)
    }

    fn destination_array(&self, value: &Object) -> Result<Object> {
        let mut value = self.resolved(value)?;
        if let Object::Dictionary(dictionary) = value {
            value = self.resolved(dictionary.get(b"D").map_err(malformed)?)?;
        }
        let values = value.as_array().map_err(malformed)?;
        if values.len() < 2
            || !values[0]
                .as_reference()
                .ok()
                .is_some_and(|id| self.page_ids.contains(&id))
            || values[1].as_name().is_err()
        {
            return Err(PdfMasterError::Malformed(
                "invalid local destination array".into(),
            ));
        }
        Ok(Object::Array(values.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_destinations_are_indexed_once_and_lookup_work_is_globally_bounded() {
        let session = Session::blank(100., 100., 1).unwrap();
        let mut source = Document::load_mem(&session.bytes().unwrap()).unwrap();
        let page = source.get_pages()[&1];
        let mut names = Vec::new();
        for index in 0..1000 {
            names.push(Object::string_literal(format!("target-{index}")));
            names.push(Object::Array(vec![
                Object::Reference(page),
                Object::Name(b"Fit".to_vec()),
            ]));
        }
        source
            .catalog_mut()
            .unwrap()
            .set("Names", dictionary! {"Dests"=>dictionary! {"Names"=>names}});
        let mut copier = GraphCopy {
            source: &source,
            output: Document::with_version("1.7"),
            page_ids: HashSet::from([page]),
            selected_ids: HashSet::from([page]),
            map: HashMap::new(),
            queue: VecDeque::new(),
            items: 0,
            name_index: None,
            lookup_work: std::cell::Cell::new(0),
        };
        let value = Object::string_literal("target-999");
        let expected = copier.destination(&value).unwrap();
        let indexed = copier.lookup_work.get();
        assert!(indexed >= 1000);
        for _ in 0..100 {
            assert_eq!(copier.destination(&value).unwrap(), expected);
        }
        assert!(
            copier.lookup_work.get() - indexed < 500,
            "must not rescan the tree per link"
        );
        copier.lookup_work.set(MAX_GRAPH_ITEMS);
        assert!(
            copier.destination(&value).is_err(),
            "aggregate relationship work cannot reset per link"
        );
    }
}
