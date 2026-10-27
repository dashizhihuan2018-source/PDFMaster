use lopdf::{Document, Object, ObjectId, Stream, dictionary, text_string};
use pdfmaster_core::{AnnotationRow, Edit, Session};

fn fixture(build: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    let mut doc =
        Document::load_mem(&Session::blank(200., 120., 2).unwrap().bytes().unwrap()).unwrap();
    let pages = doc.get_pages().values().copied().collect::<Vec<_>>();
    build(&mut doc, &pages);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn annotation(kind: &str) -> Object {
    dictionary! {"Subtype"=>Object::Name(kind.as_bytes().to_vec()), "Contents"=>text_string("中文 🦀"), "Rect"=>vec![10.into(),20.into(),50.into(),60.into()], "M"=>Object::string_literal("D:20200102030405Z")}.into()
}

fn set_annots(doc: &mut Document, page: ObjectId, entries: Vec<Object>) {
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", entries);
}

fn delete(expected: &AnnotationRow) -> Edit {
    serde_json::from_value(serde_json::json!({"kind":"delete_annotation","expected":expected}))
        .expect("the unified annotation operation must decode")
}

fn unchanged_on_error(session: &mut Session, expected: &AnnotationRow) {
    let bytes = session.bytes().unwrap();
    let info = session.info();
    assert!(session.apply(delete(expected)).is_err());
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(session.info(), info);
}

#[test]
fn unified_delete_removes_membership_for_every_sidebar_type_with_exact_undo_redo() {
    for kind in [
        "Text",
        "FreeText",
        "Highlight",
        "Underline",
        "StrikeOut",
        "Ink",
        "Square",
        "Circle",
        "Line",
        "Stamp",
        "Caret",
        "FileAttachment",
        "Sound",
        "Redact",
        "UnknownVendorType",
    ] {
        let bytes = fixture(|doc, pages| {
            set_annots(
                doc,
                pages[0],
                vec![annotation("Link"), annotation(kind), annotation("Stamp")],
            )
        });
        let mut session = Session::open(&bytes).unwrap();
        let rows = session.annotations().unwrap();
        assert_eq!(rows[0].annotation_index, 1);
        session.apply(delete(&rows[0])).unwrap();
        let after = session.bytes().unwrap();
        assert_eq!(session.annotations().unwrap().len(), 1, "{kind}");
        assert_eq!(session.annotations().unwrap()[0].subtype, "Stamp");
        let doc = Document::load_mem(&after).unwrap();
        let page = doc.get_pages()[&1];
        let annots = doc
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap();
        assert_eq!(
            doc.dereference(annots).unwrap().1.as_array().unwrap().len(),
            2
        );
        assert!(session.info().dirty);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), bytes);
        assert!(!session.info().dirty);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
        let reopened = Session::open(&after).unwrap();
        assert_eq!(reopened.annotations().unwrap().len(), 1);
    }
}

#[test]
fn stale_identity_or_metadata_cannot_delete_a_different_row_or_discard_redo() {
    let bytes = fixture(|doc, pages| {
        let id = doc.add_object(annotation("Stamp"));
        set_annots(doc, pages[0], vec![id.into()]);
    });
    let mut session = Session::open(&bytes).unwrap();
    let row = session.annotations().unwrap().remove(0);
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
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
        let mut value = serde_json::to_value(&row).unwrap();
        value[field] = match field {
            "page_index" | "annotation_index" => 1.into(),
            "page_id" | "object_id" => serde_json::json!([999, 0]),
            "subtype" | "contents" | "modified" => "stale".into(),
            "color" => serde_json::json!({"r":1.,"g":0.,"b":0.}),
            _ => serde_json::json!({"x":1.,"y":1.,"width":2.,"height":2.}),
        };
        unchanged_on_error(&mut session, &serde_json::from_value(value).unwrap());
        assert!(session.info().can_redo);
    }
    session.apply(Edit::MovePage { from: 0, to: 1 }).unwrap();
    unchanged_on_error(&mut session, &row);
}

#[test]
fn shared_array_target_and_popup_are_copy_on_write_not_object_erasure() {
    let mut saved_ids = None;
    let bytes = fixture(|doc, pages| {
        let appearance = doc.add_object(Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),40.into(),40.into()]},
            b"1 0 0 RG 1 w 1 1 m 39 39 l S".to_vec(),
        ));
        let payload = doc.add_object(Stream::new(
            dictionary! {"Type"=>"EmbeddedFile"},
            b"self-created attachment bytes\0only".to_vec(),
        ));
        let file = doc.add_object(
            dictionary! {"Type"=>"Filespec","F"=>"qa.txt","EF"=>dictionary! {"F"=>payload}},
        );
        let mut annotation = annotation("FileAttachment");
        annotation
            .as_dict_mut()
            .unwrap()
            .set("AP", dictionary! {"N"=>appearance});
        annotation.as_dict_mut().unwrap().set("FS", file);
        let root = doc.add_object(annotation);
        let popup = doc.add_object(dictionary! {"Subtype"=>"Popup","Parent"=>root});
        doc.get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Popup", popup);
        let array = doc.add_object(vec![root.into(), popup.into()]);
        for &page in pages {
            doc.get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", array);
        }
        saved_ids = Some(vec![array, root, popup, appearance, payload, file]);
    });
    let original = Document::load_mem(&bytes).unwrap();
    let mut session = Session::open(&bytes).unwrap();
    session
        .apply(delete(&session.annotations().unwrap()[0]))
        .unwrap();
    let rows = session.annotations().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].page_index, 1);
    let after = Document::load_mem(&session.bytes().unwrap()).unwrap();
    for id in saved_ids.unwrap() {
        assert_eq!(
            after.get_object(id).unwrap(),
            original.get_object(id).unwrap()
        );
    }
    let ids = after.get_pages();
    assert_eq!(
        after.get_object(ids[&2]).unwrap(),
        original.get_object(ids[&2]).unwrap()
    );
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes);
}

#[test]
fn generic_removal_does_not_require_editable_geometry_or_text_note_parser_limits() {
    let bytes = fixture(|doc, pages| {
        let mut target = annotation("Text");
        target.as_dict_mut().unwrap().set("Rect", Object::Null);
        let mut entries = vec![annotation("Link"); 1500];
        entries.push(target);
        set_annots(doc, pages[0], entries);
    });
    let mut session = Session::open(&bytes).unwrap();
    let row = session.annotations().unwrap().remove(0);
    assert!(row.bounds.is_none());
    assert_eq!(row.annotation_index, 1500);
    session.apply(delete(&row)).unwrap();
    assert!(session.annotations().unwrap().is_empty());
}

#[test]
fn tagged_threaded_ambiguous_and_cyclic_relationships_fail_atomically() {
    for case in ["tagged", "reply", "reverse-reply", "ambiguous", "cycle"] {
        let bytes = fixture(|doc, pages| {
            let root = doc.add_object(annotation("Ink"));
            let mut entries = vec![root.into()];
            match case {
                "tagged" => {
                    doc.get_object_mut(root)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("StructParent", 1);
                }
                "reply" => {
                    doc.get_object_mut(root)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("IRT", root);
                }
                "reverse-reply" => {
                    let mut reply = annotation("Text");
                    reply.as_dict_mut().unwrap().set("IRT", root);
                    set_annots(doc, pages[1], vec![reply]);
                }
                "ambiguous" => {
                    for _ in 0..2 {
                        let p = doc.add_object(dictionary! {"Subtype"=>"Popup","Parent"=>root});
                        entries.push(p.into());
                    }
                }
                _ => {
                    let p = doc.new_object_id();
                    doc.objects.insert(p, Object::Reference(p));
                    doc.get_object_mut(root)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Popup", p);
                }
            }
            set_annots(doc, pages[0], entries);
        });
        let mut session = Session::open(&bytes).unwrap();
        let row = session.annotations().unwrap().remove(0);
        unchanged_on_error(&mut session, &row);
    }
}

#[test]
fn filtered_widgets_links_and_popups_are_never_delete_targets() {
    for kind in ["Widget", "Link", "Popup"] {
        let bytes = fixture(|doc, pages| {
            set_annots(doc, pages[0], vec![annotation(kind), annotation("Stamp")])
        });
        let mut session = Session::open(&bytes).unwrap();
        let mut row = session.annotations().unwrap().remove(0);
        row.annotation_index = 0;
        row.subtype = kind.into();
        unchanged_on_error(&mut session, &row);
    }
}
