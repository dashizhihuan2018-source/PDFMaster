use lopdf::{Document, Object, ObjectId, Stream, dictionary, text_string};
use pdfmaster_core::{AnnotationRow, Color, Edit, Session, TextAlignment};

fn fixture(build: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    let mut doc =
        Document::load_mem(&Session::blank(595., 842., 2).unwrap().bytes().unwrap()).unwrap();
    let pages = doc.get_pages().values().copied().collect::<Vec<_>>();
    build(&mut doc, &pages);
    save(doc)
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn annotation(contents: &str) -> Object {
    dictionary! {
        "Subtype" => "FreeText",
        "Rect" => vec![10.into(), 20.into(), 110.into(), 60.into()],
        "Contents" => text_string(contents),
        "DA" => Object::string_literal("/A 10 Tf 0 g"),
    }
    .into()
}

fn set_annots(doc: &mut Document, page: ObjectId, entries: Vec<Object>) {
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", entries);
}

fn generic_delete(expected: &AnnotationRow) -> Edit {
    serde_json::from_value(serde_json::json!({
        "kind": "delete_annotation",
        "expected": expected,
    }))
    .unwrap()
}

fn prepare_both_history(session: &mut Session) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let original = session.bytes().unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let first = session.bytes().unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let second = session.bytes().unwrap();
    session.undo().unwrap();
    assert!(session.info().can_undo && session.info().can_redo);
    (original, first, second)
}

fn assert_history_snapshots_are_reachable(
    session: &mut Session,
    original: &[u8],
    first: &[u8],
    second: &[u8],
) {
    assert_eq!(session.bytes().unwrap(), first);
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), original);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), first);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), second);
}

fn resolved_page_streams(
    document: &Document,
    page_id: ObjectId,
) -> Vec<(lopdf::Dictionary, Vec<u8>)> {
    fn collect(
        document: &Document,
        value: &Object,
        output: &mut Vec<(lopdf::Dictionary, Vec<u8>)>,
    ) {
        match value {
            Object::Reference(id) => collect(document, document.get_object(*id).unwrap(), output),
            Object::Array(values) => {
                for value in values {
                    collect(document, value, output);
                }
            }
            Object::Stream(stream) => output.push((stream.dict.clone(), stream.content.clone())),
            other => panic!("page Contents contained unexpected object: {other:?}"),
        }
    }

    let page = document.get_object(page_id).unwrap().as_dict().unwrap();
    let mut output = Vec::new();
    collect(document, page.get(b"Contents").unwrap(), &mut output);
    output
}

#[test]
fn typed_delete_is_one_transaction_and_reopens_absent() {
    let bytes = include_bytes!("fixtures/unicode-ap/chinese.pdf");
    let mut session = Session::open(bytes).unwrap();
    let expected = session.free_texts(0).unwrap().annotations.remove(0);
    session.delete_free_text(&expected).unwrap();
    let after = session.bytes().unwrap();
    assert!(
        Session::open(&after)
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .is_empty()
    );
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes.as_slice());
    assert!(!session.info().can_undo);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), after);
}

#[test]
fn every_forged_typed_snapshot_field_fails_without_touching_reachable_history() {
    let bytes = fixture(|doc, pages| {
        let id = doc.add_object(annotation("selected"));
        set_annots(doc, pages[0], vec![id.into()]);
    });
    for field in [
        "page_index",
        "annotation_index",
        "page_id",
        "object_id",
        "bounds.x",
        "bounds.y",
        "bounds.width",
        "bounds.height",
        "contents",
        "style.font_resource",
        "style.font_size",
        "style.color.r",
        "style.color.g",
        "style.color.b",
        "style.alignment",
        "flags",
        "revision",
    ] {
        let mut session = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut session);
        let mut expected = session.free_texts(0).unwrap().annotations.remove(0);
        match field {
            "page_index" => expected.page_index = usize::MAX,
            "annotation_index" => expected.annotation_index = 1,
            "page_id" => expected.page_id = (999, 0),
            "object_id" => expected.object_id = Some((999, 0)),
            "bounds.x" => expected.bounds.x += 1.0,
            "bounds.y" => expected.bounds.y += 1.0,
            "bounds.width" => expected.bounds.width += 1.0,
            "bounds.height" => expected.bounds.height += 1.0,
            "contents" => expected.contents.push('!'),
            "style.font_resource" => expected.style.font_resource.push(b'X'),
            "style.font_size" => expected.style.font_size += 1.0,
            "style.color.r" => expected.style.color.r = 0.25,
            "style.color.g" => expected.style.color.g = 0.5,
            "style.color.b" => expected.style.color.b = 0.75,
            "style.alignment" => expected.style.alignment = TextAlignment::Center,
            "flags" => expected.flags ^= 1,
            _ => expected.revision[0] ^= 1,
        }
        let before = session.info();
        assert!(session.delete_free_text(&expected).is_err(), "{field}");
        assert_eq!(session.bytes().unwrap(), first, "{field}");
        assert_eq!(session.info(), before, "{field}");
        assert_history_snapshots_are_reachable(&mut session, &original, &first, &second);
    }
}

#[test]
fn every_forged_generic_snapshot_field_preserves_reachable_history() {
    let bytes = fixture(|doc, pages| {
        let id = doc.add_object(annotation("selected"));
        set_annots(doc, pages[0], vec![id.into()]);
    });
    for field in [
        "page_index",
        "annotation_index",
        "page_id",
        "object_id",
        "subtype",
        "contents",
        "color",
        "bounds",
        "modified",
    ] {
        let mut session = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut session);
        let mut expected = session.annotations().unwrap().remove(0);
        match field {
            "page_index" => expected.page_index = usize::MAX,
            "annotation_index" => expected.annotation_index = 1,
            "page_id" => expected.page_id = (999, 0),
            "object_id" => expected.object_id = Some((999, 0)),
            "subtype" => expected.subtype = "Stamp".into(),
            "contents" => expected.contents.push('!'),
            "color" => {
                expected.color = Some(Color {
                    r: 0.25,
                    g: 0.5,
                    b: 0.75,
                })
            }
            "bounds" => expected.bounds.as_mut().unwrap().x += 1.0,
            _ => expected.modified = Some("2026-11-05T00:00:00Z".into()),
        }
        let before = session.info();
        assert!(session.apply(generic_delete(&expected)).is_err(), "{field}");
        assert_eq!(session.bytes().unwrap(), first, "{field}");
        assert_eq!(session.info(), before, "{field}");
        assert_history_snapshots_are_reachable(&mut session, &original, &first, &second);
    }
}

#[test]
fn revision_rejects_ap_da_metadata_and_other_page_changes_but_identical_bytes_work() {
    let base = fixture(|doc, pages| {
        let root = doc.add_object(annotation("selected"));
        set_annots(doc, pages[0], vec![root.into()]);
    });
    let mut baseline = Session::open(&base).unwrap();
    let (_, baseline_first, _) = prepare_both_history(&mut baseline);
    let expected = baseline.free_texts(0).unwrap().annotations.remove(0);
    assert_eq!(baseline.bytes().unwrap(), baseline_first);

    let mut identical = Session::open(&base).unwrap();
    let _ = prepare_both_history(&mut identical);
    identical.delete_free_text(&expected).unwrap();

    for case in ["ap", "da", "metadata", "other-page"] {
        let mut document = Document::load_mem(&base).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        let entry = document
            .get_object(pages[0])
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        match case {
            "ap" => {
                let ap = document.add_object(Stream::new(
                    dictionary! {"Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),1.into(),1.into()]},
                    b"q Q".to_vec(),
                ));
                document
                    .get_object_mut(entry)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("AP", dictionary! {"N"=>ap});
            }
            "da" => document
                .get_object_mut(entry)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("DA", Object::string_literal("/A 11 Tf 0 g")),
            "metadata" => document
                .get_object_mut(entry)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("VendorPrivate", Object::string_literal("changed")),
            _ => document
                .get_object_mut(pages[1])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("VendorPrivate", Object::string_literal("changed")),
        }
        let changed = save(document);
        let mut session = Session::open(&changed).unwrap();
        let (original, first, second) = prepare_both_history(&mut session);
        let current_revision = session
            .free_texts(0)
            .unwrap()
            .annotations
            .remove(0)
            .revision;
        assert_ne!(expected.revision, current_revision, "{case}");
        let before = session.info();
        assert!(session.delete_free_text(&expected).is_err(), "{case}");
        assert_eq!(session.bytes().unwrap(), first, "{case}");
        assert_eq!(session.info(), before, "{case}");
        assert_history_snapshots_are_reachable(&mut session, &original, &first, &second);
    }
}

#[test]
fn readonly_and_locked_reject_typed_and_generic_while_locked_contents_allows_delete() {
    for flags in [64_u32, 128, 64 | 512, 128 | 512] {
        let bytes = fixture(|doc, pages| {
            let mut value = annotation("locked");
            value.as_dict_mut().unwrap().set("F", i64::from(flags));
            set_annots(doc, pages[0], vec![value]);
        });
        let mut typed = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut typed);
        let expected = typed.free_texts(0).unwrap().annotations.remove(0);
        let before = typed.info();
        assert!(typed.delete_free_text(&expected).is_err());
        assert_eq!(typed.bytes().unwrap(), first);
        assert_eq!(typed.info(), before);
        assert_history_snapshots_are_reachable(&mut typed, &original, &first, &second);

        let mut generic = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut generic);
        let row = generic.annotations().unwrap().remove(0);
        let before = generic.info();
        assert!(generic.apply(generic_delete(&row)).is_err());
        assert_eq!(generic.bytes().unwrap(), first);
        assert_eq!(generic.info(), before);
        assert_history_snapshots_are_reachable(&mut generic, &original, &first, &second);
    }

    let bytes = fixture(|doc, pages| {
        let mut value = annotation("locked-contents");
        value.as_dict_mut().unwrap().set("F", 512);
        set_annots(doc, pages[0], vec![value]);
    });
    let mut typed = Session::open(&bytes).unwrap();
    let expected = typed.free_texts(0).unwrap().annotations.remove(0);
    typed.delete_free_text(&expected).unwrap();
    assert!(typed.free_texts(0).unwrap().annotations.is_empty());
    let mut generic = Session::open(&bytes).unwrap();
    let row = generic.annotations().unwrap().remove(0);
    generic.apply(generic_delete(&row)).unwrap();
    assert!(generic.annotations().unwrap().is_empty());
}

#[test]
fn malformed_flags_reject_projection_and_generic_delete_atomically() {
    for value in [
        Object::Integer(-1),
        Object::String(b"64".to_vec(), lopdf::StringFormat::Literal),
    ] {
        let bytes = fixture(|doc, pages| {
            let mut target = annotation("malformed-flags");
            target.as_dict_mut().unwrap().set("F", value);
            set_annots(doc, pages[0], vec![target]);
        });
        let mut typed = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut typed);
        let before = typed.info();
        assert!(typed.free_texts(0).is_err());
        assert_eq!(typed.bytes().unwrap(), first);
        assert_eq!(typed.info(), before);
        assert_history_snapshots_are_reachable(&mut typed, &original, &first, &second);

        let mut generic = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut generic);
        let row = generic.annotations().unwrap().remove(0);
        let before = generic.info();
        assert!(generic.apply(generic_delete(&row)).is_err());
        assert_eq!(generic.bytes().unwrap(), first);
        assert_eq!(generic.info(), before);
        assert_history_snapshots_are_reachable(&mut generic, &original, &first, &second);
    }
}

#[test]
fn generic_non_freetext_keeps_legacy_flag_behavior() {
    let bytes = fixture(|doc, pages| {
        let target = dictionary! {
            "Subtype"=>"Stamp",
            "Rect"=>vec![10.into(),20.into(),30.into(),40.into()],
            "F"=>Object::string_literal("not-an-integer"),
        };
        set_annots(doc, pages[0], vec![target.into()]);
    });
    let mut session = Session::open(&bytes).unwrap();
    let row = session.annotations().unwrap().remove(0);
    session.apply(generic_delete(&row)).unwrap();
    assert!(session.annotations().unwrap().is_empty());
}

#[test]
fn deletion_is_copy_on_write_and_preserves_page_streams_resources_and_owned_objects() {
    let mut preserved = Vec::new();
    let bytes = fixture(|doc, pages| {
        let first_page_stream = doc.add_object(Stream::new(
            dictionary! {"Piece"=>"first", "Version"=>1},
            b"q 1 0 0 1 5 6 cm Q".to_vec(),
        ));
        let mut second_stream = Stream::new(
            dictionary! {"Piece"=>"second", "Version"=>2},
            b"q 1 0 0 1 1 1 cm Q\n".repeat(128),
        );
        second_stream.compress().unwrap();
        let second_page_stream = doc.add_object(second_stream);
        let nested_contents = doc.add_object(vec![second_page_stream.into()]);
        let font = doc
            .add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let resources = doc.add_object(dictionary! {"Font"=>dictionary! {"A"=>font}});
        let appearance = doc.add_object(Stream::new(
            dictionary! {"Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),100.into(),40.into()], "Resources"=>resources},
            b"q 0 g Q".to_vec(),
        ));
        let contents = doc.add_object(Object::string_literal("indirect contents"));
        let mut target = annotation("ignored");
        let target_dictionary = target.as_dict_mut().unwrap();
        target_dictionary.set("Contents", contents);
        target_dictionary.set("AP", dictionary! {"N"=>appearance});
        let root = doc.add_object(target);
        let popup = doc.add_object(dictionary! {"Subtype"=>"Popup", "Parent"=>root});
        doc.get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Popup", popup);
        let annots = doc.add_object(vec![root.into(), popup.into()]);
        for page in pages {
            let page_dictionary = doc.get_object_mut(*page).unwrap().as_dict_mut().unwrap();
            page_dictionary.set("Annots", annots);
            page_dictionary.set("Resources", resources);
            page_dictionary.set(
                "Contents",
                vec![first_page_stream.into(), nested_contents.into()],
            );
        }
        preserved = vec![
            annots,
            root,
            popup,
            appearance,
            contents,
            resources,
            font,
            first_page_stream,
            second_page_stream,
            nested_contents,
        ];
    });
    let original = Document::load_mem(&bytes).unwrap();
    let mut session = Session::open(&bytes).unwrap();
    let expected = session.free_texts(0).unwrap().annotations.remove(0);
    session.delete_free_text(&expected).unwrap();
    let after_bytes = session.bytes().unwrap();
    let after = Document::load_mem(&after_bytes).unwrap();
    for id in preserved {
        assert_eq!(
            after.get_object(id).unwrap(),
            original.get_object(id).unwrap()
        );
    }
    let original_pages = original.get_pages().values().copied().collect::<Vec<_>>();
    let after_pages = after.get_pages().values().copied().collect::<Vec<_>>();
    for index in 0..2 {
        let before_page = original
            .get_object(original_pages[index])
            .unwrap()
            .as_dict()
            .unwrap();
        let after_page = after
            .get_object(after_pages[index])
            .unwrap()
            .as_dict()
            .unwrap();
        assert_eq!(
            after_page.get(b"Resources").unwrap(),
            before_page.get(b"Resources").unwrap()
        );
        assert_eq!(
            resolved_page_streams(&after, after_pages[index]),
            resolved_page_streams(&original, original_pages[index])
        );
        let streams = resolved_page_streams(&after, after_pages[index]);
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].1, b"q 1 0 0 1 5 6 cm Q");
        assert!(!streams[1].1.is_empty());
        assert_eq!(
            streams[1].0.get(b"Filter").unwrap().as_name().unwrap(),
            b"FlateDecode"
        );
    }
    assert!(
        Session::open(&after_bytes)
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .is_empty()
    );
    assert_eq!(
        Session::open(&after_bytes)
            .unwrap()
            .free_texts(1)
            .unwrap()
            .annotations
            .len(),
        1
    );
}

#[test]
fn relationship_and_cross_page_budget_refusals_preserve_real_undo_and_redo() {
    for case in [
        "struct",
        "outgoing",
        "incoming",
        "malformed-popup",
        "ambiguous",
        "budget",
    ] {
        let bytes = fixture(|doc, pages| {
            let root = doc.add_object(annotation("selected"));
            let mut first = vec![root.into()];
            match case {
                "struct" => doc
                    .get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("StructParent", 1),
                "outgoing" => doc
                    .get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("IRT", root),
                "incoming" => set_annots(
                    doc,
                    pages[1],
                    vec![dictionary! {"Subtype"=>"Stamp", "IRT"=>root}.into()],
                ),
                "malformed-popup" => doc
                    .get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Popup", dictionary! {"Subtype"=>"Popup", "Parent"=>root}),
                "ambiguous" => {
                    first.push(
                        doc.add_object(dictionary! {"Subtype"=>"Popup", "Parent"=>root})
                            .into(),
                    );
                    first.push(
                        doc.add_object(dictionary! {"Subtype"=>"Popup", "Parent"=>root})
                            .into(),
                    );
                }
                _ => set_annots(
                    doc,
                    pages[1],
                    (0..16_384)
                        .map(|_| dictionary! {"Subtype"=>"Stamp"}.into())
                        .collect(),
                ),
            }
            set_annots(doc, pages[0], first);
        });
        let mut session = Session::open(&bytes).unwrap();
        let (original, first, second) = prepare_both_history(&mut session);
        let expected = session.free_texts(0).unwrap().annotations.remove(0);
        let info = session.info();
        assert!(session.delete_free_text(&expected).is_err(), "{case}");
        assert_eq!(session.bytes().unwrap(), first, "{case}");
        assert_eq!(session.info(), info, "{case}");
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), original, "{case}");
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), first, "{case}");
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), second, "{case}");
    }
}
