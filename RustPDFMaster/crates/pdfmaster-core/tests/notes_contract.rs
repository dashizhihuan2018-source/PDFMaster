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

fn assert_update_unchanged(session: &mut Session, edit: Edit) {
    let before_bytes = session.bytes().unwrap();
    let before_info = session.info();
    assert!(session.apply(edit).is_err());
    assert_eq!(session.bytes().unwrap(), before_bytes);
    assert_eq!(session.info(), before_info);
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
        assert_update_unchanged(&mut stale, edit);
    }

    let unsupported = Object::Dictionary(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => valid_rect(),
    });
    let mut unsupported =
        Session::open(&document_with_annots(1, vec![unsupported].into())).unwrap();
    assert_update_unchanged(
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
    assert_update_unchanged(
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
    assert_update_unchanged(
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
