use lopdf::{Document, Object, ObjectId, dictionary, text_string};
use pdfmaster_core::{Edit, Session};

fn fixture(build: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    fixture_pages(2, build)
}

fn fixture_pages(count: usize, build: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    let bytes = Session::blank(200., 120., count).unwrap().bytes().unwrap();
    let mut doc = Document::load_mem(&bytes).unwrap();
    let pages = doc.get_pages().values().copied().collect::<Vec<_>>();
    build(&mut doc, &pages);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn set_annots(doc: &mut Document, page: ObjectId, rows: Vec<Object>) {
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", rows);
}

fn annotation(kind: &str) -> Object {
    dictionary! {"Subtype"=>Object::Name(kind.as_bytes().to_vec()), "Rect"=>vec![10.into(),20.into(),50.into(),60.into()]}.into()
}

#[test]
fn projects_all_annotation_types_without_reindexing_or_mutating_document() {
    let bytes = fixture(|doc, pages| {
        let mut rows = vec![
            annotation("Widget"),
            annotation("Link"),
            annotation("Popup"),
        ];
        for kind in [
            "Highlight",
            "Underline",
            "StrikeOut",
            "Ink",
            "FreeText",
            "Text",
            "Square",
            "Circle",
            "Line",
            "Stamp",
        ] {
            let mut row = annotation(kind).as_dict().unwrap().clone();
            row.set("Contents", text_string("中文 · résumé"));
            row.set("C", vec![1.into(), 0.into(), 0.into()]);
            row.set("M", Object::string_literal("D:20261027093000+08'00'"));
            rows.push(doc.add_object(row).into());
        }
        set_annots(doc, pages[0], rows);
        set_annots(doc, pages[1], vec![annotation("Text")]);
    });
    let session = Session::open(&bytes).unwrap();
    let before = session.info();
    let rows = session.annotations().unwrap();
    assert_eq!(rows.len(), 11);
    assert_eq!(rows[0].subtype, "Highlight");
    assert_eq!(rows[0].annotation_index, 3);
    assert_eq!(rows[9].subtype, "Stamp");
    assert_eq!(rows[10].page_index, 1);
    assert_eq!(rows[10].annotation_index, 0);
    assert_eq!(rows[0].contents, "中文 · résumé");
    assert_eq!(
        rows[0].modified.as_deref(),
        Some("2026-10-27T09:30:00+08:00")
    );
    assert_eq!(rows[0].color.unwrap().r, 1.);
    assert!(rows[0].object_id.is_some());
    assert!(rows[10].object_id.is_none());
    assert_eq!(session.info(), before);
    assert_eq!(session.bytes().unwrap(), bytes);
}

#[test]
fn page_reordering_preserves_pdf_object_identity_and_undo_restores_projection() {
    let bytes = fixture(|doc, pages| {
        let id = doc.add_object(annotation("Text"));
        let alias = doc.add_object(Object::Reference(id));
        set_annots(doc, pages[1], vec![alias.into()]);
    });
    let mut session = Session::open(&bytes).unwrap();
    let original = session.annotations().unwrap();
    session.apply(Edit::MovePage { from: 1, to: 0 }).unwrap();
    let moved = session.annotations().unwrap();
    assert_eq!(moved[0].page_index, 0);
    assert_eq!(moved[0].page_id, original[0].page_id);
    assert_eq!(moved[0].object_id, original[0].object_id);
    session.undo().unwrap();
    assert_eq!(session.annotations().unwrap(), original);
}

#[test]
fn decodes_utf16le_and_device_colors_without_assuming_every_annotation_is_rgb() {
    let bytes = fixture(|doc, pages| {
        let mut rows = Vec::new();
        for color in [
            vec![0.25.into()],
            vec![1.into(), 0.into(), 0.into(), 0.into()],
            vec![],
        ] {
            let mut row = annotation("FreeText").as_dict().unwrap().clone();
            let encoded = [
                vec![255, 254],
                "便笺🌏".encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat();
            row.set(
                "Contents",
                Object::String(encoded, lopdf::StringFormat::Hexadecimal),
            );
            row.set("C", color);
            rows.push(row.into());
        }
        set_annots(doc, pages[0], rows);
    });
    let rows = Session::open(&bytes).unwrap().annotations().unwrap();
    assert_eq!(rows[0].contents, "便笺🌏");
    let gray = rows[0].color.unwrap();
    assert_eq!((gray.r, gray.g, gray.b), (0.25, 0.25, 0.25));
    let cyan = rows[1].color.unwrap();
    assert_eq!((cyan.r, cyan.g, cyan.b), (0., 1., 1.));
    assert!(rows[2].color.is_none());
}

#[test]
fn dates_validate_calendar_defaults_and_preserve_unspecified_timezone() {
    for (input, expected) in [
        ("D:2024", Some("2024-01-01T00:00:00")),
        ("D:20240229Z", Some("2024-02-29T00:00:00Z")),
        ("D:20261027093000-05'30'", Some("2026-10-27T09:30:00-05:30")),
        ("D:20230229", None),
        ("D:202613", None),
        ("D:20261027093061", None),
        ("D:20261027093000+25'00'", None),
        ("D:20261027093000Zjunk", None),
    ] {
        let bytes = fixture(|doc, pages| {
            let mut row = annotation("Text").as_dict().unwrap().clone();
            row.set("M", Object::string_literal(input));
            set_annots(doc, pages[0], vec![row.into()]);
        });
        assert_eq!(
            Session::open(&bytes).unwrap().annotations().unwrap()[0]
                .modified
                .as_deref(),
            expected,
            "{input}"
        );
    }
}

#[test]
fn invalid_bounds_disable_rectangle_navigation_but_do_not_drop_a_real_annotation() {
    let bytes = fixture(|doc, pages| {
        let mut row = annotation("Stamp").as_dict().unwrap().clone();
        row.set("Rect", vec![0.into(), 0.into(), 0.into(), 0.into()]);
        set_annots(doc, pages[0], vec![row.into()]);
    });
    let rows = Session::open(&bytes).unwrap().annotations().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].bounds.is_none());
}

#[test]
fn cyclic_and_malformed_annotations_are_errors_not_a_false_empty_list() {
    for cycle in [false, true] {
        let bytes = fixture(|doc, pages| {
            let row = if cycle {
                let id = doc.new_object_id();
                doc.set_object(id, Object::Reference(id));
                Object::Reference(id)
            } else {
                42.into()
            };
            set_annots(doc, pages[0], vec![row]);
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.annotations().is_err());
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn entry_and_text_budgets_fail_explicitly_including_filtered_entries() {
    for text in [false, true] {
        let bytes = fixture(|doc, pages| {
            let rows = if text {
                let mut row = annotation("Text").as_dict().unwrap().clone();
                row.set("Contents", text_string(&"x".repeat(65537)));
                vec![row.into()]
            } else {
                vec![annotation("Link"); 4097]
            };
            set_annots(doc, pages[0], rows);
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.annotations().is_err());
        assert!(!session.info().dirty);
    }
}

#[test]
fn global_projection_budgets_count_shared_objects_each_time_they_are_displayed() {
    for (pages, per_page, kind, text_len) in [
        (5, 4096, "Link", 0),
        (2, 2049, "Text", 0),
        (2, 17, "Text", 65536),
    ] {
        let bytes = fixture_pages(pages, |doc, pages| {
            let mut row = annotation(kind).as_dict().unwrap().clone();
            row.set("Contents", text_string(&"x".repeat(text_len)));
            let id = doc.add_object(row);
            for page in pages {
                set_annots(doc, *page, vec![Object::Reference(id); per_page]);
            }
        });
        let session = Session::open(&bytes).unwrap();
        let before = session.info();
        assert!(matches!(
            session.annotations(),
            Err(pdfmaster_core::PdfMasterError::Budget(_))
        ));
        assert_eq!(session.info(), before);
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn cycles_in_projected_metadata_and_annotation_arrays_fail_without_mutation() {
    for key in ["Annots", "Subtype", "Contents", "C", "M", "Rect"] {
        let bytes = fixture(|doc, pages| {
            let cycle = doc.new_object_id();
            doc.set_object(cycle, Object::Reference(cycle));
            let mut row = annotation("Text").as_dict().unwrap().clone();
            if key == "Annots" {
                doc.get_object_mut(pages[0])
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set(key, cycle);
            } else {
                row.set(key, cycle);
                set_annots(doc, pages[0], vec![row.into()]);
            }
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.annotations().is_err(), "{key}");
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn created_annotations_have_real_modification_dates_and_redo_preserves_the_same_date() {
    use pdfmaster_core::{Color, LineEnding, Point, Rect, ShapeKind};
    let bounds = Rect {
        x: 20.,
        y: 20.,
        width: 60.,
        height: 40.,
    };
    let color = Color {
        r: 1.,
        g: 0.,
        b: 0.,
    };
    for edit in [
        Edit::AddNote {
            page: 0,
            bounds,
            contents: "new note".into(),
            color,
        },
        Edit::AddShape {
            page: 0,
            bounds,
            shape: ShapeKind::Rectangle,
            stroke: color,
            fill: None,
            line_width: 2.,
        },
        Edit::AddLine {
            page: 0,
            start: Point { x: 20., y: 20. },
            end: Point { x: 80., y: 60. },
            endings: [LineEnding::None; 2],
            stroke: color,
            fill: None,
            line_width: 2.,
        },
        Edit::AddInk {
            page: 0,
            strokes: vec![vec![Point { x: 20., y: 20. }, Point { x: 80., y: 60. }]],
            color,
            line_width: 2.,
            smoothed: false,
        },
    ] {
        let mut session = Session::blank(200., 120., 1).unwrap();
        let before = session.bytes().unwrap();
        session.apply(edit).unwrap();
        let created = session.annotations().unwrap();
        assert!(
            created[0]
                .modified
                .as_ref()
                .is_some_and(|d| d.ends_with('Z')),
            "missing real /M on {}",
            created[0].subtype
        );
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), before);
        session.redo().unwrap();
        assert_eq!(session.annotations().unwrap(), created);
    }
}

#[test]
fn updating_note_or_shape_refreshes_old_modification_date_without_touching_other_rows() {
    let bytes = fixture(|doc, pages| {
        let mut rows = Vec::new();
        for kind in ["Text", "Square", "Text"] {
            let mut row = annotation(kind).as_dict().unwrap().clone();
            row.set("Contents", text_string("old"));
            row.set("M", Object::string_literal("D:20000101000000Z"));
            row.set("C", vec![1.into(), 0.into(), 0.into()]);
            rows.push(doc.add_object(row).into());
        }
        set_annots(doc, pages[0], rows);
    });
    let mut session = Session::open(&bytes).unwrap();
    session
        .apply(Edit::UpdateNote {
            page: 0,
            annotation_index: 0,
            expected_contents: "old".into(),
            contents: "changed".into(),
        })
        .unwrap();
    let note_updated = session.annotations().unwrap();
    assert_ne!(
        note_updated[0].modified.as_deref(),
        Some("2000-01-01T00:00:00Z")
    );
    assert_eq!(
        note_updated[2].modified.as_deref(),
        Some("2000-01-01T00:00:00Z")
    );
    let expected = session.shapes(0).unwrap().shapes[0].clone();
    session
        .apply(Edit::UpdateShape {
            page: 0,
            bounds: expected.bounds,
            stroke: expected.stroke,
            fill: expected.fill,
            line_width: 2.,
            expected,
        })
        .unwrap();
    let shape_updated = session.annotations().unwrap();
    assert_ne!(
        shape_updated[1].modified.as_deref(),
        Some("2000-01-01T00:00:00Z")
    );
    assert_eq!(
        shape_updated[2].modified.as_deref(),
        Some("2000-01-01T00:00:00Z")
    );
    session.undo().unwrap();
    assert_eq!(session.annotations().unwrap(), note_updated);
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes);
}
