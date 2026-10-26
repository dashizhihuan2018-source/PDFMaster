use lopdf::{Dictionary, Document, Object, Stream, StringFormat, dictionary, text_string};
use pdfmaster_core::{Color, Edit, NoteList, Rect, Session, TextNote};

const MIB: usize = 1024 * 1024;

fn fixture_pdf(page_count: usize) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::new();
    for _ in 0..page_count {
        let contents = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
        page_ids.push(document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 120.into()],
            "Resources" => Dictionary::new(),
            "Contents" => contents,
        }));
    }
    document.objects.insert(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_count as i64,
        }
        .into(),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    save(document)
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn text_annotation(contents: Option<Object>, rect: Object, color: Option<Object>) -> Object {
    let mut annotation = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Text",
        "Rect" => rect,
    };
    if let Some(contents) = contents {
        annotation.set("Contents", contents);
    }
    if let Some(color) = color {
        annotation.set("C", color);
    }
    annotation.into()
}

fn valid_rect() -> Object {
    vec![10.into(), 20.into(), 50.into(), 70.into()].into()
}

fn rgb(r: f32, g: f32, b: f32) -> Object {
    vec![r.into(), g.into(), b.into()].into()
}

fn set_annots(document: &mut Document, page_number: u32, annots: Object) {
    let page_id = document.get_pages()[&page_number];
    document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", annots);
}

fn document_with_annots(page_number: u32, annots: Object) -> Vec<u8> {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    set_annots(&mut document, page_number, annots);
    save(document)
}

fn document_with_annots_reference_chain(reference_count: usize) -> Vec<u8> {
    let mut document = Document::load_mem(&fixture_pdf(1)).unwrap();
    let mut annots = Object::Array(Vec::new());
    for _ in 0..reference_count {
        annots = Object::Reference(document.add_object(annots));
    }
    set_annots(&mut document, 1, annots);
    save(document)
}

fn document_with_annots_reference_cycle() -> Vec<u8> {
    let mut document = Document::load_mem(&fixture_pdf(1)).unwrap();
    let first = document.new_object_id();
    let second = document.new_object_id();
    document.objects.insert(first, Object::Reference(second));
    document.objects.insert(second, Object::Reference(first));
    set_annots(&mut document, 1, Object::Reference(first));
    save(document)
}

fn utf16le(value: &str) -> Object {
    let mut bytes = vec![0xff, 0xfe];
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Object::String(bytes, StringFormat::Hexadecimal)
}

fn annotation_dictionary(document: &Document, page_number: u32, index: usize) -> Dictionary {
    let page = document
        .get_object(document.get_pages()[&page_number])
        .unwrap()
        .as_dict()
        .unwrap();
    let (_, annots) = document.dereference(page.get(b"Annots").unwrap()).unwrap();
    let (_, annotation) = document
        .dereference(&annots.as_array().unwrap()[index])
        .unwrap();
    annotation.as_dict().unwrap().clone()
}

fn assert_apply_unchanged(session: &mut Session, edit: Edit) {
    let before_bytes = session.bytes().unwrap();
    let before_info = session.info();
    assert!(session.apply(edit).is_err());
    assert_eq!(session.bytes().unwrap(), before_bytes);
    assert_eq!(session.info(), before_info);
}

#[test]
fn note_lifecycle_rejects_malformed_or_cyclic_reply_references_without_mutation() {
    for case in [
        "direct",
        "missing",
        "cycle",
        "depth64",
        "depth65",
        "generation",
    ] {
        let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
        let note = document.add_object(text_annotation(
            Some(text_string("root")),
            valid_rect(),
            None,
        ));
        let parent = match case {
            "direct" => Object::Integer(7),
            "missing" => Object::Reference((99999, 0)),
            "cycle" => {
                let id = document.new_object_id();
                document.objects.insert(id, Object::Reference(id));
                Object::Reference(id)
            }
            "generation" => Object::Reference((note.0, note.1 + 1)),
            _ => {
                let mut parent = Object::Reference(note);
                let hops = if case == "depth64" { 64 } else { 65 };
                for _ in 1..hops {
                    parent = Object::Reference(document.add_object(parent));
                }
                parent
            }
        };
        let mut reply = text_annotation(None, valid_rect(), None)
            .as_dict()
            .unwrap()
            .clone();
        reply.set("IRT", parent);
        set_annots(&mut document, 1, vec![Object::Reference(note)].into());
        set_annots(&mut document, 2, vec![Object::Dictionary(reply)].into());
        let mut session = Session::open(&save(document)).unwrap();
        let before = session.bytes().unwrap();
        let info = session.info();
        let error = session
            .apply(delete_note(0, 0, "root"))
            .unwrap_err()
            .to_string();
        assert_eq!(session.bytes().unwrap(), before);
        assert_eq!(session.info(), info);
        if case == "depth64" {
            assert!(error.contains("threaded"), "{error}");
        }
        if case == "depth65" {
            assert!(error.contains("depth exceeded"), "{error}");
        }
    }
}

#[test]
fn note_relationship_scan_accepts_exact_16384_and_rejects_16385_entries_atomically() {
    for total in [16_384, 16_385] {
        let mut document = Document::load_mem(&fixture_pdf(17)).unwrap();
        let note = document.add_object(text_annotation(
            Some(text_string("root")),
            valid_rect(),
            None,
        ));
        set_annots(&mut document, 1, vec![Object::Reference(note)].into());
        let mut remaining: usize = total - 1;
        for page in 2..=17 {
            let count = remaining.min(1024);
            let entries = (0..count)
                .map(|_| Object::Dictionary(dictionary! {"Subtype"=>"Link", "Rect"=>valid_rect()}))
                .collect::<Vec<_>>();
            set_annots(&mut document, page, entries.into());
            remaining -= count;
        }
        assert_eq!(remaining, 0);
        let mut session = Session::open(&save(document)).unwrap();
        if total == 16_384 {
            session.apply(delete_note(0, 0, "root")).unwrap();
            assert_eq!(session.notes(0).unwrap(), NoteList::default());
            assert_eq!(session.notes(1).unwrap().unsupported_count, 1024);
        } else {
            assert_apply_unchanged(&mut session, delete_note(0, 0, "root"));
        }
    }
}

#[test]
fn note_relationship_scan_rejects_malformed_other_page_without_mutation() {
    for invalid in [
        Object::Integer(5),
        Object::Reference((99999, 0)),
        vec![Object::Integer(5)].into(),
    ] {
        let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
        let note = document.add_object(text_annotation(
            Some(text_string("root")),
            valid_rect(),
            None,
        ));
        set_annots(&mut document, 1, vec![Object::Reference(note)].into());
        set_annots(&mut document, 2, invalid);
        let mut session = Session::open(&save(document)).unwrap();
        assert_apply_unchanged(&mut session, delete_note(0, 0, "root"));
    }
}

fn delete_note(page: usize, annotation_index: usize, contents: &str) -> Edit {
    serde_json::from_value(serde_json::json!({
        "kind":"delete_note", "page":page, "annotation_index":annotation_index,
        "expected_contents":contents,
    }))
    .unwrap()
}

#[test]
fn note_popup_lifecycle_supports_optional_parent_and_reverse_links_but_never_claims_foreign_popups()
{
    for relationship in ["forward_only", "reverse_only", "foreign", "duplicate_root"] {
        let mut document = Document::load_mem(&fixture_pdf(1)).unwrap();
        let root = document.add_object(text_annotation(None, valid_rect(), None));
        let other = document.add_object(text_annotation(
            Some(text_string("other")),
            valid_rect(),
            None,
        ));
        let mut popup = dictionary! {"Subtype"=>"Popup", "Rect"=>valid_rect(), "Open"=>true};
        if relationship != "forward_only" {
            popup.set(
                "Parent",
                if relationship == "foreign" {
                    other
                } else {
                    root
                },
            );
        }
        let popup = document.add_object(popup);
        if relationship != "reverse_only" {
            document
                .get_object_mut(root)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Popup", popup);
        }
        let mut annots = vec![
            Object::Reference(root),
            Object::Reference(popup),
            Object::Reference(other),
        ];
        if relationship == "duplicate_root" {
            annots.push(Object::Reference(root));
        }
        set_annots(&mut document, 1, annots.into());
        let source = save(document);
        let mut session = Session::open(&source).unwrap();
        let edit = Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: String::new(),
            contents: "changed".into(),
        };
        if relationship == "foreign" || relationship == "duplicate_root" {
            assert_apply_unchanged(&mut session, edit);
            assert_apply_unchanged(&mut session, delete_note(0, 0, ""));
            continue;
        }
        session.apply(edit).unwrap();
        let edited = session.bytes().unwrap();
        let parsed = Document::load_mem(&edited).unwrap();
        let root_dict = annotation_dictionary(&parsed, 1, 0);
        assert_eq!(root_dict.get(b"Popup").unwrap(), &{
            let page = parsed
                .get_object(parsed.get_pages()[&1])
                .unwrap()
                .as_dict()
                .unwrap();
            let (_, entries) = parsed.dereference(page.get(b"Annots").unwrap()).unwrap();
            entries.as_array().unwrap()[1].clone()
        });
        session.apply(delete_note(0, 0, "changed")).unwrap();
        assert_eq!(session.notes(0).unwrap().notes[0].contents, "other");
        assert_eq!(session.notes(0).unwrap().unsupported_count, 0);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), edited);
    }
}

#[test]
fn note_lifecycle_rejects_replies_to_the_selected_root_on_another_page_atomically() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let note = document.add_object(text_annotation(
        Some(text_string("thread root")),
        valid_rect(),
        None,
    ));
    let mut reply = text_annotation(Some(text_string("reply")), valid_rect(), None)
        .as_dict()
        .unwrap()
        .clone();
    reply.set("IRT", note);
    set_annots(&mut document, 1, vec![Object::Reference(note)].into());
    set_annots(&mut document, 2, vec![Object::Dictionary(reply)].into());
    let mut session = Session::open(&save(document)).unwrap();
    assert_apply_unchanged(&mut session, delete_note(0, 0, "thread root"));
    assert_apply_unchanged(
        &mut session,
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "thread root".into(),
            contents: "changed".into(),
        },
    );
    assert_apply_unchanged(&mut session, delete_note(1, 0, "reply"));
}

#[test]
fn update_note_rebinds_existing_page_backlinks_in_copied_root_and_popup() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let page1 = document.get_pages()[&1];
    let page2 = document.get_pages()[&2];
    let note = document.add_object(text_annotation(
        Some(text_string("old")),
        valid_rect(),
        None,
    ));
    let popup = document.add_object(
        dictionary! {"Subtype"=>"Popup", "Rect"=>valid_rect(), "Parent"=>note, "P"=>page1},
    );
    let root = document
        .get_object_mut(note)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root.set("Popup", popup);
    root.set("P", page1);
    let annots = document.add_object(vec![Object::Reference(note), Object::Reference(popup)]);
    set_annots(&mut document, 1, Object::Reference(annots));
    set_annots(&mut document, 2, Object::Reference(annots));
    let mut session = Session::open(&save(document)).unwrap();
    session
        .apply(Edit::UpdateNote {
            page: 1,
            annotation_index: 0,
            expected_contents: "old".into(),
            contents: "new".into(),
        })
        .unwrap();
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    for index in [0, 1] {
        assert_eq!(
            annotation_dictionary(&parsed, 2, index).get(b"P").unwrap(),
            &Object::Reference(page2)
        );
        assert_eq!(
            annotation_dictionary(&parsed, 1, index).get(b"P").unwrap(),
            &Object::Reference(page1)
        );
    }
}

#[test]
fn updated_note_popup_is_rebound_on_its_page_and_can_be_deleted_without_changing_shared_page() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let note = document.add_object(text_annotation(
        Some(text_string("before")),
        valid_rect(),
        None,
    ));
    let popup = document.add_object(dictionary! {
        "Subtype"=>"Popup", "Rect"=>valid_rect(), "Parent"=>note,
        "Open"=>true, "P"=>document.get_pages()[&1],
    });
    document
        .get_object_mut(note)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Popup", popup);
    let annots = document.add_object(vec![Object::Reference(note), Object::Reference(popup)]);
    set_annots(&mut document, 1, Object::Reference(annots));
    set_annots(&mut document, 2, Object::Reference(annots));
    let source = save(document);
    let mut session = Session::open(&source).unwrap();
    let untouched = session.notes(1).unwrap();
    session
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "before".into(),
            contents: "after 🦀".into(),
        })
        .unwrap();
    let edited = session.bytes().unwrap();
    let parsed = Document::load_mem(&edited).unwrap();
    let updated_note = annotation_dictionary(&parsed, 1, 0);
    let updated_popup = annotation_dictionary(&parsed, 1, 1);
    let page = parsed
        .get_object(parsed.get_pages()[&1])
        .unwrap()
        .as_dict()
        .unwrap();
    let (_, entries) = parsed.dereference(page.get(b"Annots").unwrap()).unwrap();
    let entries = entries.as_array().unwrap();
    assert_eq!(updated_popup.get(b"Parent").unwrap(), &entries[0]);
    assert_eq!(updated_note.get(b"Popup").unwrap(), &entries[1]);
    assert_eq!(updated_popup.get(b"Open").unwrap(), &Object::Boolean(true));
    assert_ne!(entries[0], Object::Reference(note));
    assert_ne!(entries[1], Object::Reference(popup));
    assert_eq!(session.notes(1).unwrap(), untouched);
    assert_eq!(
        annotation_dictionary(&parsed, 2, 1).get(b"Parent").unwrap(),
        &Object::Reference(note)
    );
    session.apply(delete_note(0, 0, "after 🦀")).unwrap();
    assert_eq!(session.notes(0).unwrap(), NoteList::default());
    assert_eq!(session.notes(1).unwrap(), untouched);
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), edited);
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), source);
}

#[test]
fn delete_note_reopens_and_undoes_without_changing_other_annotations_or_source() {
    for indirect in [false, true] {
        let mut document = Document::load_mem(&fixture_pdf(1)).unwrap();
        let note = text_annotation(Some(text_string("删除 🦀")), valid_rect(), None);
        let retained = text_annotation(Some(text_string("保留")), valid_rect(), None);
        let values = if indirect {
            vec![
                Object::Reference(document.add_object(note)),
                Object::Reference(document.add_object(retained)),
            ]
        } else {
            vec![note, retained]
        };
        let annots = if indirect {
            Object::Reference(document.add_object(values))
        } else {
            Object::Array(values)
        };
        set_annots(&mut document, 1, annots);
        let source = save(document);
        let mut session = Session::open(&source).unwrap();
        let text = session.text(0).unwrap();
        session.apply(delete_note(0, 0, "删除 🦀")).unwrap();
        assert_eq!(session.notes(0).unwrap().notes.len(), 1);
        assert_eq!(session.notes(0).unwrap().notes[0].contents, "保留");
        assert_eq!(session.text(0).unwrap(), text);
        assert_eq!(
            Session::open(&source)
                .unwrap()
                .notes(0)
                .unwrap()
                .notes
                .len(),
            2
        );
        let exported = session.bytes().unwrap();
        assert_eq!(
            Session::open(&exported).unwrap().notes(0).unwrap(),
            session.notes(0).unwrap()
        );
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), source);
        assert!(!session.info().dirty);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), exported);
    }
}

#[test]
fn delete_note_copies_shared_annots_and_removes_only_its_associated_popup() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let note = document.add_object(text_annotation(
        Some(text_string("with popup")),
        valid_rect(),
        None,
    ));
    let popup = document.add_object(dictionary! {
        "Type"=>"Annot", "Subtype"=>"Popup", "Rect"=>valid_rect(), "Parent"=>note,
    });
    document
        .get_object_mut(note)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Popup", popup);
    let retained = document.add_object(text_annotation(
        Some(text_string("keep")),
        valid_rect(),
        None,
    ));
    let annots = document.add_object(vec![
        Object::Reference(popup),
        Object::Reference(retained),
        Object::Reference(note),
    ]);
    set_annots(&mut document, 1, Object::Reference(annots));
    set_annots(&mut document, 2, Object::Reference(annots));
    let mut session = Session::open(&save(document)).unwrap();
    let other_page = session.notes(1).unwrap();
    session.apply(delete_note(0, 2, "with popup")).unwrap();
    assert_eq!(session.notes(0).unwrap().unsupported_count, 0);
    assert_eq!(session.notes(0).unwrap().notes.len(), 1);
    assert_eq!(session.notes(0).unwrap().notes[0].contents, "keep");
    assert_eq!(session.notes(1).unwrap(), other_page);
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    assert_eq!(
        annotation_dictionary(&parsed, 1, 0)
            .get(b"Contents")
            .unwrap(),
        &text_string("keep")
    );
    // Shared objects remain available to the untouched second page, not erased.
    assert!(parsed.objects.contains_key(&note));
    assert!(parsed.objects.contains_key(&popup));
}

#[test]
fn delete_last_note_is_valid_and_stale_or_invalid_deletes_preserve_history() {
    let source = document_with_annots(
        1,
        vec![text_annotation(
            Some(text_string("last")),
            valid_rect(),
            None,
        )]
        .into(),
    );
    let mut session = Session::open(&source).unwrap();
    for edit in [
        delete_note(2, 0, "last"),
        delete_note(0, 1, "last"),
        delete_note(0, 0, "stale"),
        delete_note(0, 0, &"x".repeat(MIB + 1)),
    ] {
        assert_apply_unchanged(&mut session, edit);
    }
    session.apply(delete_note(0, 0, "last")).unwrap();
    assert_eq!(session.notes(0).unwrap(), NoteList::default());
    assert!(
        Session::open(&session.bytes().unwrap())
            .unwrap()
            .notes(0)
            .unwrap()
            .notes
            .is_empty()
    );
    session.undo().unwrap();
    assert!(session.info().can_redo);
    assert_apply_unchanged(&mut session, delete_note(0, 0, "stale"));
    assert!(session.info().can_redo);
}

#[test]
fn delete_note_rejects_malformed_popup_ownership_and_tagged_notes_atomically() {
    for invalid in ["wrong_parent", "wrong_subtype", "struct_parent"] {
        let mut document = Document::load_mem(&fixture_pdf(1)).unwrap();
        let note = document.add_object(text_annotation(
            Some(text_string("target")),
            valid_rect(),
            None,
        ));
        let other = document.add_object(text_annotation(
            Some(text_string("other")),
            valid_rect(),
            None,
        ));
        let popup = document.add_object(dictionary! {
            "Subtype"=> if invalid=="wrong_subtype" {"Text"}else{"Popup"},
            "Rect"=>valid_rect(), "Parent"=>if invalid=="wrong_parent" {other}else{note},
        });
        let target = document
            .get_object_mut(note)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        target.set("Popup", popup);
        if invalid == "struct_parent" {
            target.set("StructParent", 0);
        }
        set_annots(
            &mut document,
            1,
            vec![
                Object::Reference(note),
                Object::Reference(popup),
                Object::Reference(other),
            ]
            .into(),
        );
        let mut session = Session::open(&save(document)).unwrap();
        assert_apply_unchanged(&mut session, delete_note(0, 0, "target"));
    }
}

#[test]
fn delete_note_cannot_remove_an_unsupported_annotation_and_wire_tag_is_stable() {
    let source = document_with_annots(
        1,
        vec![Object::Dictionary(
            dictionary! {"Subtype"=>"Link","Rect"=>valid_rect()},
        )]
        .into(),
    );
    let mut session = Session::open(&source).unwrap();
    assert_apply_unchanged(&mut session, delete_note(0, 0, ""));
    let edit = delete_note(1, 4, "中文");
    assert_eq!(
        serde_json::to_value(edit).unwrap(),
        serde_json::json!({"kind":"delete_note","page":1,"annotation_index":4,"expected_contents":"中文"})
    );
}

#[test]
fn delete_note_removes_a_popup_identified_by_parent_when_forward_link_is_absent() {
    let mut document = Document::load_mem(&fixture_pdf(1)).unwrap();
    let note = document.add_object(text_annotation(
        Some(text_string("reverse")),
        valid_rect(),
        None,
    ));
    let popup =
        document.add_object(dictionary! {"Subtype"=>"Popup", "Rect"=>valid_rect(), "Parent"=>note});
    set_annots(
        &mut document,
        1,
        vec![Object::Reference(note), Object::Reference(popup)].into(),
    );
    let mut session = Session::open(&save(document)).unwrap();
    session.apply(delete_note(0, 0, "reverse")).unwrap();
    assert_eq!(session.notes(0).unwrap(), NoteList::default());
}

#[test]
fn notes_enumerate_direct_and_indirect_text_annotations_with_unicode_and_color() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let indirect = document.add_object(text_annotation(
        Some(text_string("批注 🚀")),
        valid_rect(),
        Some(rgb(0.25, 0.5, 0.75)),
    ));
    let direct = text_annotation(
        Some(utf16le("little 😀")),
        vec![60.into(), 10.into(), 90.into(), 40.into()].into(),
        None,
    );
    let pdf_doc = text_annotation(
        Some(Object::String(b"rate \x8b".to_vec(), StringFormat::Literal)),
        vec![100.into(), 20.into(), 150.into(), 50.into()].into(),
        Some(rgb(1.0, 0.0, 0.0)),
    );
    let unsupported = Object::Dictionary(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![0.into(), 0.into(), 5.into(), 5.into()],
    });
    let annots = document.add_object(vec![
        Object::Reference(indirect),
        unsupported,
        direct,
        pdf_doc,
    ]);
    set_annots(&mut document, 2, Object::Reference(annots));

    let session = Session::open(&save(document)).unwrap();
    assert_eq!(session.notes(0).unwrap(), NoteList::default());
    assert_eq!(
        session.notes(1).unwrap(),
        NoteList {
            notes: vec![
                TextNote {
                    annotation_index: 0,
                    bounds: Rect {
                        x: 10.0,
                        y: 20.0,
                        width: 40.0,
                        height: 50.0,
                    },
                    contents: "批注 🚀".into(),
                    color: Some(Color {
                        r: 0.25,
                        g: 0.5,
                        b: 0.75,
                    }),
                },
                TextNote {
                    annotation_index: 2,
                    bounds: Rect {
                        x: 60.0,
                        y: 10.0,
                        width: 30.0,
                        height: 30.0,
                    },
                    contents: "little 😀".into(),
                    color: None,
                },
                TextNote {
                    annotation_index: 3,
                    bounds: Rect {
                        x: 100.0,
                        y: 20.0,
                        width: 50.0,
                        height: 30.0,
                    },
                    contents: "rate ‰".into(),
                    color: Some(Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                    }),
                },
            ],
            unsupported_count: 1,
        }
    );
}

#[test]
fn notes_missing_contents_is_empty_and_public_values_round_trip_json() {
    let bytes = document_with_annots(1, vec![text_annotation(None, valid_rect(), None)].into());
    let notes = Session::open(&bytes).unwrap().notes(0).unwrap();
    assert_eq!(notes.notes[0].contents, "");
    let json = serde_json::to_string(&notes).unwrap();
    assert_eq!(serde_json::from_str::<NoteList>(&json).unwrap(), notes);
}

#[test]
fn notes_is_read_only_and_add_note_survives_export_reopen_undo_and_redo() {
    let mut session = Session::blank(200.0, 120.0, 2).unwrap();
    session
        .apply(Edit::AddNote {
            page: 1,
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 50.0,
            },
            contents: "saved 📝".into(),
            color: Color {
                r: 1.0,
                g: 0.8,
                b: 0.0,
            },
        })
        .unwrap();
    let expected = session.notes(1).unwrap();
    assert_eq!(expected.notes.len(), 1);
    assert_eq!(expected.notes[0].contents, "saved 📝");

    let before_bytes = session.bytes().unwrap();
    let before_info = session.info();
    assert_eq!(session.notes(1).unwrap(), expected);
    assert_eq!(session.bytes().unwrap(), before_bytes);
    assert_eq!(session.info(), before_info);

    let reopened = Session::open(&before_bytes).unwrap();
    assert_eq!(reopened.notes(1).unwrap(), expected);
    session.undo().unwrap();
    assert_eq!(session.notes(1).unwrap(), NoteList::default());
    session.redo().unwrap();
    assert_eq!(session.notes(1).unwrap(), expected);
}

#[test]
fn update_note_unicode_reopens_and_undoes_while_preserving_annotation_fields() {
    let mut session = Session::blank(200.0, 120.0, 1).unwrap();
    session
        .apply(Edit::AddNote {
            page: 0,
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 50.0,
            },
            contents: "old ✏️".into(),
            color: Color {
                r: 1.0,
                g: 0.8,
                b: 0.0,
            },
        })
        .unwrap();
    let before = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let before_annotation = annotation_dictionary(&before, 1, 0);

    session
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "old ✏️".into(),
            contents: "新内容 🚀".into(),
        })
        .unwrap();
    assert_eq!(session.notes(0).unwrap().notes[0].contents, "新内容 🚀");
    let exported = session.bytes().unwrap();
    assert_eq!(
        Session::open(&exported).unwrap().notes(0).unwrap().notes[0].contents,
        "新内容 🚀"
    );

    let after = Document::load_mem(&exported).unwrap();
    let after_annotation = annotation_dictionary(&after, 1, 0);
    for key in [b"Rect".as_slice(), b"C", b"AP", b"F", b"Name"] {
        assert_eq!(
            before_annotation.get(key).unwrap(),
            after_annotation.get(key).unwrap()
        );
    }

    session.undo().unwrap();
    assert_eq!(session.notes(0).unwrap().notes[0].contents, "old ✏️");
    session.redo().unwrap();
    assert_eq!(session.notes(0).unwrap().notes[0].contents, "新内容 🚀");
}

#[test]
fn update_note_handles_direct_and_indirect_containers_and_preserves_unknown_fields() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let mut direct = text_annotation(
        Some(Object::string_literal("direct old")),
        valid_rect(),
        Some(rgb(0.1, 0.2, 0.3)),
    )
    .as_dict()
    .unwrap()
    .clone();
    direct.set("PrivateExtension", Object::string_literal("keep-direct"));
    set_annots(
        &mut document,
        1,
        Object::Array(vec![Object::Dictionary(direct)]),
    );

    let mut indirect = text_annotation(
        Some(text_string("indirect old")),
        valid_rect(),
        Some(rgb(0.4, 0.5, 0.6)),
    )
    .as_dict()
    .unwrap()
    .clone();
    indirect.set("PrivateExtension", Object::string_literal("keep-indirect"));
    let annotation_id = document.add_object(indirect);
    let annots_id = document.add_object(vec![Object::Reference(annotation_id)]);
    set_annots(&mut document, 2, Object::Reference(annots_id));

    let mut session = Session::open(&save(document)).unwrap();
    session
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "direct old".into(),
            contents: "direct new".into(),
        })
        .unwrap();
    session
        .apply(Edit::UpdateNote {
            page: 1,
            annotation_index: 0,
            expected_contents: "indirect old".into(),
            contents: "indirect new".into(),
        })
        .unwrap();

    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    assert_eq!(
        annotation_dictionary(&parsed, 1, 0)
            .get(b"PrivateExtension")
            .unwrap()
            .as_str()
            .unwrap(),
        b"keep-direct"
    );
    assert_eq!(
        annotation_dictionary(&parsed, 2, 0)
            .get(b"PrivateExtension")
            .unwrap()
            .as_str()
            .unwrap(),
        b"keep-indirect"
    );
    assert_eq!(session.notes(0).unwrap().notes[0].contents, "direct new");
    assert_eq!(session.notes(1).unwrap().notes[0].contents, "indirect new");
}

#[test]
fn update_note_copy_on_write_does_not_change_a_shared_annotation_or_annots_array() {
    let mut document = Document::load_mem(&fixture_pdf(2)).unwrap();
    let annotation = document.add_object(text_annotation(
        Some(Object::string_literal("shared old")),
        valid_rect(),
        Some(rgb(1.0, 0.0, 0.0)),
    ));
    let annots = document.add_object(vec![Object::Reference(annotation)]);
    set_annots(&mut document, 1, Object::Reference(annots));
    set_annots(&mut document, 2, Object::Reference(annots));

    let mut session = Session::open(&save(document)).unwrap();
    session
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "shared old".into(),
            contents: "page one only".into(),
        })
        .unwrap();
    assert_eq!(session.notes(0).unwrap().notes[0].contents, "page one only");
    assert_eq!(session.notes(1).unwrap().notes[0].contents, "shared old");
}

#[test]
fn update_note_rejects_stale_invalid_unsupported_malformed_and_oversized_inputs_atomically() {
    let mut stale = Session::blank(200.0, 120.0, 1).unwrap();
    stale
        .apply(Edit::AddNote {
            page: 0,
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 50.0,
            },
            contents: "current".into(),
            color: Color {
                r: 1.0,
                g: 0.8,
                b: 0.0,
            },
        })
        .unwrap();
    for edit in [
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "stale".into(),
            contents: "new".into(),
        },
        Edit::UpdateNote {
            page: 0,
            annotation_index: 1,
            expected_contents: "current".into(),
            contents: "new".into(),
        },
        Edit::UpdateNote {
            page: 1,
            annotation_index: 0,
            expected_contents: "current".into(),
            contents: "new".into(),
        },
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "current".into(),
            contents: " \n\t ".into(),
        },
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "current".into(),
            contents: "x".repeat(64 * 1024 + 1),
        },
    ] {
        assert_apply_unchanged(&mut stale, edit);
    }

    let unsupported = Object::Dictionary(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => valid_rect(),
    });
    let mut unsupported =
        Session::open(&document_with_annots(1, vec![unsupported].into())).unwrap();
    assert_apply_unchanged(
        &mut unsupported,
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: String::new(),
            contents: "new".into(),
        },
    );

    let malformed = text_annotation(Some(Object::Integer(7)), valid_rect(), None);
    let mut malformed = Session::open(&document_with_annots(1, vec![malformed].into())).unwrap();
    assert_apply_unchanged(
        &mut malformed,
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: String::new(),
            contents: "new".into(),
        },
    );

    let oversized_old = vec![
        text_annotation(
            Some(Object::String(
                "a".repeat(MIB / 2 + 1).into_bytes(),
                StringFormat::Literal,
            )),
            valid_rect(),
            None,
        ),
        text_annotation(
            Some(Object::String(
                "b".repeat(MIB / 2 + 1).into_bytes(),
                StringFormat::Literal,
            )),
            vec![60.into(), 10.into(), 90.into(), 40.into()].into(),
            None,
        ),
    ];
    let mut oversized_old = Session::open(&document_with_annots(1, oversized_old.into())).unwrap();
    assert_apply_unchanged(
        &mut oversized_old,
        Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "a".repeat(MIB / 2 + 1),
            contents: "new".into(),
        },
    );
}

#[test]
fn update_note_enforces_post_edit_total_and_accepts_exact_mib_and_64k_boundaries() {
    let annots = vec![
        text_annotation(
            Some(Object::String(
                "a".repeat(MIB - 1).into_bytes(),
                StringFormat::Literal,
            )),
            valid_rect(),
            None,
        ),
        text_annotation(
            None,
            vec![60.into(), 10.into(), 90.into(), 40.into()].into(),
            None,
        ),
    ];
    let mut boundary = Session::open(&document_with_annots(1, annots.into())).unwrap();
    boundary
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 1,
            expected_contents: String::new(),
            contents: "x".into(),
        })
        .unwrap();
    assert_eq!(
        boundary
            .notes(0)
            .unwrap()
            .notes
            .iter()
            .map(|note| note.contents.len())
            .sum::<usize>(),
        MIB
    );
    boundary.undo().unwrap();
    assert!(boundary.info().can_redo);
    assert_apply_unchanged(
        &mut boundary,
        Edit::UpdateNote {
            page: 0,
            annotation_index: 1,
            expected_contents: String::new(),
            contents: "xy".into(),
        },
    );
    assert!(boundary.info().can_redo);

    let mut max_text = Session::open(&document_with_annots(
        1,
        vec![text_annotation(None, valid_rect(), None)].into(),
    ))
    .unwrap();
    let contents = "z".repeat(64 * 1024);
    max_text
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: String::new(),
            contents: contents.clone(),
        })
        .unwrap();
    assert_eq!(max_text.notes(0).unwrap().notes[0].contents, contents);
}

#[test]
fn add_note_preserves_readable_annotation_count_and_content_postconditions() {
    let unsupported = || {
        Object::Dictionary(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        })
    };
    let add = |contents: &str| Edit::AddNote {
        page: 0,
        bounds: Rect {
            x: 100.0,
            y: 60.0,
            width: 20.0,
            height: 20.0,
        },
        contents: contents.into(),
        color: Color {
            r: 1.0,
            g: 0.8,
            b: 0.0,
        },
    };

    let mut exact_count = Session::open(&document_with_annots(
        1,
        (0..1023).map(|_| unsupported()).collect::<Vec<_>>().into(),
    ))
    .unwrap();
    exact_count.apply(add("count boundary")).unwrap();
    let exact = exact_count.notes(0).unwrap();
    assert_eq!(exact.unsupported_count, 1023);
    assert_eq!(exact.notes.len(), 1);

    let mut over_count = Session::open(&document_with_annots(
        1,
        (0..1024).map(|_| unsupported()).collect::<Vec<_>>().into(),
    ))
    .unwrap();
    over_count
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    over_count.undo().unwrap();
    assert!(over_count.info().can_redo);
    assert_apply_unchanged(&mut over_count, add("one too many"));
    assert!(over_count.info().can_redo);

    let almost_mib = text_annotation(
        Some(Object::String(
            "a".repeat(MIB - 1).into_bytes(),
            StringFormat::Literal,
        )),
        valid_rect(),
        None,
    );
    let mut exact_contents =
        Session::open(&document_with_annots(1, vec![almost_mib.clone()].into())).unwrap();
    exact_contents.apply(add("x")).unwrap();
    assert_eq!(
        exact_contents
            .notes(0)
            .unwrap()
            .notes
            .iter()
            .map(|note| note.contents.len())
            .sum::<usize>(),
        MIB
    );

    let mut over_contents =
        Session::open(&document_with_annots(1, vec![almost_mib].into())).unwrap();
    over_contents
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    over_contents.undo().unwrap();
    assert_apply_unchanged(&mut over_contents, add("xy"));
    assert!(over_contents.info().can_redo);

    let malformed = Object::Dictionary(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![10.into(), 10.into(), 10.into(), 20.into()],
    });
    let mut malformed = Session::open(&document_with_annots(1, vec![malformed].into())).unwrap();
    assert_apply_unchanged(
        &mut malformed,
        add("must not bypass malformed existing entry"),
    );
}

#[test]
fn add_note_uses_the_same_64_reference_limit_and_rejects_bad_array_entries() {
    let add = || Edit::AddNote {
        page: 0,
        bounds: Rect {
            x: 100.0,
            y: 60.0,
            width: 20.0,
            height: 20.0,
        },
        contents: "new note".into(),
        color: Color {
            r: 1.0,
            g: 0.8,
            b: 0.0,
        },
    };

    let mut at_limit = Session::open(&document_with_annots_reference_chain(64)).unwrap();
    at_limit.apply(add()).unwrap();
    assert_eq!(at_limit.notes(0).unwrap().notes.len(), 1);

    let mut too_deep = Session::open(&document_with_annots_reference_chain(65)).unwrap();
    assert_apply_unchanged(&mut too_deep, add());

    for annots in [
        vec![Object::Integer(7)].into(),
        vec![Object::Reference((9999, 0))].into(),
    ] {
        let mut malformed = Session::open(&document_with_annots(1, annots)).unwrap();
        assert_apply_unchanged(&mut malformed, add());
    }
}

#[test]
fn update_note_serde_shape_is_stable() {
    assert_eq!(
        serde_json::to_value(Edit::UpdateNote {
            page: 2,
            annotation_index: 4,
            expected_contents: "old".into(),
            contents: "new".into(),
        })
        .unwrap(),
        serde_json::json!({
            "kind": "update_note",
            "page": 2,
            "annotation_index": 4,
            "expected_contents": "old",
            "contents": "new",
        })
    );
}

#[test]
fn notes_reject_invalid_page_without_mutation() {
    let session = Session::open(&fixture_pdf(1)).unwrap();
    let before = session.bytes().unwrap();
    let error = session.notes(1).unwrap_err();
    assert!(error.to_string().contains("invalid page index 1"));
    assert_eq!(session.bytes().unwrap(), before);
}

#[test]
fn notes_reject_malformed_containers_references_and_dictionaries() {
    let cases = [
        (Object::Integer(1), "Annots"),
        (Object::Reference((9999, 0)), "object"),
        (vec![Object::Reference((9999, 0))].into(), "object"),
        (vec![Object::Integer(1)].into(), "dictionary"),
        (
            vec![Object::Dictionary(dictionary! { "Rect" => valid_rect() })].into(),
            "Subtype",
        ),
    ];
    for (annots, message) in cases {
        let session = Session::open(&document_with_annots(1, annots)).unwrap();
        let before = session.bytes().unwrap();
        let error = session.notes(0).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "expected {message:?}, got {error}"
        );
        assert_eq!(session.bytes().unwrap(), before);
    }
}

#[test]
fn notes_allow_64_actual_references_but_reject_65_and_cycles_atomically() {
    let accepted = Session::open(&document_with_annots_reference_chain(64)).unwrap();
    let accepted_bytes = accepted.bytes().unwrap();
    let accepted_info = accepted.info();
    assert_eq!(accepted.notes(0).unwrap(), NoteList::default());
    assert_eq!(accepted.bytes().unwrap(), accepted_bytes);
    assert_eq!(accepted.info(), accepted_info);

    let rejected = Session::open(&document_with_annots_reference_chain(65)).unwrap();
    let rejected_bytes = rejected.bytes().unwrap();
    let rejected_info = rejected.info();
    let error = rejected.notes(0).unwrap_err();
    assert!(error.to_string().contains("depth exceeded"), "{error}");
    assert_eq!(rejected.bytes().unwrap(), rejected_bytes);
    assert_eq!(rejected.info(), rejected_info);

    let cyclic = Session::open(&document_with_annots_reference_cycle()).unwrap();
    let cyclic_bytes = cyclic.bytes().unwrap();
    let cyclic_info = cyclic.info();
    let error = cyclic.notes(0).unwrap_err();
    assert!(error.to_string().contains("cycle"), "{error}");
    assert_eq!(cyclic.bytes().unwrap(), cyclic_bytes);
    assert_eq!(cyclic.info(), cyclic_info);
}

#[test]
fn notes_reject_malformed_text_rect_color_and_contents() {
    let annotations = [
        text_annotation(
            Some(Object::string_literal("bad rect")),
            vec![10.into(), 20.into(), 10.into(), 70.into()].into(),
            None,
        ),
        text_annotation(
            Some(Object::string_literal("outside")),
            vec![10.into(), 20.into(), 250.into(), 70.into()].into(),
            None,
        ),
        text_annotation(
            Some(Object::string_literal("bad color")),
            valid_rect(),
            Some(vec![1.into(), 0.into()].into()),
        ),
        text_annotation(
            Some(Object::string_literal("bad range")),
            valid_rect(),
            Some(rgb(1.1, 0.0, 0.0)),
        ),
        text_annotation(Some(Object::Integer(7)), valid_rect(), None),
        text_annotation(
            Some(Object::String(
                vec![0xfe, 0xff, 0xd8, 0x00],
                StringFormat::Hexadecimal,
            )),
            valid_rect(),
            None,
        ),
        text_annotation(
            Some(Object::String(
                vec![0xff, 0xfe, 0x00],
                StringFormat::Hexadecimal,
            )),
            valid_rect(),
            None,
        ),
    ];
    for annotation in annotations {
        let session = Session::open(&document_with_annots(1, vec![annotation].into())).unwrap();
        let before = session.bytes().unwrap();
        assert!(session.notes(0).is_err());
        assert_eq!(session.bytes().unwrap(), before);
    }
}

#[test]
fn notes_reject_malformed_rect_even_for_an_unsupported_subtype() {
    let unsupported = Object::Dictionary(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![10.into(), 20.into(), 10.into(), 70.into()],
    });
    let session = Session::open(&document_with_annots(1, vec![unsupported].into())).unwrap();
    assert!(session.notes(0).is_err());
}

#[test]
fn notes_enforce_entry_and_total_decoded_content_budgets() {
    let unsupported = || {
        Object::Dictionary(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        })
    };
    let session = Session::open(&document_with_annots(
        1,
        (0..1025).map(|_| unsupported()).collect::<Vec<_>>().into(),
    ))
    .unwrap();
    assert!(session.notes(0).unwrap_err().to_string().contains("count"));

    let first = "a".repeat(MIB / 2 + 1);
    let second = "b".repeat(MIB / 2 + 1);
    let annots = vec![
        text_annotation(
            Some(Object::String(first.into_bytes(), StringFormat::Literal)),
            valid_rect(),
            None,
        ),
        text_annotation(
            Some(Object::String(second.into_bytes(), StringFormat::Literal)),
            vec![60.into(), 10.into(), 90.into(), 40.into()].into(),
            None,
        ),
    ];
    let session = Session::open(&document_with_annots(1, annots.into())).unwrap();
    assert!(
        session
            .notes(0)
            .unwrap_err()
            .to_string()
            .contains("contents")
    );
}
