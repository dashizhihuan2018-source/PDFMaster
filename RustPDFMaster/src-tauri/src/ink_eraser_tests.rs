use super::*;
use lopdf::dictionary;

fn workspace(kinds: &[&str]) -> Workspace {
    let session = Session::blank(300., 200., 2).unwrap();
    let mut doc = lopdf::Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page = doc.get_pages()[&1];
    let mut annotations = vec![];
    for (index, kind) in kinds.iter().enumerate() {
        let mut d = lopdf::dictionary! {
            "Type" => "Annot", "Subtype" => *kind,
            "Rect" => vec![20.into(),20.into(),80.into(),80.into()],
            "Contents" => lopdf::Object::string_literal(format!("stroke {index}")),
        };
        if *kind == "Ink" {
            d.set(
                "InkList",
                vec![lopdf::Object::Array(vec![
                    20.into(),
                    20.into(),
                    80.into(),
                    80.into(),
                ])],
            );
        }
        annotations.push(doc.add_object(d).into());
    }
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", annotations);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    Workspace {
        session: Some(Session::open(&bytes).unwrap()),
        active_document_id: 1,
        document_epoch: 1,
        ..Default::default()
    }
}

fn erase(w: &mut Workspace, id: u32) -> Reply<Option<DocumentView>> {
    w.erase_ink(id, 50., 50., 4., 4.)
}

fn rewrite(
    w: &mut Workspace,
    edit: impl FnOnce(&mut lopdf::Document, lopdf::ObjectId, lopdf::ObjectId),
) {
    let mut doc = lopdf::Document::load_mem(&w.session.as_ref().unwrap().bytes().unwrap()).unwrap();
    let page = doc.get_pages()[&1];
    let ink = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    edit(&mut doc, page, ink);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    w.session = Some(Session::open(&bytes).unwrap());
}

#[test]
fn ink_eraser_real_checkpoint_preserves_source_tabs_and_readonly_prepare_does_not_block_close() {
    let mut w = workspace(&["Ink", "Ink"]);
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("original.pdf");
    let path = dir.path().join("draft.json");
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    std::fs::write(&source, &before).unwrap();
    w.source_documents.insert(1, source.clone());
    w.journal = Some(recovery::Journal::new(path.clone()));
    let other = Session::blank(123., 234., 1).unwrap();
    let other_bytes = other.bytes().unwrap();
    w.inactive_documents.push(tabs::Parked {
        id: 2,
        session: other,
        name: "keep".into(),
        restored: false,
    });
    let c = w.prepare_ink_eraser(1, 0).unwrap();
    assert!(w.prepare_close(1).is_ok());
    assert!(!path.exists());
    assert!(!w.session.as_ref().unwrap().info().can_undo);
    let view = erase(&mut w, c.id).unwrap().unwrap();
    assert!(view.warning.is_none());
    assert!(view.info.dirty);
    assert_eq!(w.active_document_id, 1);
    assert_eq!(
        w.session.as_ref().unwrap().info().pages[1],
        Session::open(&before).unwrap().info().pages[1]
    );
    assert_eq!(
        w.inactive_documents[0].session.bytes().unwrap(),
        other_bytes
    );
    assert_eq!(std::fs::read(source).unwrap(), before);
    let recovered = recovery::Journal::new(path).load_many().unwrap();
    assert_eq!(recovered.len(), 1); // Clean inactive tabs are not recovery drafts.
    assert_eq!(
        Session::open(
            &recovered
                .iter()
                .find(|d| d.bytes != other_bytes)
                .unwrap()
                .bytes
        )
        .unwrap()
        .annotations()
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn ink_eraser_unavailable_bounds_skip_but_malformed_budget_relationship_fail_explicitly() {
    let mut w = workspace(&["Ink", "Text"]);
    rewrite(&mut w, |doc, page, _| {
        let id = doc
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[1]
            .as_reference()
            .unwrap();
        doc.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .remove(b"Rect");
    });
    let c = w.prepare_ink_eraser(1, 0).unwrap();
    assert!(erase(&mut w, c.id).unwrap().is_some());
    for case in ["tagged", "reply", "budget", "malformed"] {
        let mut w = workspace(&["Ink"]);
        rewrite(&mut w, |doc, _, ink| {
            let d = doc.get_object_mut(ink).unwrap().as_dict_mut().unwrap();
            match case {
                "tagged" => {
                    d.set("StructParent", 1);
                }
                "reply" => {
                    d.set("IRT", ink);
                }
                "budget" => {
                    d.set("Contents", lopdf::Object::string_literal("x".repeat(65537)));
                }
                _ => {
                    d.set("Subtype", lopdf::Object::string_literal("invalid"));
                }
            }
        });
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        match w.prepare_ink_eraser(1, 0) {
            Ok(c) => assert!(erase(&mut w, c.id).is_err(), "{case}"),
            Err(_) => assert_eq!(case, "malformed"),
        }
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(!w.session.as_ref().unwrap().info().can_undo);
    }
}

#[test]
fn ink_eraser_topmost_actual_delete_undo_redo_reopen_and_warning() {
    let mut w = workspace(&["Ink", "Ink"]);
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let c = w.prepare_ink_eraser(1, 0).unwrap();
    assert_eq!(
        serde_json::to_value(&c).unwrap(),
        serde_json::json!({"id":1,"document_id":1,"page":0})
    );
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    let view = erase(&mut w, c.id).unwrap().unwrap();
    assert!(view.warning.is_some());
    let rows = w.session.as_ref().unwrap().annotations().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].contents, "stroke 0");
    let deleted = w.session.as_ref().unwrap().bytes().unwrap();
    assert_eq!(
        Session::open(&deleted)
            .unwrap()
            .annotations()
            .unwrap()
            .len(),
        1
    );
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("erased.pdf");
    std::fs::write(&saved, &deleted).unwrap();
    assert_eq!(
        Session::open(&std::fs::read(saved).unwrap())
            .unwrap()
            .annotations()
            .unwrap()[0]
            .contents,
        "stroke 0"
    );
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert!(w.session.as_mut().unwrap().undo().is_err());
    w.session.as_mut().unwrap().redo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), deleted);
    assert!(erase(&mut w, c.id).is_err());
}

#[test]
fn ink_eraser_nonink_occludes_link_skips_and_miss_consumes() {
    for kind in ["Text", "Widget", "Popup", "Unknown"] {
        let mut w = workspace(&["Ink", kind]);
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let c = w.prepare_ink_eraser(1, 0).unwrap();
        assert!(erase(&mut w, c.id).unwrap().is_none(), "{kind}");
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(erase(&mut w, c.id).is_err());
    }
    let mut w = workspace(&["Ink", "Link"]);
    let c = w.prepare_ink_eraser(1, 0).unwrap();
    assert!(erase(&mut w, c.id).unwrap().is_some());
    let mut w = workspace(&["Ink"]);
    let c = w.prepare_ink_eraser(1, 0).unwrap();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    assert!(w.erase_ink(c.id, 150., 150., 4., 4.).unwrap().is_none());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert!(erase(&mut w, c.id).is_err());
}

#[test]
fn ink_eraser_stale_capabilities_guards_aba_and_invalid_input_are_atomic() {
    let mut w = workspace(&["Ink", "Ink"]);
    let first = w.prepare_ink_eraser(1, 0).unwrap();
    let second = w.prepare_ink_eraser(1, 0).unwrap();
    assert!(erase(&mut w, first.id).is_err());
    assert!(erase(&mut w, second.id).unwrap().is_some());
    for change in 0..4 {
        let mut w = workspace(&["Ink"]);
        let c = w.prepare_ink_eraser(1, 0).unwrap();
        match change {
            0 => w.active_document_id += 1,
            1 => w.document_epoch += 1,
            2 => {
                w.session
                    .as_mut()
                    .unwrap()
                    .apply(Edit::RotatePages {
                        pages: vec![0],
                        degrees: 90,
                    })
                    .unwrap();
            }
            _ => {
                w.session
                    .as_mut()
                    .unwrap()
                    .apply(Edit::RotatePages {
                        pages: vec![0],
                        degrees: 90,
                    })
                    .unwrap();
                w.checkpoint().unwrap();
                w.session.as_mut().unwrap().undo().unwrap();
                w.checkpoint().unwrap();
            }
        }
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(erase(&mut w, c.id).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }
    for (x, y, tx, ty) in [
        (f32::NAN, 50., 4., 4.),
        (-1., 50., 4., 4.),
        (301., 50., 4., 4.),
        (50., 201., 4., 4.),
        (50., 50., -1., 4.),
        (50., 50., 4., 201.),
        (50., 50., f32::INFINITY, 4.),
    ] {
        let mut w = workspace(&["Ink"]);
        let c = w.prepare_ink_eraser(1, 0).unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(w.erase_ink(c.id, x, y, tx, ty).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }
    let mut w = workspace(&["Ink"]);
    assert!(w.prepare_ink_eraser(99, 0).is_err());
    assert!(w.prepare_ink_eraser(1, 99).is_err());
    w.export_reservation = Some(1);
    assert!(w.prepare_ink_eraser(1, 0).is_err());
    w.export_reservation = None;
    let c = w.prepare_ink_eraser(1, 0).unwrap();
    w.export_reservation = Some(2);
    assert!(erase(&mut w, c.id).is_err());
    w.export_reservation = None;
    w.begin_ink(
        "erase-guard",
        0,
        pdfmaster_core::Color {
            r: 0.,
            g: 0.,
            b: 0.,
        },
        2.,
    )
    .unwrap();
    assert!(w.prepare_ink_eraser(1, 0).is_err());
    assert!(erase(&mut w, c.id).is_err());
}

#[test]
fn ink_eraser_does_not_replace_any_live_inline_editor() {
    for kind in ["note", "text", "shape"] {
        let mut w = workspace(&["Ink"]);
        let c = w.prepare_ink_eraser(1, 0).unwrap();
        match kind {
            "note" => {
                w.begin_canvas_note_editor(
                    "eraser-note-guard",
                    1,
                    0,
                    note_editor::CanvasNoteTarget::New {
                        x: 10.,
                        y: 10.,
                        page_width: 300.,
                        page_height: 200.,
                        rotation: 0,
                    },
                )
                .unwrap();
            }
            "text" => {
                w.begin_canvas_free_text_editor(
                    "eraser-text-guard",
                    1,
                    0,
                    free_text_editor::CanvasFreeTextTarget::New {
                        bounds: pdfmaster_core::Rect {
                            x: 10.,
                            y: 10.,
                            width: 100.,
                            height: 50.,
                        },
                        style: free_text_editor::FreeTextAppearance {
                            font: free_text_editor::FreeTextFont::Sans,
                            font_size: 12.,
                            color: pdfmaster_core::Color {
                                r: 0.,
                                g: 0.,
                                b: 0.,
                            },
                            alignment: pdfmaster_core::TextAlignment::Left,
                            line_height: 1.2,
                        },
                    },
                )
                .unwrap();
            }
            _ => {
                let revision = w
                    .session
                    .as_ref()
                    .unwrap()
                    .canvas_shapes(0)
                    .unwrap()
                    .revision;
                w.begin_canvas_shape_editor(
                    "eraser-shape-guard",
                    1,
                    0,
                    shape_editor::CanvasShapeTarget::New {
                        draft: shape_editor_tests::rect(),
                        revision,
                    },
                )
                .unwrap();
            }
        }
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(w.prepare_ink_eraser(1, 0).is_err());
        assert!(erase(&mut w, c.id).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(
            w.note_editor.is_some() || w.free_text_editor.is_some() || w.shape_editor.is_some()
        );
    }
}
