//! Bounded, cross-document import of the owned text producer's AP resource graph.
use crate::text_appearance::PreparedTextAppearance;
use crate::{FreeTextAppearanceStyle, PdfMasterError, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_OBJECTS: usize = 65_536;
const MAX_NODES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;
fn malformed() -> PdfMasterError {
    PdfMasterError::Malformed("invalid FreeText appearance carrier".into())
}
fn collect(source: &Document, roots: &[Object]) -> Result<BTreeSet<ObjectId>> {
    struct Walk<'a> {
        source: &'a Document,
        seen: BTreeSet<ObjectId>,
        path: BTreeSet<ObjectId>,
        nodes: usize,
        bytes: usize,
        decoded_bytes: usize,
    }
    impl Walk<'_> {
        fn visit(&mut self, value: &Object, depth: usize) -> Result<()> {
            self.nodes += 1;
            if depth > MAX_DEPTH || self.nodes > MAX_NODES {
                return Err(PdfMasterError::Budget("appearance graph"));
            }
            match value {
                Object::Reference(id) => {
                    if self.path.contains(id) {
                        return Err(malformed());
                    }
                    if !self.seen.contains(id) {
                        if self.seen.len() == MAX_OBJECTS {
                            return Err(PdfMasterError::Budget("appearance objects"));
                        }
                        self.path.insert(*id);
                        self.visit(
                            self.source.objects.get(id).ok_or_else(malformed)?,
                            depth + 1,
                        )?;
                        self.path.remove(id);
                        self.seen.insert(*id);
                    }
                }
                Object::Array(values) => {
                    for value in values {
                        self.visit(value, depth + 1)?;
                    }
                }
                Object::Dictionary(dict) => {
                    for (_, value) in dict.iter() {
                        self.visit(value, depth + 1)?;
                    }
                }
                Object::Stream(stream) => {
                    self.bytes = self
                        .bytes
                        .checked_add(stream.content.len())
                        .ok_or(PdfMasterError::Budget("appearance streams"))?;
                    if self.bytes > MAX_BYTES {
                        return Err(PdfMasterError::Budget("appearance streams"));
                    }
                    let remaining = MAX_BYTES - self.decoded_bytes;
                    let decoded = stream
                        .decompressed_content_with_limit(remaining)
                        .map_err(|_| PdfMasterError::Budget("appearance decoded streams"))?;
                    self.decoded_bytes += decoded.len();
                    for (_, value) in stream.dict.iter() {
                        self.visit(value, depth + 1)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }
    let mut walk = Walk {
        source,
        seen: BTreeSet::new(),
        path: BTreeSet::new(),
        nodes: 0,
        bytes: 0,
        decoded_bytes: 0,
    };
    for value in roots {
        walk.visit(value, 0)?;
    }
    Ok(walk.seen)
}
fn id_map(start: u32, objects: &BTreeSet<ObjectId>) -> Result<BTreeMap<ObjectId, ObjectId>> {
    // Reserve room for the form, annotation, popup and Annots allocations as well.
    let count =
        u32::try_from(objects.len()).map_err(|_| PdfMasterError::Budget("appearance IDs"))?;
    start
        .checked_add(count)
        .and_then(|n| n.checked_add(4))
        .ok_or(PdfMasterError::Budget("appearance IDs"))?;
    Ok(objects
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, (start + index as u32 + 1, 0)))
        .collect())
}
fn rewrite(value: &Object, ids: &BTreeMap<ObjectId, ObjectId>) -> Result<Object> {
    Ok(match value {
        Object::Reference(id) => Object::Reference(*ids.get(id).ok_or_else(malformed)?),
        Object::Array(values) => Object::Array(
            values
                .iter()
                .map(|v| rewrite(v, ids))
                .collect::<Result<_>>()?,
        ),
        Object::Dictionary(dict) => {
            let mut result = Dictionary::new();
            for (key, value) in dict.iter() {
                result.set(key.clone(), rewrite(value, ids)?);
            }
            result.into()
        }
        Object::Stream(stream) => {
            let dict = rewrite(&Object::Dictionary(stream.dict.clone()), ids)?
                .as_dict()
                .map_err(|_| malformed())?
                .clone();
            Stream::new(dict, stream.content.clone()).into()
        }
        _ => value.clone(),
    })
}
fn carrier_roots(bytes: &[u8], width: f32, height: f32) -> Result<(Document, Stream, Dictionary)> {
    if bytes.len() > MAX_BYTES {
        return Err(PdfMasterError::Budget("appearance carrier"));
    }
    let source = Document::load_mem_with_options(
        bytes,
        lopdf::LoadOptions {
            strict: true,
            max_decompressed_size: Some(MAX_BYTES),
            ..Default::default()
        },
    )
    .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
    let pages = source.get_pages();
    if pages.len() != 1 {
        return Err(malformed());
    }
    let page = source
        .get_object(*pages.values().next().ok_or_else(malformed)?)
        .and_then(Object::as_dict)
        .map_err(|_| malformed())?;
    let get = |key: &[u8]| -> Result<&Object> {
        crate::notes::resolve(&source, page.get(key).map_err(|_| malformed())?)
    };
    let media = get(b"MediaBox")?.as_array().map_err(|_| malformed())?;
    if media.len() != 4 {
        return Err(malformed());
    }
    for (value, expected) in media.iter().zip([0., 0., width, height]) {
        if crate::notes::resolve(&source, value)?
            .as_float()
            .map_err(|_| malformed())?
            != expected
        {
            return Err(malformed());
        }
    }
    let stream = get(b"Contents")?
        .as_stream()
        .map_err(|_| malformed())?
        .clone();
    if stream.dict.has(b"Filter") || stream.content.len() > MAX_BYTES {
        return Err(malformed());
    }
    let resources = get(b"Resources")?
        .as_dict()
        .map_err(|_| malformed())?
        .clone();
    Ok((source, stream, resources))
}
fn first_font(source: &Document, resources: &Dictionary, contents: &[u8]) -> Result<Vec<u8>> {
    let content = lopdf::content::Content::decode(contents).map_err(|_| malformed())?;
    let fonts = crate::notes::resolve(source, resources.get(b"Font").map_err(|_| malformed())?)?
        .as_dict()
        .map_err(|_| malformed())?;
    let mut first = None;
    for op in content.operations.iter().filter(|op| op.operator == "Tf") {
        if op.operands.len() != 2 {
            return Err(malformed());
        }
        let name = op.operands[0].as_name().map_err(|_| malformed())?;
        let size = op.operands[1].as_float().map_err(|_| malformed())?;
        if !size.is_finite() || size <= 0. {
            return Err(malformed());
        }
        let font = crate::notes::resolve(source, fonts.get(name).map_err(|_| malformed())?)?
            .as_dict()
            .map_err(|_| malformed())?;
        if crate::notes::resolve(source, font.get(b"Type").map_err(|_| malformed())?)?
            .as_name()
            .map_err(|_| malformed())?
            != b"Font"
        {
            return Err(malformed());
        }
        if first.is_none() {
            first = Some(name.to_vec());
        }
    }
    first.ok_or_else(malformed)
}
pub(crate) fn import(
    candidate: &mut Document,
    prepared: &PreparedTextAppearance,
    width: f32,
    height: f32,
    style: &FreeTextAppearanceStyle,
) -> Result<(ObjectId, Object)> {
    let (source, contents, resources) = carrier_roots(prepared.pdf_bytes(), width, height)?;
    let font = first_font(&source, &resources, &contents.content)?;
    let roots = [
        Object::Stream(contents.clone()),
        Object::Dictionary(resources.clone()),
    ];
    let objects = collect(&source, &roots)?;
    let ids = id_map(candidate.max_id, &objects)?;
    let imported: Vec<_> = objects
        .iter()
        .map(|id| {
            Ok((
                ids[id],
                rewrite(source.objects.get(id).ok_or_else(malformed)?, &ids)?,
            ))
        })
        .collect::<Result<_>>()?;
    let mut form = rewrite(&Object::Stream(contents), &ids)?
        .as_stream()
        .map_err(|_| malformed())?
        .clone();
    form.dict.set("Type", "XObject");
    form.dict.set("Subtype", "Form");
    form.dict.set("FormType", 1);
    form.dict.set(
        "BBox",
        vec![0.into(), 0.into(), width.into(), height.into()],
    );
    form.dict.set(
        "Matrix",
        vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
    );
    form.dict
        .set("Resources", rewrite(&Object::Dictionary(resources), &ids)?);
    let da = lopdf::content::Content {
        operations: vec![
            lopdf::content::Operation::new("Tf", vec![Object::Name(font), style.font_size.into()]),
            lopdf::content::Operation::new(
                "rg",
                vec![
                    style.color.r.into(),
                    style.color.g.into(),
                    style.color.b.into(),
                ],
            ),
        ],
    }
    .encode()
    .map_err(|e| PdfMasterError::Serialization(e.to_string()))?;
    // No candidate mutation until all collection, remapping and serialization succeeds.
    for (id, object) in imported {
        candidate.max_id = candidate.max_id.max(id.0);
        candidate.objects.insert(id, object);
    }
    let form_id = candidate
        .try_add_object(form)
        .map_err(|_| PdfMasterError::Budget("appearance IDs"))?;
    Ok((form_id, Object::string_literal(da)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Stream, dictionary};
    fn carrier(
        pages: usize,
        contents: usize,
        filter: bool,
        media: Option<Vec<Object>>,
        resources: Object,
    ) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for _ in 0..pages {
            let mut streams = Vec::new();
            for _ in 0..contents {
                let mut stream = Stream::new(dictionary! {}, b"q Q\n".repeat(100));
                if filter {
                    stream.compress().unwrap();
                }
                streams.push(Object::Reference(doc.add_object(stream)));
            }
            let content = if contents == 1 {
                streams.pop().unwrap()
            } else {
                Object::Array(streams)
            };
            let mut page = dictionary! {"Type"=>"Page","Parent"=>pages_id,"Contents"=>content,"Resources"=>resources.clone()};
            if let Some(media) = &media {
                page.set("MediaBox", media.clone());
            }
            kids.push(Object::Reference(doc.add_object(page)));
        }
        doc.objects.insert(
            pages_id,
            dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>pages as i64}.into(),
        );
        let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
        doc.trailer.set("Root", root);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn free_text_carrier_rejects_size_page_count_box_content_filter_and_resource_shapes() {
        let media = Some(vec![0.into(), 0.into(), 100.into(), 100.into()]);
        let invalid = vec![
            carrier(2, 1, false, media.clone(), dictionary! {}.into()),
            carrier(1, 2, false, media.clone(), dictionary! {}.into()),
            carrier(1, 1, true, media.clone(), dictionary! {}.into()),
            carrier(1, 1, false, None, dictionary! {}.into()),
            carrier(
                1,
                1,
                false,
                Some(vec![1.into(), 0.into(), 101.into(), 100.into()]),
                dictionary! {}.into(),
            ),
            carrier(
                1,
                1,
                false,
                Some(vec![0.into(), 0.into(), 99.into(), 100.into()]),
                dictionary! {}.into(),
            ),
            carrier(1, 1, false, media.clone(), Object::Integer(1)),
            vec![0; MAX_BYTES + 1],
            b"not a PDF".to_vec(),
        ];
        let rejected: Vec<_> = invalid
            .iter()
            .map(|bytes| carrier_roots(bytes, 100., 100.).is_err())
            .collect();
        assert_eq!(rejected, vec![true; 9]);
        assert!(
            carrier_roots(
                &carrier(1, 1, false, media, dictionary! {}.into()),
                100.,
                100.
            )
            .is_ok()
        );
    }
    #[test]
    fn free_text_carrier_rejects_over_budget_decompression() {
        let bytes = carrier(
            1,
            1,
            false,
            Some(vec![0.into(), 0.into(), 100.into(), 100.into()]),
            dictionary! {}.into(),
        );
        let mut doc = Document::load_mem(&bytes).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let id = doc
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .as_reference()
            .unwrap();
        let stream = doc.get_object_mut(id).unwrap().as_stream_mut().unwrap();
        stream.set_content(vec![0; MAX_BYTES + 1]);
        stream.compress().unwrap();
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        assert!(bytes.len() < MAX_BYTES);
        assert!(carrier_roots(&bytes, 100., 100.).is_err());
    }
    #[test]
    fn free_text_all_tf_resources_must_be_bound_not_just_the_first() {
        let source = Document::with_version("1.7");
        let resources = dictionary! {"Font"=>dictionary!{"f0"=>dictionary!{"Type"=>"Font"}}};
        assert!(first_font(&source, &resources, b"/f0 12 Tf /missing 12 Tf").is_err());
    }
    #[test]
    fn free_text_da_font_must_be_the_first_tf_bound_to_valid_local_font_resources() {
        let mut source = Document::with_version("1.7");
        let font = source.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type0"});
        let resources = dictionary! {"Font"=>dictionary!{"f0"=>font,"f1"=>font}};
        assert_eq!(
            first_font(&source, &resources, b"/f1 12 Tf /f0 12 Tf").unwrap(),
            b"f1"
        );
    }
    #[test]
    fn free_text_da_font_rejects_unbound_malformed_and_wrong_type_tf_resources() {
        let source = Document::with_version("1.7");
        let cases = [
            (dictionary! {}, b"/f0 12 Tf".as_slice()),
            (
                dictionary! {"Font"=>dictionary!{"f0"=>dictionary!{"Type"=>"NotFont"}}},
                b"/f0 12 Tf".as_slice(),
            ),
            (
                dictionary! {"Font"=>dictionary!{"f0"=>dictionary!{"Type"=>"Font"}}},
                b"/missing 12 Tf".as_slice(),
            ),
            (dictionary! {}, b"12 Tf".as_slice()),
            (dictionary! {}, b"q Q".as_slice()),
        ];
        let rejected: Vec<_> = cases
            .iter()
            .map(|(resources, contents)| first_font(&source, resources, contents).is_err())
            .collect();
        assert_eq!(rejected, vec![true; 5]);
    }
    #[test]
    fn free_text_closure_does_not_follow_unreachable_catalog_page_tree_or_metadata() {
        let mut source = Document::with_version("1.7");
        source.objects.insert((1, 0), Object::Integer(7));
        source.objects.insert((99, 0), Object::Reference((99, 0)));
        source.trailer.set("Root", (99, 0));
        let roots = [
            Object::Stream(Stream::new(
                dictionary! {"OwnedResource"=>(1,0)},
                b"same content".to_vec(),
            )),
            dictionary! {"Font"=>(1,0)}.into(),
        ];
        assert_eq!(collect(&source, &roots).unwrap(), BTreeSet::from([(1, 0)]));
    }
    #[test]
    fn free_text_closure_missing_and_current_path_cycles_are_rejected() {
        let mut doc = Document::with_version("1.7");
        doc.objects.insert((1, 0), Object::Reference((2, 0)));
        doc.objects.insert((2, 0), Object::Reference((1, 0)));
        let rejected = vec![
            collect(&doc, &[Object::Reference((99, 0))]).is_err(),
            collect(&doc, &[Object::Reference((1, 0))]).is_err(),
        ];
        assert_eq!(rejected, vec![true; 2]);
    }
    #[test]
    fn free_text_closure_shared_references_are_deduplicated_not_mistaken_for_cycles() {
        let mut doc = Document::with_version("1.7");
        doc.objects.insert((1, 0), Object::Integer(7));
        doc.objects
            .insert((2, 0), dictionary! {"A"=>(1,0),"B"=>(1,0)}.into());
        assert_eq!(
            collect(
                &doc,
                &[Object::Reference((2, 0)), Object::Reference((1, 0))]
            )
            .unwrap(),
            BTreeSet::from([(1, 0), (2, 0)])
        );
    }
    fn nested(depth: usize) -> Object {
        (0..depth).fold(Object::Null, |value, _| Object::Array(vec![value]))
    }
    #[test]
    fn free_text_closure_depth_exact_boundary_is_bounded() {
        let doc = Document::with_version("1.7");
        assert!(collect(&doc, &[nested(MAX_DEPTH)]).is_ok());
        assert!(collect(&doc, &[nested(MAX_DEPTH + 1)]).is_err());
    }
    #[test]
    fn free_text_closure_object_count_exact_boundary_is_bounded() {
        let mut doc = Document::with_version("1.7");
        for index in 1..=MAX_OBJECTS + 1 {
            doc.objects.insert((index as u32, 0), Object::Null);
        }
        let roots: Vec<_> = (1..=MAX_OBJECTS)
            .map(|index| Object::Reference((index as u32, 0)))
            .collect();
        assert!(collect(&doc, &roots).is_ok());
        let mut excessive = roots;
        excessive.push(Object::Reference((MAX_OBJECTS as u32 + 1, 0)));
        assert!(collect(&doc, &excessive).is_err());
    }
    #[test]
    fn free_text_closure_node_count_exact_boundary_is_bounded() {
        let doc = Document::with_version("1.7");
        let roots = vec![Object::Null; MAX_NODES];
        assert!(collect(&doc, &roots).is_ok());
        let mut excessive = roots;
        excessive.push(Object::Null);
        assert!(collect(&doc, &excessive).is_err());
    }
    #[test]
    fn free_text_closure_bounds_decoded_resource_streams_and_combined_decoded_bytes() {
        let doc = Document::with_version("1.7");
        let mut huge = Stream::new(dictionary! {}, vec![0; MAX_BYTES + 1]);
        huge.compress().unwrap();
        let mut half = Stream::new(dictionary! {}, vec![0; MAX_BYTES / 2 + 1]);
        half.compress().unwrap();
        let rejected = vec![
            collect(&doc, &[Object::Stream(huge)]).is_err(),
            collect(&doc, &[Object::Stream(half.clone()), Object::Stream(half)]).is_err(),
        ];
        assert_eq!(rejected, vec![true; 2]);
        let mut source = Document::with_version("1.7");
        let mut exact = Stream::new(dictionary! {}, vec![0; MAX_BYTES]);
        exact.compress().unwrap();
        let id = source.add_object(exact);
        assert_eq!(
            collect(&source, &[Object::Reference(id), Object::Reference(id)]).unwrap(),
            BTreeSet::from([id])
        );
    }
    #[test]
    fn free_text_closure_counts_contents_and_resource_stream_bytes_together() {
        let doc = Document::with_version("1.7");
        let exact = Object::Stream(Stream::new(dictionary! {}, vec![0; MAX_BYTES]));
        assert!(collect(&doc, std::slice::from_ref(&exact)).is_ok());
        let extra = Object::Stream(Stream::new(dictionary! {}, vec![0]));
        assert!(collect(&doc, &[exact, extra]).is_err());
    }
    #[test]
    fn free_text_closure_checked_object_ids_preserve_candidate_allocation_room() {
        let objects = BTreeSet::from([(1, 0), (8, 0)]);
        assert!(id_map(u32::MAX, &objects).is_err());
        assert!(id_map(u32::MAX - 1, &objects).is_err());
        let mapped = id_map(10, &objects).unwrap();
        assert_eq!(
            mapped,
            BTreeMap::from([((1, 0), (11, 0)), ((8, 0), (12, 0))])
        );
    }
    #[test]
    fn free_text_rewrite_rejects_unmapped_nested_reference() {
        let value = Object::Stream(Stream::new(
            dictionary! {"Resources"=>dictionary!{"Font"=>vec![Object::Reference((1,0))]}},
            vec![],
        ));
        assert!(rewrite(&value, &BTreeMap::new()).is_err());
    }
    #[test]
    fn free_text_rewrite_remaps_nested_stream_and_dictionary_references_and_rejects_missing_map() {
        let value = Object::Stream(Stream::new(
            dictionary! {"Resources"=>dictionary!{"Font"=>vec![Object::Reference((1,0)),Object::Reference((2,0))]}},
            b"same operators".to_vec(),
        ));
        let ids = BTreeMap::from([((1, 0), (11, 0)), ((2, 0), (12, 0))]);
        let rewritten = rewrite(&value, &ids).unwrap();
        let stream = rewritten.as_stream().unwrap();
        assert_eq!(stream.content, b"same operators");
        let fonts = stream
            .dict
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Font")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            fonts,
            &vec![Object::Reference((11, 0)), Object::Reference((12, 0))]
        );
        assert!(rewrite(&value, &BTreeMap::new()).is_err());
    }
}
