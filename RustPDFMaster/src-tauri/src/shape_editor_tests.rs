use super::*;
use crate::shape_editor::CanvasShapeTarget;
use pdfmaster_core::{CanvasShapeDraft, CanvasShapeGeometry, Color, Rect};

#[test]
fn complete_canvas_projection_is_flattened_readonly_and_preserves_active_owner() {
    let (mut w, _dir, path) = workspace();
    w.session
        .as_mut()
        .unwrap()
        .create_canvas_shape(0, &rect())
        .unwrap();
    rewrite(&mut w, |doc, page, shape| {
        let mut rows = vec![lopdf::Object::Reference(shape)];
        for kind in ["Widget", "Popup", "Link"] {
            let mut d = lopdf::Dictionary::new();
            d.set("Subtype", lopdf::Object::Name(kind.as_bytes().to_vec()));
            d.set("Rect", vec![10.into(), 10.into(), 20.into(), 20.into()]);
            rows.push(doc.add_object(d).into());
        }
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", rows);
    });
    let t = target(&w);
    let owner = begin(&mut w, t);
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let journal = std::fs::read(&path).ok();
    let value =
        serde_json::to_value(w.canvas_shape_projection_for_document(1, 0).unwrap()).unwrap();
    assert_eq!(value["hit_rows"].as_array().unwrap().len(), 4);
    assert_eq!(value["annotation_count"], 4);
    assert_eq!(value["unsupported_count"], 3);
    assert_eq!(value["annotations"].as_array().unwrap().len(), 1);
    assert_eq!(value["revision"].as_array().unwrap().len(), 32);
    assert!(value.get("list").is_none());
    for i in 0..4 {
        assert_eq!(value["hit_rows"][i]["annotation_index"], i);
    }
    assert!(w.canvas_shape_projection_for_document(99, 0).is_err());
    assert!(w.canvas_shape_projection_for_document(1, 99).is_err());
    assert_eq!(w.shape_editor.as_ref().unwrap().id, owner.id);
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).ok(), journal);
}

pub(super) fn rect() -> CanvasShapeDraft {
    CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Rectangle {
            bounds: Rect {
                x: 20.,
                y: 20.,
                width: 100.,
                height: 60.,
            },
        },
        stroke: Color {
            r: 1.,
            g: 0.,
            b: 0.,
        },
        fill: None,
        line_width: 2.,
        opacity: 1.,
    }
}

#[test]
fn shape_alpha_new_terminal_retry_is_exact_and_changed_opacity_preserves_receipt() {
    let (mut w, _dir, path) = workspace();
    let mut draft = rect();
    draft.opacity = 0.5;
    let t = CanvasShapeTarget::New {
        draft: draft.clone(),
        revision: w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .revision,
    };
    let o = begin(&mut w, t);
    let json = serde_json::to_value(&o).unwrap();
    assert_eq!(json["target"]["draft"]["opacity"], 0.5);
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    w.commit_shape_editor(o.id, &o.request, draft.clone())
        .unwrap();
    assert_eq!(
        w.session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations[0]
            .opacity,
        0.5
    );
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let journal = std::fs::read(&path).unwrap();
    w.commit_shape_editor(o.id, &o.request, draft.clone())
        .unwrap();
    draft.opacity = 0.;
    assert!(w.commit_shape_editor(o.id, &o.request, draft).is_err());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    let mut old = serde_json::to_value(rect()).unwrap();
    old.as_object_mut().unwrap().remove("opacity");
    let old: CanvasShapeDraft = serde_json::from_value(old).unwrap();
    assert_eq!(old.opacity, 1.);
}

fn exact_frame_line(bounds: Rect) -> CanvasShapeDraft {
    CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Line {
            start: pdfmaster_core::Point { x: 40., y: 60. },
            end: pdfmaster_core::Point { x: 140., y: 60. },
            endings: [pdfmaster_core::LineEnding::None; 2],
            dash_pattern: vec![],
            bounds: Some(bounds),
        },
        ..rect()
    }
}

#[test]
fn exact_line_frame_owner_retry_distinguishes_bounds_and_preserves_journal_recovery() {
    let (mut w, _dir, path) = workspace();
    let initial = Rect {
        x: 38.,
        y: 58.,
        width: 104.,
        height: 4.,
    };
    let final_frame = Rect {
        x: 35.,
        y: 55.,
        width: 110.,
        height: 10.,
    };
    w.session
        .as_mut()
        .unwrap()
        .create_canvas_shape(0, &exact_frame_line(initial))
        .unwrap();
    w.session.as_mut().unwrap().mark_saved();
    let original = w.session.as_ref().unwrap().bytes().unwrap();
    let target = w
        .session
        .as_ref()
        .unwrap()
        .canvas_shapes(0)
        .unwrap()
        .annotations
        .remove(0);
    let owner = begin(&mut w, CanvasShapeTarget::Existing { original: target });
    let json = serde_json::to_value(&owner).unwrap();
    assert_eq!(
        json["target"]["original"]["original"]["bounds"]["height"],
        4.
    );
    let draft = exact_frame_line(final_frame);
    w.commit_shape_editor(owner.id, &owner.request, draft.clone())
        .unwrap();
    let committed = w.session.as_ref().unwrap().bytes().unwrap();
    assert_ne!(committed, original);
    let info = w.session.as_ref().unwrap().info();
    let journal = std::fs::read(&path).unwrap();
    let recovered = recovery::Journal::new(path.clone())
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(recovered.bytes, committed);
    assert_eq!(
        Session::open(&recovered.bytes)
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations[0]
            .original
            .bounds,
        final_frame
    );
    w.commit_shape_editor(owner.id, &owner.request, draft)
        .unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), committed);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    let changed_bounds_only = exact_frame_line(Rect {
        height: 12.,
        ..final_frame
    });
    assert!(
        w.commit_shape_editor(owner.id, &owner.request, changed_bounds_only)
            .is_err()
    );
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), committed);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    w.cancel_shape_editor(owner.id, &owner.request).unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), committed);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    let reopened = Session::open(&committed).unwrap();
    assert_eq!(
        reopened.canvas_shapes(0).unwrap().annotations[0]
            .original
            .bounds,
        final_frame
    );
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
    assert_eq!(
        w.session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations[0]
            .original
            .bounds,
        initial
    );
    w.session.as_mut().unwrap().redo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), committed);
}

#[test]
fn exact_line_frame_fresh_noop_in_middle_history_preserves_journal_and_redo() {
    let (mut w, dir, path) = workspace();
    let bounds = Rect {
        x: 38.,
        y: 58.,
        width: 104.,
        height: 4.,
    };
    let draft = exact_frame_line(bounds);
    w.session
        .as_mut()
        .unwrap()
        .create_canvas_shape(0, &draft)
        .unwrap();
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    let future = w.session.as_ref().unwrap().bytes().unwrap();
    w.session.as_mut().unwrap().undo().unwrap();
    w.checkpoint().unwrap();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    assert!(info.can_undo && info.can_redo);
    let journal = std::fs::read(&path).unwrap();
    let target = w
        .session
        .as_ref()
        .unwrap()
        .canvas_shapes(0)
        .unwrap()
        .annotations
        .remove(0);
    assert_eq!(target.original.bounds, bounds);
    let owner = begin(&mut w, CanvasShapeTarget::Existing { original: target });
    let blocker = dir.path().join("exact-frame-blocked");
    std::fs::write(&blocker, b"blocked").unwrap();
    w.journal = Some(recovery::Journal::new(blocker.join("journal.json")));
    let view = w
        .commit_shape_editor(owner.id, &owner.request, draft.clone())
        .unwrap();
    assert!(view.warning.is_none());
    w.commit_shape_editor(owner.id, &owner.request, draft)
        .unwrap();
    w.cancel_shape_editor(owner.id, &owner.request).unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    assert_eq!(
        Session::open(&before)
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations[0]
            .original
            .bounds,
        bounds
    );
    w.session.as_mut().unwrap().redo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), future);
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
}

#[test]
fn shape_alpha_existing_zero_opaque_updates_and_middle_history_noops_are_real() {
    let (mut w, dir, path) = workspace();
    let mut draft = rect();
    draft.opacity = 0.5;
    w.session
        .as_mut()
        .unwrap()
        .create_canvas_shape(0, &draft)
        .unwrap();
    for (index, alpha) in [0., 1.].into_iter().enumerate() {
        let selected = w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .remove(0);
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let old_render = w.session.as_ref().unwrap().render_png(0, 1.).unwrap();
        let o = w
            .begin_canvas_shape_editor(
                &format!("shape-alpha-update-{index}"),
                1,
                0,
                CanvasShapeTarget::Existing { original: selected },
            )
            .unwrap();
        draft.opacity = alpha;
        w.commit_shape_editor(o.id, &o.request, draft.clone())
            .unwrap();
        assert_eq!(
            w.session
                .as_ref()
                .unwrap()
                .canvas_shapes(0)
                .unwrap()
                .annotations[0]
                .opacity,
            alpha
        );
        assert_ne!(
            w.session.as_ref().unwrap().render_png(0, 1.).unwrap(),
            old_render
        );
        let after = w.session.as_ref().unwrap().bytes().unwrap();
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        w.session.as_mut().unwrap().redo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), after);
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::Rotate {
                page: 1,
                degrees: 90,
            })
            .unwrap();
        w.session.as_mut().unwrap().undo().unwrap();
        w.checkpoint().unwrap();
        let selected = w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .remove(0);
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let journal = std::fs::read(&path).unwrap();
        let o = w
            .begin_canvas_shape_editor(
                &format!("shape-alpha-noop-{index}"),
                1,
                0,
                CanvasShapeTarget::Existing { original: selected },
            )
            .unwrap();
        let blocker = dir.path().join(format!("blocked-{index}"));
        std::fs::write(&blocker, b"blocked").unwrap();
        w.journal = Some(recovery::Journal::new(blocker.join("journal.json")));
        assert!(
            w.commit_shape_editor(o.id, &o.request, draft.clone())
                .unwrap()
                .warning
                .is_none()
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(std::fs::read(&path).unwrap(), journal);
        w.journal = Some(recovery::Journal::new(path.clone()));
    }
}

#[test]
fn shape_alpha_checkpoint_failure_retry_keeps_terminal_warning_and_single_undo() {
    let (mut w, dir, _) = workspace();
    let mut draft = rect();
    draft.opacity = 0.5;
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let t = CanvasShapeTarget::New {
        draft: draft.clone(),
        revision: w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .revision,
    };
    let o = begin(&mut w, t);
    let blocker = dir.path().join("blocked-alpha");
    std::fs::write(&blocker, b"blocked").unwrap();
    w.journal = Some(recovery::Journal::new(blocker.join("journal.json")));
    assert!(
        w.commit_shape_editor(o.id, &o.request, draft.clone())
            .unwrap()
            .warning
            .is_some()
    );
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    w.journal = None;
    assert!(
        w.commit_shape_editor(o.id, &o.request, draft.clone())
            .unwrap()
            .warning
            .is_some()
    );
    draft.opacity = 1.;
    assert!(w.commit_shape_editor(o.id, &o.request, draft).is_err());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
}

pub(super) fn workspace() -> (Workspace, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shape-draft.json");
    let mut session = Session::blank(300., 200., 2).unwrap();
    session.mark_saved();
    (
        Workspace {
            session: Some(session),
            active_document_id: 1,
            next_document_id: 1,
            document_epoch: 1,
            journal: Some(recovery::Journal::new(path.clone())),
            ..Default::default()
        },
        dir,
        path,
    )
}

pub(super) fn target(w: &Workspace) -> CanvasShapeTarget {
    CanvasShapeTarget::New {
        draft: rect(),
        revision: w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .revision,
    }
}

#[test]
fn valid_begin_is_read_only_and_view_restores_exact_owner() {
    let (mut w, _dir, path) = workspace();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let target = target(&w);
    let owner = w
        .begin_canvas_shape_editor("shape-new-0001", 1, 0, target)
        .unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.view().unwrap().shape_editor.unwrap(), owner);
    assert!(!path.exists());
    let json = serde_json::to_value(&owner).unwrap();
    assert_eq!(json["target"]["kind"], "new");
    let keys: std::collections::BTreeSet<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["id", "page", "request", "target"].into_iter().collect()
    );
}

#[test]
fn read_is_bound_to_current_document() {
    let (w, _dir, _) = workspace();
    assert_eq!(
        w.canvas_shapes_for_document(1, 0).unwrap(),
        w.session.as_ref().unwrap().canvas_shapes(0).unwrap()
    );
    assert!(w.canvas_shapes_for_document(2, 0).is_err());
    assert!(w.canvas_shapes_for_document(1, 2).is_err());
}

#[test]
fn begin_repeat_cancel_and_tombstones_are_exact_and_read_only() {
    let (mut w, _dir, path) = workspace();
    let t = target(&w);
    let owner = w
        .begin_canvas_shape_editor("shape-new-0001", 1, 0, t.clone())
        .unwrap();
    assert_eq!(
        w.begin_canvas_shape_editor(&owner.request, 1, 0, t.clone())
            .unwrap(),
        owner
    );
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    assert!(
        w.cancel_canvas_shape_request("shape-other-0002", 1)
            .is_err()
    );
    assert!(!w.seen_shape_requests.contains("shape-other-0002"));
    assert!(
        w.cancel_canvas_shape_request("shape-other-0002", 2)
            .is_err()
    );
    w.cancel_shape_editor(owner.id, &owner.request).unwrap();
    w.cancel_shape_editor(owner.id, &owner.request).unwrap();
    assert!(
        w.begin_canvas_shape_editor(&owner.request, 1, 0, t.clone())
            .is_err()
    );
    w.cancel_canvas_shape_request("shape-before-0002", 1)
        .unwrap();
    assert!(
        w.begin_canvas_shape_editor("shape-before-0002", 1, 0, t)
            .is_err()
    );
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert!(!w.session.as_ref().unwrap().info().can_undo);
    assert!(!path.exists());
}

fn assert_request_cancel_rejected_without_mutation(w: &mut Workspace, request: &str, doc: u32) {
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let id = w.next_editor_id;
    let seen = w.seen_shape_requests.clone();
    let owner = w.shape_editor.clone();
    assert!(w.cancel_canvas_shape_request(request, doc).is_err());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(w.next_editor_id, id);
    assert_eq!(w.seen_shape_requests, seen);
    assert_eq!(w.shape_editor, owner);
}

#[test]
fn finished_request_only_cancel_requires_current_terminal_receipt() {
    for outcome in 0..3 {
        let (mut w, _dir, path) = workspace();
        let t = if outcome == 1 {
            existing(&mut w)
        } else {
            target(&w)
        };
        let o = begin(&mut w, t);
        match outcome {
            0 => {
                w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
            }
            1 => {
                let prepared = prepare(&w, &o);
                w.apply_confirmed_shape_editor_delete(&prepared).unwrap();
            }
            _ => {
                w.cancel_shape_editor(o.id, &o.request).unwrap();
            }
        }
        let terminal = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let journal = std::fs::read(&path).ok();
        w.cancel_canvas_shape_request(&o.request, 1).unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), terminal);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(std::fs::read(&path).ok(), journal);
        assert_request_cancel_rejected_without_mutation(&mut w, &o.request, 2);
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::Rotate {
                page: 1,
                degrees: 90,
            })
            .unwrap();
        assert_request_cancel_rejected_without_mutation(&mut w, &o.request, 1);
        w.session.as_mut().unwrap().undo().unwrap();
        w.checkpoint().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), terminal);
        assert_request_cancel_rejected_without_mutation(&mut w, &o.request, 1);
    }
}

#[test]
fn old_request_cannot_cancel_after_checkpoint_epoch_or_newer_owner() {
    let (mut w, _dir, _) = workspace();
    w.replace_document(Session::blank(300., 200., 1).unwrap(), "other.pdf".into())
        .unwrap();
    w.activate_document(1).unwrap();
    let t = target(&w);
    let o = begin(&mut w, t);
    w.cancel_shape_editor(o.id, &o.request).unwrap();
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    w.activate_document(2).unwrap();
    w.activate_document(1).unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    assert_request_cancel_rejected_without_mutation(&mut w, &o.request, 1);
    let t = target(&w);
    let newer = w
        .begin_canvas_shape_editor("shape-newer-0002", 1, 0, t)
        .unwrap();
    assert_request_cancel_rejected_without_mutation(&mut w, &o.request, 1);
    assert_eq!(w.shape_editor.as_ref().unwrap(), &newer);
    w.cancel_shape_editor(newer.id, &newer.request).unwrap();
    assert_request_cancel_rejected_without_mutation(&mut w, &o.request, 1);
    w.cancel_canvas_shape_request(&newer.request, 1).unwrap();
}

#[test]
fn prebegin_request_cancel_repeat_is_bound_to_document_epoch_and_bytes() {
    let (mut w, _dir, path) = workspace();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    w.cancel_canvas_shape_request("shape-before-0002", 1)
        .unwrap();
    w.cancel_canvas_shape_request("shape-before-0002", 1)
        .unwrap();
    assert_eq!(w.seen_shape_requests.len(), 1);
    assert!(!path.exists());
    assert_request_cancel_rejected_without_mutation(&mut w, "shape-before-0002", 2);
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    assert_request_cancel_rejected_without_mutation(&mut w, "shape-before-0002", 1);
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    w.checkpoint().unwrap();
    assert_request_cancel_rejected_without_mutation(&mut w, "shape-before-0002", 1);
    let t = target(&w);
    assert!(
        w.begin_canvas_shape_editor("shape-before-0002", 1, 0, t)
            .is_err()
    );
}

#[test]
fn prebegin_receipt_only_retains_last_request_and_reserves_all_tombstones() {
    let (mut w, _dir, _) = workspace();
    w.cancel_canvas_shape_request("shape-first-0001", 1)
        .unwrap();
    w.cancel_canvas_shape_request("shape-second-0002", 1)
        .unwrap();
    assert_request_cancel_rejected_without_mutation(&mut w, "shape-first-0001", 1);
    w.cancel_canvas_shape_request("shape-second-0002", 1)
        .unwrap();
    assert_eq!(w.seen_shape_requests.len(), 2);
    for i in 2..4096 {
        w.cancel_canvas_shape_request(&format!("shape-budget-{i:04}"), 1)
            .unwrap();
    }
    assert_eq!(w.seen_shape_requests.len(), 4096);
    w.cancel_canvas_shape_request("shape-budget-4095", 1)
        .unwrap();
    assert_request_cancel_rejected_without_mutation(&mut w, "shape-budget-4096", 1);
    assert_eq!(w.seen_shape_requests.len(), 4096);
    let t = target(&w);
    assert!(
        w.begin_canvas_shape_editor("shape-first-0001", 1, 0, t)
            .is_err()
    );
}

#[test]
fn invalid_begin_never_consumes_counter_tombstone_or_journal() {
    let (mut w, _dir, path) = workspace();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    for request in ["short", &"a".repeat(65), "形状-0001", "shape/new-0001"] {
        let t = target(&w);
        assert!(w.begin_canvas_shape_editor(request, 1, 0, t).is_err());
    }
    for (doc, page) in [(2, 0), (1, 2)] {
        let t = target(&w);
        assert!(
            w.begin_canvas_shape_editor("shape-new-0001", doc, page, t)
                .is_err()
        );
    }
    let mut stale = target(&w);
    if let CanvasShapeTarget::New { revision, .. } = &mut stale {
        revision[0] ^= 1;
    }
    assert!(
        w.begin_canvas_shape_editor("shape-new-0001", 1, 0, stale)
            .is_err()
    );
    let mut invalid = target(&w);
    if let CanvasShapeTarget::New { draft, .. } = &mut invalid {
        draft.line_width = f32::NAN;
    }
    assert!(
        w.begin_canvas_shape_editor("shape-new-0001", 1, 0, invalid)
            .is_err()
    );
    assert_eq!(w.next_editor_id, 0);
    assert!(w.seen_shape_requests.is_empty());
    assert!(w.shape_editor.is_none());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert!(!path.exists());
}

#[test]
fn request_capacity_and_counter_exhaustion_do_not_steal_exact_retry() {
    let (mut w, _dir, _) = workspace();
    for i in 0..4095 {
        w.cancel_canvas_shape_request(&format!("shape-cancel-{i:04}"), 1)
            .unwrap();
    }
    let t = target(&w);
    let owner = w
        .begin_canvas_shape_editor("shape-new-0001", 1, 0, t.clone())
        .unwrap();
    assert_eq!(w.seen_shape_requests.len(), 4096);
    assert_eq!(
        w.begin_canvas_shape_editor(&owner.request, 1, 0, t.clone())
            .unwrap(),
        owner
    );
    w.cancel_shape_editor(owner.id, &owner.request).unwrap();
    assert!(
        w.begin_canvas_shape_editor("shape-over-4097", 1, 0, t)
            .is_err()
    );
    let (mut w, _dir2, _) = workspace();
    w.next_editor_id = u32::MAX;
    let t = target(&w);
    assert!(
        w.begin_canvas_shape_editor("shape-new-0001", 1, 0, t)
            .is_err()
    );
    assert!(w.seen_shape_requests.is_empty());
}

pub(super) fn existing(w: &mut Workspace) -> CanvasShapeTarget {
    w.session
        .as_mut()
        .unwrap()
        .create_canvas_shape(0, &rect())
        .unwrap();
    w.session.as_mut().unwrap().mark_saved();
    CanvasShapeTarget::Existing {
        original: w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .remove(0),
    }
}

#[test]
fn existing_begin_checks_whole_core_selection_and_page() {
    let (mut w, _dir, _) = workspace();
    let t = existing(&mut w);
    for part in 0..6 {
        let mut forged = t.clone();
        if let CanvasShapeTarget::Existing { original } = &mut forged {
            match part {
                0 => original.flags ^= 128,
                1 => original.page_id.0 += 1,
                2 => original.object_id = Some((99999, 0)),
                3 => original.revision[0] ^= 1,
                4 => original.original.line_width += 1.,
                _ => original.page_index = 1,
            }
        }
        assert!(
            w.begin_canvas_shape_editor("shape-new-0001", 1, 0, forged)
                .is_err()
        );
        assert!(w.seen_shape_requests.is_empty());
    }
    let owner = w
        .begin_canvas_shape_editor("shape-new-0001", 1, 0, t)
        .unwrap();
    assert_eq!(
        serde_json::to_value(owner).unwrap()["target"]["kind"],
        "existing"
    );
}

pub(super) fn begin(w: &mut Workspace, t: CanvasShapeTarget) -> shape_editor::ShapeEditor {
    w.begin_canvas_shape_editor("shape-new-0001", 1, 0, t)
        .unwrap()
}

#[test]
fn create_commit_exact_retry_and_cross_operation_are_one_transaction() {
    let (mut w, _dir, path) = workspace();
    let original = w.session.as_ref().unwrap().bytes().unwrap();
    let t = target(&w);
    let o = begin(&mut w, t);
    let view = w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
    assert!(view.shape_editor.is_none());
    let committed = w.session.as_ref().unwrap().bytes().unwrap();
    let journal = std::fs::read(&path).unwrap();
    w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), committed);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    let mut different = rect();
    different.line_width = 3.;
    assert!(w.commit_shape_editor(o.id, &o.request, different).is_err());
    w.cancel_shape_editor(o.id, &o.request).unwrap();
    assert_eq!(
        Session::open(&committed)
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .len(),
        1
    );
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
    assert!(!w.session.as_ref().unwrap().info().can_undo);
    w.session.as_mut().unwrap().redo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), committed);
}

#[test]
fn changed_existing_rect_ellipse_and_line_preserve_exact_undo_redo_reopen() {
    use pdfmaster_core::{LineEnding, Point};
    let geometries = [
        rect().geometry,
        CanvasShapeGeometry::Ellipse {
            bounds: Rect {
                x: 30.,
                y: 40.,
                width: 50.,
                height: 60.,
            },
        },
        CanvasShapeGeometry::Line {
            start: Point { x: 150., y: 80. },
            end: Point { x: 40., y: 120. },
            endings: [LineEnding::None, LineEnding::OpenArrow],
            dash_pattern: vec![3., 2.],
            bounds: None,
        },
    ];
    for geometry in geometries {
        let (mut w, _dir, _) = workspace();
        let mut draft = rect();
        draft.geometry = geometry;
        w.session
            .as_mut()
            .unwrap()
            .create_canvas_shape(0, &draft)
            .unwrap();
        w.session.as_mut().unwrap().mark_saved();
        let original = w.session.as_ref().unwrap().bytes().unwrap();
        let selected = w
            .session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .remove(0);
        let o = begin(&mut w, CanvasShapeTarget::Existing { original: selected });
        draft.line_width = 4.;
        draft.fill = Some(Color {
            r: 0.,
            g: 1.,
            b: 0.,
        });
        match &mut draft.geometry {
            CanvasShapeGeometry::Rectangle { bounds } | CanvasShapeGeometry::Ellipse { bounds } => {
                bounds.x += 5.;
                bounds.height += 3.;
            }
            CanvasShapeGeometry::Line {
                start,
                end,
                endings,
                dash_pattern,
                ..
            } => {
                start.x -= 10.;
                end.y -= 10.;
                *endings = [LineEnding::ClosedArrow, LineEnding::None];
                *dash_pattern = vec![5., 1.];
            }
        }
        w.commit_shape_editor(o.id, &o.request, draft).unwrap();
        let after = w.session.as_ref().unwrap().bytes().unwrap();
        assert_ne!(after, original);
        assert_eq!(
            Session::open(&after)
                .unwrap()
                .canvas_shapes(0)
                .unwrap()
                .annotations
                .len(),
            1
        );
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
        w.session.as_mut().unwrap().redo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), after);
    }
}

#[test]
fn no_op_in_middle_of_history_preserves_redo_and_does_not_checkpoint() {
    let (mut w, dir, path) = workspace();
    existing(&mut w);
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    w.session.as_mut().unwrap().undo().unwrap();
    w.checkpoint().unwrap();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    assert!(info.can_redo);
    let journal = std::fs::read(&path).unwrap();
    let selected = w
        .session
        .as_ref()
        .unwrap()
        .canvas_shapes(0)
        .unwrap()
        .annotations
        .remove(0);
    let o = begin(&mut w, CanvasShapeTarget::Existing { original: selected });
    // A real checkpoint at this destination fails; the no-op must not try it.
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"blocked").unwrap();
    w.journal = Some(recovery::Journal::new(blocker.join("journal.json")));
    let view = w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
    assert!(view.warning.is_none());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    w.session.as_mut().unwrap().redo().unwrap();
    assert!(w.checkpoint().unwrap().warning.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), journal);
}

#[test]
fn payload_failures_retain_full_owner_and_two_sided_history() {
    let (mut w, _dir, path) = workspace();
    existing(&mut w);
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    w.session.as_mut().unwrap().undo().unwrap();
    w.checkpoint().unwrap();
    let t = target(&w);
    let o = begin(&mut w, t);
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let journal = std::fs::read(&path).unwrap();
    let mut invalids = Vec::new();
    let mut d = rect();
    d.line_width = f32::NAN;
    invalids.push(d);
    let mut d = rect();
    d.stroke.r = 2.;
    invalids.push(d);
    let mut d = rect();
    d.geometry = CanvasShapeGeometry::Rectangle {
        bounds: Rect {
            x: 290.,
            y: 20.,
            width: 50.,
            height: 30.,
        },
    };
    invalids.push(d);
    let mut d = rect();
    d.geometry = CanvasShapeGeometry::Ellipse {
        bounds: Rect {
            x: 20.,
            y: 20.,
            width: 100.,
            height: 60.,
        },
    };
    invalids.push(d);
    for d in invalids {
        assert!(w.commit_shape_editor(o.id, &o.request, d).is_err());
    }
    assert!(w.commit_shape_editor(o.id + 1, &o.request, rect()).is_err());
    assert!(
        w.commit_shape_editor(o.id, "shape-forged-0001", rect())
            .is_err()
    );
    assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
}

#[test]
fn cancelled_owner_cannot_commit_and_new_owner_cannot_be_cleared_by_old_retry() {
    let (mut w, _dir, _) = workspace();
    let t = target(&w);
    let o = begin(&mut w, t);
    w.cancel_shape_editor(o.id, &o.request).unwrap();
    assert!(w.commit_shape_editor(o.id, &o.request, rect()).is_err());
    let t = target(&w);
    let newer = w
        .begin_canvas_shape_editor("shape-newer-0002", 1, 0, t)
        .unwrap();
    assert!(w.cancel_shape_editor(o.id, &o.request).is_err());
    assert!(w.commit_shape_editor(o.id, &o.request, rect()).is_err());
    assert_eq!(w.shape_editor.as_ref().unwrap(), &newer);
}

#[test]
fn later_workspace_checkpoint_cannot_revive_terminal_even_after_undo() {
    let (mut w, _dir, _) = workspace();
    let t = target(&w);
    let o = begin(&mut w, t);
    w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
    let terminal = w.session.as_ref().unwrap().bytes().unwrap();
    apply_single_edit(
        &mut w,
        Edit::Rotate {
            page: 1,
            degrees: 90,
        },
    )
    .unwrap();
    w.session.as_mut().unwrap().undo().unwrap();
    w.checkpoint().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), terminal);
    assert!(w.commit_shape_editor(o.id, &o.request, rect()).is_err());
    assert!(w.cancel_shape_editor(o.id, &o.request).is_err());
}

#[test]
fn checkpoint_failure_commits_once_and_exact_retry_keeps_sticky_warning() {
    let (mut w, dir, _) = workspace();
    let original = w.session.as_ref().unwrap().bytes().unwrap();
    let t = target(&w);
    let o = begin(&mut w, t);
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"blocked").unwrap();
    w.journal = Some(recovery::Journal::new(blocker.join("journal.json")));
    assert!(
        w.commit_shape_editor(o.id, &o.request, rect())
            .unwrap()
            .warning
            .is_some()
    );
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    w.journal = None;
    assert!(
        w.commit_shape_editor(o.id, &o.request, rect())
            .unwrap()
            .warning
            .is_some()
    );
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
}

#[test]
fn shape_owner_blocks_export() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(w.begin_export().is_err());
}
#[test]
fn shape_owner_counts_as_unsaved_on_clean_pdf() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(!w.session.as_ref().unwrap().is_dirty());
    assert!(w.has_unsaved_work());
}
#[test]
fn shape_owner_blocks_note_begin() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(
        w.begin_canvas_note_editor(
            "note-new-0001",
            1,
            0,
            note_editor::CanvasNoteTarget::New {
                x: 30.,
                y: 30.,
                page_width: 300.,
                page_height: 200.,
                rotation: 0
            }
        )
        .is_err()
    );
}
#[test]
fn shape_owner_blocks_ink_begin() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(w.begin_ink("ink-new-0001", 0, rect().stroke, 2.).is_err());
}
#[test]
fn shape_owner_blocks_text_begin() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(
        w.begin_canvas_free_text_editor("text-new-0001", 1, 0, text_target())
            .is_err()
    );
}
#[test]
fn shape_owner_blocks_tab_activate() {
    let (mut w, _d, _) = workspace();
    w.replace_document(Session::blank(300., 200., 1).unwrap(), "other.pdf".into())
        .unwrap();
    w.activate_document(1).unwrap();
    let t = target(&w);
    begin(&mut w, t);
    assert!(w.activate_document(2).is_err());
}
#[test]
fn shape_owner_blocks_prepare_close() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(w.prepare_close(1).is_err());
}
#[test]
fn shape_owner_blocks_install() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(
        w.replace_document(Session::blank(300., 200., 1).unwrap(), "other.pdf".into())
            .is_err()
    );
}
#[test]
fn shape_owner_blocks_insert() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(w.begin_insertion(1).is_err());
}
#[test]
fn shape_owner_blocks_generic_edit() {
    let (mut w, _d, _) = workspace();
    let t = target(&w);
    begin(&mut w, t);
    assert!(
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 1,
                degrees: 90
            }
        )
        .is_err()
    );
}

pub(super) fn text_target() -> free_text_editor::CanvasFreeTextTarget {
    free_text_editor::CanvasFreeTextTarget::New {
        bounds: Rect {
            x: 20.,
            y: 20.,
            width: 100.,
            height: 60.,
        },
        style: free_text_editor::FreeTextAppearance {
            font: free_text_editor::FreeTextFont::Sans,
            font_size: 12.,
            color: rect().stroke,
            alignment: pdfmaster_core::TextAlignment::Left,
            line_height: 1.2,
        },
    }
}

#[test]
fn text_background_rejects_other_shape_owner() {
    let (mut w, _dir, _) = workspace();
    let t = target(&w);
    let o = begin(&mut w, t);
    w.shape_editor = None;
    let text = w
        .begin_canvas_free_text_editor("text-new-0001", 1, 0, text_target())
        .unwrap();
    w.shape_editor = Some(o);
    let bytes = w.session.as_ref().unwrap().bytes().unwrap();
    assert!(
        w.free_text_editor_background(text.id, &text.request, 1.)
            .is_err()
    );
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
}

fn prepare(w: &Workspace, o: &shape_editor::ShapeEditor) -> shape_editor::PreparedShapeDelete {
    match w.prepare_shape_editor_delete(o.id, &o.request).unwrap() {
        shape_editor::ShapeDeletePreparation::Confirm(p) => p,
        shape_editor::ShapeDeletePreparation::Finished(_) => panic!("not yet deleted"),
    }
}

#[test]
fn prepare_retains_owner_and_confirmed_delete_is_one_transaction_and_replay() {
    let (mut w, _dir, path) = workspace();
    let t = existing(&mut w);
    w.checkpoint().unwrap();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let journal = std::fs::read(&path).unwrap();
    let o = begin(&mut w, t);
    let prepared = prepare(&w, &o);
    let duplicate = prepare(&w, &o);
    assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    // Dropping a prepared value models a dismissed native dialog.
    drop(prepared);
    assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
    let view = w.apply_confirmed_shape_editor_delete(&duplicate).unwrap();
    assert!(view.shape_editor.is_none());
    assert!(
        w.session
            .as_ref()
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .is_empty()
    );
    let terminal = w.session.as_ref().unwrap().bytes().unwrap();
    let journal = std::fs::read(&path).unwrap();
    assert!(matches!(
        w.prepare_shape_editor_delete(o.id, &o.request).unwrap(),
        shape_editor::ShapeDeletePreparation::Finished(_)
    ));
    w.apply_confirmed_shape_editor_delete(&duplicate).unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), terminal);
    assert_eq!(std::fs::read(&path).unwrap(), journal);
    assert!(w.commit_shape_editor(o.id, &o.request, rect()).is_err());
    w.cancel_shape_editor(o.id, &o.request).unwrap();
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    w.session.as_mut().unwrap().redo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), terminal);
    assert!(
        Session::open(&terminal)
            .unwrap()
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .is_empty()
    );
}

#[test]
fn delete_new_cancelled_committed_or_newer_owner_is_never_allowed() {
    let (mut w, _dir, _) = workspace();
    let t = target(&w);
    let o = begin(&mut w, t);
    assert!(w.prepare_shape_editor_delete(o.id, &o.request).is_err());
    w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
    assert!(w.prepare_shape_editor_delete(o.id, &o.request).is_err());
    let selected = w
        .session
        .as_ref()
        .unwrap()
        .canvas_shapes(0)
        .unwrap()
        .annotations
        .remove(0);
    let o = w
        .begin_canvas_shape_editor(
            "shape-delete-0002",
            1,
            0,
            CanvasShapeTarget::Existing { original: selected },
        )
        .unwrap();
    let prepared = prepare(&w, &o);
    w.cancel_shape_editor(o.id, &o.request).unwrap();
    assert!(w.prepare_shape_editor_delete(o.id, &o.request).is_err());
    assert!(w.apply_confirmed_shape_editor_delete(&prepared).is_err());
    let t = target(&w);
    let newer = w
        .begin_canvas_shape_editor("shape-newer-0003", 1, 0, t)
        .unwrap();
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    assert!(w.apply_confirmed_shape_editor_delete(&prepared).is_err());
    assert_eq!(w.shape_editor.as_ref().unwrap(), &newer);
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
}

#[test]
fn delete_await_rechecks_document_epoch_and_bytes_and_preserves_failed_owner() {
    for conflict in 0..3 {
        let (mut w, _dir, path) = workspace();
        let t = existing(&mut w);
        w.checkpoint().unwrap();
        let o = begin(&mut w, t);
        let prepared = prepare(&w, &o);
        match conflict {
            0 => w.active_document_id += 1,
            1 => w.document_epoch += 1,
            _ => {
                w.session
                    .as_mut()
                    .unwrap()
                    .apply(Edit::Rotate {
                        page: 1,
                        degrees: 90,
                    })
                    .unwrap();
            }
        }
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let journal = std::fs::read(&path).unwrap();
        assert!(w.apply_confirmed_shape_editor_delete(&prepared).is_err());
        assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(std::fs::read(&path).unwrap(), journal);
    }
}

#[test]
fn generic_shape_note_and_annotation_delete_cannot_bypass_active_shape_owner() {
    let (mut w, _dir, _) = workspace();
    let t = existing(&mut w);
    let o = begin(&mut w, t);
    let original = w
        .session
        .as_ref()
        .unwrap()
        .canvas_shapes(0)
        .unwrap()
        .annotations
        .remove(0)
        .original;
    let row = w.session.as_ref().unwrap().annotations().unwrap().remove(0);
    assert!(prepare_shape_delete(&w, 0, original).is_err());
    assert!(prepare_annotation_delete(&w, row).is_err());
    assert!(prepare_note_delete(&w, 0, 0, "").is_err());
    assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
}

fn install_other_owner(w: &mut Workspace, kind: usize) {
    match kind {
        0 => {
            w.begin_canvas_note_editor(
                "note-new-0001",
                1,
                0,
                note_editor::CanvasNoteTarget::New {
                    x: 30.,
                    y: 30.,
                    page_width: 300.,
                    page_height: 200.,
                    rotation: 0,
                },
            )
            .unwrap();
        }
        1 => {
            w.begin_canvas_free_text_editor("text-new-0001", 1, 0, text_target())
                .unwrap();
        }
        2 => {
            w.begin_ink("ink-new-0001", 0, rect().stroke, 2.).unwrap();
        }
        _ => {
            w.begin_export().unwrap();
        }
    }
}

fn all_shape_operations_reject_other_owner(kind: usize) {
    let mut rejected = Vec::new();
    for operation in 0..9 {
        let (mut w, _dir, path) = workspace();
        let t = existing(&mut w);
        w.checkpoint().unwrap();
        let o = begin(&mut w, t.clone());
        let prepared = prepare(&w, &o);
        let saved = if operation >= 7 {
            w.cancel_shape_editor(o.id, &o.request).unwrap();
            None
        } else {
            w.shape_editor.take()
        };
        install_other_owner(&mut w, kind);
        w.shape_editor = saved;
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let journal = std::fs::read(&path).unwrap();
        let err = match operation {
            0 => w.begin_canvas_shape_editor(&o.request, 1, 0, t).is_err(),
            1 => w.cancel_canvas_shape_request(&o.request, 1).is_err(),
            2 => w.cancel_shape_editor(o.id, &o.request).is_err(),
            3 => w.commit_shape_editor(o.id, &o.request, rect()).is_err(),
            4 => w.shape_editor_background(o.id, &o.request, 1.).is_err(),
            5 => w.prepare_shape_editor_delete(o.id, &o.request).is_err(),
            6 => w.apply_confirmed_shape_editor_delete(&prepared).is_err(),
            7 => w.cancel_shape_editor(o.id, &o.request).is_err(),
            _ => w
                .begin_canvas_shape_editor("shape-newer-0002", 1, 0, t)
                .is_err(),
        };
        rejected.push(err);
        if err {
            assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
            assert_eq!(w.session.as_ref().unwrap().info(), info);
            assert_eq!(std::fs::read(&path).unwrap(), journal);
            if operation < 7 {
                assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
            }
        }
    }
    assert_eq!(
        rejected,
        vec![true; 9],
        "other owner {kind}; operation order: begin, request cancel, cancel, commit, background, prepare, apply, terminal cancel, new begin"
    );
}

#[test]
fn shapes_reject_note_owner_in_every_entrypoint() {
    all_shape_operations_reject_other_owner(0);
}
#[test]
fn shapes_reject_text_owner_in_every_entrypoint() {
    all_shape_operations_reject_other_owner(1);
}
#[test]
fn shapes_reject_ink_owner_in_every_entrypoint() {
    all_shape_operations_reject_other_owner(2);
}
#[test]
fn shapes_reject_export_owner_in_every_entrypoint() {
    all_shape_operations_reject_other_owner(3);
}

fn rewrite(w: &mut Workspace, edit: impl FnOnce(&mut lopdf::Document, (u32, u16), (u32, u16))) {
    let mut doc = lopdf::Document::load_mem(&w.session.as_ref().unwrap().bytes().unwrap()).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let annotation = doc
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
    edit(&mut doc, page, annotation);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    w.session = Some(Session::open(&bytes).unwrap());
}

#[test]
fn real_locked_readonly_styles_and_annotation_budget_reject_admission_without_consumption() {
    for case in 0..6 {
        let (mut w, _dir, path) = workspace();
        existing(&mut w);
        rewrite(&mut w, |doc, page, annotation| match case {
            0..=2 => doc
                .get_object_mut(annotation)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("F", [64, 128, 512][case]),
            3 => doc
                .get_object_mut(annotation)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("StructParent", 0),
            4 => doc
                .get_object_mut(annotation)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("IRT", annotation),
            _ => doc
                .get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", vec![lopdf::Object::Reference(annotation); 1025]),
        });
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        if case < 5 {
            let selected = w
                .session
                .as_ref()
                .unwrap()
                .canvas_shapes(0)
                .unwrap()
                .annotations
                .remove(0);
            assert!(
                w.begin_canvas_shape_editor(
                    "shape-new-0001",
                    1,
                    0,
                    CanvasShapeTarget::Existing { original: selected }
                )
                .is_err()
            );
        } else {
            // Structural source corruption: no revision can be projected.
            assert!(w.session.as_ref().unwrap().canvas_shapes(0).is_err());
            let revision = [0; 32];
            assert!(
                w.begin_canvas_shape_editor(
                    "shape-new-0001",
                    1,
                    0,
                    CanvasShapeTarget::New {
                        draft: rect(),
                        revision
                    }
                )
                .is_err()
            );
        }
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(w.next_editor_id, 0);
        assert!(w.seen_shape_requests.is_empty());
        assert!(w.shape_editor.is_none());
        assert!(!path.exists());
    }
}

#[test]
fn last_admissible_annotation_budget_rejects_valid_new_target_preflight() {
    let (mut w, _dir, path) = workspace();
    existing(&mut w);
    rewrite(&mut w, |doc, page, annotation| {
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![lopdf::Object::Reference(annotation); 1024]);
    });
    let list = w.session.as_ref().unwrap().canvas_shapes(0).unwrap();
    assert_eq!(list.annotation_count, 1024);
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let error = w
        .begin_canvas_shape_editor(
            "shape-budget-1025",
            1,
            0,
            CanvasShapeTarget::New {
                draft: rect(),
                revision: list.revision,
            },
        )
        .unwrap_err();
    assert!(
        error.contains("annotation count"),
        "actual budget error: {error}"
    );
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(w.next_editor_id, 0);
    assert!(w.seen_shape_requests.is_empty());
    assert!(w.shape_editor.is_none());
    assert!(!path.exists());
}

#[test]
fn active_identity_guards_begin_cancel_commit_and_delete_with_complete_valid_payloads() {
    for mismatch in 0..5 {
        for operation in 0..5 {
            // Begin and request cancellation do not carry an editor id.
            if mismatch == 0 && (operation == 0 || operation == 4) {
                continue;
            }
            let (mut w, _dir, path) = workspace();
            let t = existing(&mut w);
            w.checkpoint().unwrap();
            let o = begin(&mut w, t.clone());
            match mismatch {
                2 => w.active_document_id += 1,
                3 => w.document_epoch += 1,
                4 => {
                    w.session
                        .as_mut()
                        .unwrap()
                        .apply(Edit::Rotate {
                            page: 1,
                            degrees: 90,
                        })
                        .unwrap();
                }
                _ => {}
            }
            let before = w.session.as_ref().unwrap().bytes().unwrap();
            let info = w.session.as_ref().unwrap().info();
            let journal = std::fs::read(&path).unwrap();
            let id = if mismatch == 0 { o.id + 1 } else { o.id };
            let request = if mismatch == 1 {
                "shape-forged-0002"
            } else {
                &o.request
            };
            let failed = match operation {
                0 => w.begin_canvas_shape_editor(request, 1, 0, t).is_err(),
                1 => w.cancel_shape_editor(id, request).is_err(),
                2 => w.commit_shape_editor(id, request, rect()).is_err(),
                3 => w.prepare_shape_editor_delete(id, request).is_err(),
                _ => w.cancel_canvas_shape_request(request, 1).is_err(),
            };
            assert!(failed, "mismatch {mismatch}, operation {operation}");
            assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
            assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
            assert_eq!(w.session.as_ref().unwrap().info(), info);
            assert_eq!(std::fs::read(&path).unwrap(), journal);
        }
    }
}

#[test]
fn terminal_identity_is_invalid_after_real_document_reopen_and_tab_switch() {
    for reopen in [false, true] {
        let (mut w, _dir, _) = workspace();
        if !reopen {
            w.replace_document(Session::blank(300., 200., 1).unwrap(), "second.pdf".into())
                .unwrap();
            w.activate_document(1).unwrap();
        }
        let t = target(&w);
        let o = begin(&mut w, t);
        w.commit_shape_editor(o.id, &o.request, rect()).unwrap();
        let terminal = w.session.as_ref().unwrap().bytes().unwrap();
        if reopen {
            w.replace_document(Session::open(&terminal).unwrap(), "reopen.pdf".into())
                .unwrap();
        } else {
            w.activate_document(2).unwrap();
            w.activate_document(1).unwrap();
        }
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), terminal);
        assert!(w.commit_shape_editor(o.id, &o.request, rect()).is_err());
        assert!(w.cancel_shape_editor(o.id, &o.request).is_err());
    }
}

#[test]
fn delete_checkpoint_failure_is_sticky_one_undo_and_exact_replay_safe() {
    let (mut w, dir, _) = workspace();
    let t = existing(&mut w);
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let o = begin(&mut w, t);
    let prepared = prepare(&w, &o);
    let blocker = dir.path().join("not-directory");
    std::fs::write(&blocker, b"blocked").unwrap();
    w.journal = Some(recovery::Journal::new(blocker.join("journal.json")));
    assert!(
        w.apply_confirmed_shape_editor_delete(&prepared)
            .unwrap()
            .warning
            .is_some()
    );
    assert!(matches!(
        w.prepare_shape_editor_delete(o.id, &o.request).unwrap(),
        shape_editor::ShapeDeletePreparation::Finished(_)
    ));
    assert!(
        w.apply_confirmed_shape_editor_delete(&prepared)
            .unwrap()
            .warning
            .is_some()
    );
    w.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
}
