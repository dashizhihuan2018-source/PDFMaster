use crate::document::{
    MAX_PAGES, PdfMasterError, Result, Session, materialize_page_attributes, page_object_ids,
};
use crate::edit::flatten_page_tree;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{BTreeSet, HashMap, VecDeque};

pub const MAX_INSERT_FILES: usize = 32;
pub const MAX_INSERT_BYTES: usize = 64 * 1024 * 1024;

impl Session {
    /// Inserts every admitted incoming page as one undoable transaction. The
    /// caller's source bytes and this session remain unchanged on any failure.
    pub fn insert_pdf(&mut self, bytes: &[u8], index: usize) -> Result<()> {
        self.insert_pdfs(&[bytes], index)
    }

    /// Inserts files in the supplied order. All preparation is disposable and
    /// only the final serialized/reparsed document enters the session history.
    pub fn insert_pdfs(&mut self, files: &[&[u8]], index: usize) -> Result<()> {
        if index > self.pages.len() {
            return Err(PdfMasterError::InvalidPage {
                index,
                page_count: self.pages.len(),
            });
        }
        if files.is_empty() {
            return Err(PdfMasterError::InvalidOperation("no PDF files selected"));
        }
        if files.len() > MAX_INSERT_FILES {
            return Err(PdfMasterError::Budget("insert file count"));
        }
        let total = files.iter().try_fold(0usize, |total, bytes| {
            total
                .checked_add(bytes.len())
                .ok_or(PdfMasterError::Budget("insert input bytes"))
        })?;
        if total > MAX_INSERT_BYTES {
            return Err(PdfMasterError::Budget("insert input bytes"));
        }
        let mut candidate = self.document.clone();
        let mut count = self.pages.len();
        let mut cursor = index;
        for bytes in files {
            let (next, added) = insert_document(&candidate, count, bytes, cursor)?;
            candidate = next;
            count += added;
            cursor += added;
        }
        self.commit_document(candidate)
    }
}

fn insert_document(
    target: &lopdf::Document,
    count: usize,
    bytes: &[u8],
    index: usize,
) -> Result<(lopdf::Document, usize)> {
    let incoming = Session::open(bytes)?;
    let incoming_count = incoming.info().page_count;
    if count.saturating_add(incoming_count) > MAX_PAGES {
        return Err(PdfMasterError::Budget("inserted page count"));
    }
    let catalog = incoming.document.catalog().map_err(malformed)?;
    if [b"AcroForm".as_slice(), b"StructTreeRoot", b"MarkInfo"]
        .iter()
        .any(|key| catalog.has(key))
    {
        return Err(PdfMasterError::InvalidOperation(
            "global form/tagged insertion needs relationship migration",
        ));
    }
    // These are document-wide appearance policies, not per-page resources.
    // Do not silently change existing pages by adopting the incoming policy.
    if catalog.has(b"OCProperties") || catalog.has(b"OutputIntents") {
        return Err(PdfMasterError::InvalidOperation(
            "document-wide color/layer insertion policy is not yet supported",
        ));
    }
    // Catalog language cannot be preserved by inventing Page /Lang entries.
    // A mixed-language document needs tagged/marked-content migration.
    if catalog_language(&incoming.document)? != catalog_language(target)? {
        return Err(PdfMasterError::InvalidOperation(
            "mixed document language insertion needs tagged language migration",
        ));
    }
    let selected: Vec<_> = (0..incoming.pages.len()).collect();
    let mut imported = Session::open(&incoming.extract_pages(&selected)?)?.document;
    let page_ids = page_object_ids(&imported)?;
    materialize_page_attributes(&mut imported, &page_ids)?;
    let retired_catalog = imported
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(malformed)?;
    let retired_pages = imported
        .catalog()
        .and_then(|c| c.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(malformed)?;
    for &id in &page_ids {
        imported
            .get_object_mut(id)
            .and_then(Object::as_dict_mut)
            .map_err(malformed)?
            .remove(b"Parent");
    }
    // Serialized/reloaded PDFs include XRef/ObjStm container objects. Copy
    // only page-reachable objects, not stale file-format containers/roots.
    let reachable = page_reachable(&imported, &page_ids)?;
    imported.objects.retain(|id, _| reachable.contains(id));
    validate_retired_roots(&imported, &[retired_catalog, retired_pages])?;
    let mut candidate = target.clone();
    let start = candidate
        .max_id
        .checked_add(1)
        .ok_or(PdfMasterError::Budget("insert object IDs"))?;
    let length = u32::try_from(imported.objects.len())
        .map_err(|_| PdfMasterError::Budget("insert object count"))?;
    let end = start
        .checked_add(length)
        .ok_or(PdfMasterError::Budget("insert object IDs"))?;
    let map: HashMap<_, _> = imported
        .objects
        .keys()
        .copied()
        .zip((start..end).map(|id| (id, 0)))
        .collect();
    let mut remap = Remap {
        map: &map,
        items: 0,
    };
    let mapped = |id: &ObjectId| {
        map.get(id)
            .copied()
            .ok_or_else(|| PdfMasterError::Malformed("insert object ID mapping is missing".into()))
    };
    let imported_ids: Vec<_> = page_ids.iter().map(mapped).collect::<Result<_>>()?;
    for (old_id, object) in &imported.objects {
        candidate
            .objects
            .insert(mapped(old_id)?, remap.object(object, 0)?);
    }
    candidate.max_id = candidate.max_id.max(end - 1);
    let mut ids = page_object_ids(target)?;
    ids.splice(index..index, imported_ids);
    flatten_page_tree(&mut candidate, &ids)?;
    candidate.version = candidate.version.max(imported.version);
    Ok((candidate, incoming_count))
}

fn page_reachable(document: &lopdf::Document, pages: &[ObjectId]) -> Result<BTreeSet<ObjectId>> {
    fn references(
        object: &Object,
        depth: usize,
        items: &mut usize,
        queue: &mut VecDeque<ObjectId>,
    ) -> Result<()> {
        *items = items.saturating_add(1);
        if depth > 64 || *items > 1_000_000 {
            return Err(PdfMasterError::Budget("insert reachable graph"));
        }
        match object {
            Object::Reference(id) => queue.push_back(*id),
            Object::Array(values) => {
                for value in values {
                    references(value, depth + 1, items, queue)?;
                }
            }
            Object::Dictionary(dict) => {
                for (_, value) in dict {
                    references(value, depth + 1, items, queue)?;
                }
            }
            Object::Stream(stream) => {
                for (_, value) in &stream.dict {
                    references(value, depth + 1, items, queue)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut queue: VecDeque<_> = pages.iter().copied().collect();
    let mut seen = BTreeSet::new();
    let mut items = 0;
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if seen.len() > 65_536 {
            return Err(PdfMasterError::Budget("insert reachable objects"));
        }
        let object = document
            .objects
            .get(&id)
            .ok_or_else(|| PdfMasterError::Malformed("insert reference is missing".into()))?;
        references(object, 0, &mut items, &mut queue)?;
    }
    Ok(seen)
}

fn catalog_language(document: &lopdf::Document) -> Result<Option<String>> {
    let Some(mut value) = document.catalog().map_err(malformed)?.get(b"Lang").ok() else {
        return Ok(None);
    };
    let mut seen = std::collections::HashSet::new();
    while let Object::Reference(id) = value {
        if seen.len() >= 64 || !seen.insert(*id) {
            return Err(PdfMasterError::Malformed(
                "cyclic or deep language reference".into(),
            ));
        }
        value = document
            .objects
            .get(id)
            .ok_or_else(|| PdfMasterError::Malformed("language reference is missing".into()))?;
    }
    if matches!(value, Object::Null) {
        return Ok(None);
    }
    if value.as_str().map_err(malformed)?.len() > 1024 {
        return Err(PdfMasterError::Budget("document language"));
    }
    lopdf::decode_text_string(value)
        .map(Some)
        .map_err(malformed)
}

fn validate_retired_roots(document: &lopdf::Document, retired: &[ObjectId]) -> Result<()> {
    if retired.iter().any(|id| document.objects.contains_key(id)) {
        return Err(PdfMasterError::InvalidOperation(
            "retired document root survived page graph pruning",
        ));
    }
    fn visit(object: &Object, retired: &[ObjectId], depth: usize, items: &mut usize) -> Result<()> {
        *items = items.saturating_add(1);
        if depth > 64 || *items > 1_000_000 {
            return Err(PdfMasterError::Budget("insert retained reference scan"));
        }
        match object {
            Object::Reference(id) if retired.contains(id) => {
                return Err(PdfMasterError::InvalidOperation(
                    "insert retained a retired document root",
                ));
            }
            Object::Array(values) => {
                for value in values {
                    visit(value, retired, depth + 1, items)?;
                }
            }
            Object::Dictionary(dict) => {
                for (_, value) in dict {
                    visit(value, retired, depth + 1, items)?;
                }
            }
            Object::Stream(stream) => {
                for (_, value) in &stream.dict {
                    visit(value, retired, depth + 1, items)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut items = 0;
    for (id, object) in &document.objects {
        visit(object, retired, 0, &mut items).map_err(|error| {
            PdfMasterError::Malformed(format!(
                "retired-root check at object {id:?}, roots {retired:?}: {error}"
            ))
        })?;
    }
    Ok(())
}

fn malformed(error: lopdf::Error) -> PdfMasterError {
    PdfMasterError::Malformed(error.to_string())
}

struct Remap<'a> {
    map: &'a HashMap<ObjectId, ObjectId>,
    items: usize,
}
impl Remap<'_> {
    fn object(&mut self, object: &Object, depth: usize) -> Result<Object> {
        self.items = self.items.saturating_add(1);
        if depth > 64 || self.items > 1_000_000 {
            return Err(PdfMasterError::Budget("insert object graph"));
        }
        Ok(match object {
            Object::Reference(id) => {
                Object::Reference(*self.map.get(id).ok_or_else(|| {
                    PdfMasterError::Malformed("insert reference is missing".into())
                })?)
            }
            Object::Array(values) => Object::Array(
                values
                    .iter()
                    .map(|v| self.object(v, depth + 1))
                    .collect::<Result<_>>()?,
            ),
            Object::Dictionary(dict) => Object::Dictionary(self.dictionary(dict, depth)?),
            Object::Stream(stream) => {
                let mut copied = stream.clone();
                copied.dict = self.dictionary(&stream.dict, depth)?;
                Object::Stream(copied)
            }
            value => value.clone(),
        })
    }
    fn dictionary(&mut self, source: &Dictionary, depth: usize) -> Result<Dictionary> {
        let mut result = Dictionary::new();
        for (key, value) in source.iter() {
            result.set(key.clone(), self.object(value, depth + 1)?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_roots_cannot_leave_references_in_retained_objects() {
        let mut doc = lopdf::Document::with_version("1.7");
        let retired = doc.add_object(Dictionary::new());
        doc.add_object(lopdf::dictionary! {"Nested"=>vec![Object::Reference(retired)]});
        assert!(validate_retired_roots(&doc, &[retired]).is_err());
        let empty = lopdf::Document::with_version("1.7");
        assert!(validate_retired_roots(&empty, &[retired]).is_ok());
    }
    #[test]
    fn retired_root_cannot_itself_survive_as_a_reachable_seed() {
        let mut doc = lopdf::Document::with_version("1.7");
        let retired = doc.add_object(Dictionary::new());
        assert!(validate_retired_roots(&doc, &[retired]).is_err());
    }
}
