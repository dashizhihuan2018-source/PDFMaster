use pdfmaster_drawing::{Point, display_point};
use pdfmaster_ui::canvas_text::*;
use serde_json::json;
fn style() -> Appearance {
    Appearance {
        font: Font::Sans,
        font_size: 18.,
        color: Color {
            r: 0.,
            g: 0.,
            b: 0.,
        },
        alignment: Alignment::Left,
        line_height: 1.3,
    }
}
fn rect() -> Rect {
    Rect {
        x: 30.,
        y: 100.,
        width: 80.,
        height: 50.,
    }
}
fn owner() -> Owner {
    Owner {
        id: 17,
        request: "0123456789abcdef0123456789abcdef".into(),
        page: 0,
        target: Target::New {
            bounds: rect(),
            style: style(),
        },
        appearance_style: Some(style()),
    }
}
#[test]
fn full_wire_keeps_sibling_appearance_and_annotation_identity() {
    let expected_sibling_style = serde_json::to_value(style()).unwrap();
    let original_revision = [19_u8; 32];
    let fixture=json!({"id":17,"request":"0123456789abcdef0123456789abcdef","page":2,"appearance_style":expected_sibling_style,"target":{"kind":"existing","original":{
        "page_index":2,"annotation_index":4,"page_id":[9,3],"object_id":[18,2],"bounds":rect(),"contents":"原文 🦀","style":{"font_resource":[70,49],"font_size":11.,"color":{"r":0.3,"g":0.2,"b":0.1},"alignment":"right"},"flags":128,"revision":original_revision
    }}}).to_string();
    let wire = serde_json::from_str::<Owner>(&fixture).unwrap();
    let encoded = serde_json::to_value(&wire).unwrap();
    assert_eq!(encoded["appearance_style"], expected_sibling_style);
    assert_eq!(
        encoded["target"]["original"]["revision"],
        json!(original_revision)
    );
    assert_eq!(encoded["target"]["original"]["object_id"], json!([18, 2]));
}

#[test]
fn text_plane_maps_three_pdf_corners_and_inverse_for_all_rotations() {
    let bounds = rect();
    let frame = Frame {
        left: 13.,
        top: 29.,
        width: 455.,
        height: 731.,
    };
    for rotation in [0, 90, 180, 270] {
        let matrix = text_matrix(bounds, frame, 300., 500., rotation).unwrap();
        for (local, pdf) in [
            (
                Point { x: 0., y: 0. },
                Point {
                    x: bounds.x,
                    y: bounds.y + bounds.height,
                },
            ),
            (
                Point {
                    x: bounds.width,
                    y: 0.,
                },
                Point {
                    x: bounds.x + bounds.width,
                    y: bounds.y + bounds.height,
                },
            ),
            (
                Point {
                    x: 0.,
                    y: bounds.height,
                },
                Point {
                    x: bounds.x,
                    y: bounds.y,
                },
            ),
        ] {
            let display = display_point(pdf, 300., 500., rotation);
            let (w, h) = if rotation % 180 == 0 {
                (300., 500.)
            } else {
                (500., 300.)
            };
            let expected = Point {
                x: frame.left + display.x / w * frame.width,
                y: frame.top + display.y / h * frame.height,
            };
            let actual = Point {
                x: matrix[0] * local.x + matrix[2] * local.y + matrix[4],
                y: matrix[1] * local.x + matrix[3] * local.y + matrix[5],
            };
            assert!(
                (expected.x - actual.x).abs() < 0.001 && (expected.y - actual.y).abs() < 0.001,
                "rotation {rotation}: {expected:?} != {actual:?}"
            );
        }
        let delta = Point { x: 11., y: -7. };
        let screen = Point {
            x: matrix[0] * delta.x + matrix[2] * delta.y,
            y: matrix[1] * delta.x + matrix[3] * delta.y,
        };
        let back = local_delta(matrix, screen).unwrap();
        assert!((back.x - delta.x).abs() < 0.001 && (back.y - delta.y).abs() < 0.001);
    }
}

#[test]
fn move_clamps_four_edges_without_changing_size_and_eight_handles_keep_opposite_edges() {
    for (dx, dy, x, y) in [
        (-500., -500., 0., 450.),
        (500., 500., 220., 0.),
        (0., 500., 30., 0.),
        (0., -500., 30., 450.),
    ] {
        let moved = drag_rect(rect(), Point { x: dx, y: dy }, Handle::Move, 300., 500.).unwrap();
        assert_eq!(
            moved,
            Rect {
                x,
                y,
                width: 80.,
                height: 50.
            }
        );
    }
    for handle in [
        Handle::Nw,
        Handle::N,
        Handle::Ne,
        Handle::E,
        Handle::Se,
        Handle::S,
        Handle::Sw,
        Handle::W,
    ] {
        let r = drag_rect(rect(), Point { x: 20., y: 10. }, handle, 300., 500.).unwrap();
        assert_ne!(r, rect(), "{handle:?}");
        assert!(
            r.width > 0.
                && r.height > 0.
                && r.x >= 0.
                && r.y >= 0.
                && r.x + r.width <= 300.
                && r.y + r.height <= 500.
        );
        if matches!(handle, Handle::Nw | Handle::W | Handle::Sw) {
            assert_eq!(r.x + r.width, 110.);
        } else {
            assert_eq!(r.x, 30.);
        }
        if matches!(handle, Handle::Nw | Handle::N | Handle::Ne) {
            assert_eq!(r.y, 100.);
        } else if matches!(handle, Handle::Sw | Handle::S | Handle::Se) {
            assert_eq!(r.y + r.height, 150.);
        } else {
            assert_eq!(r.y, 100.);
        }
    }
}
#[test]
fn geometry_rejects_nonfinite_and_out_of_page_before_transform_or_drag() {
    for v in [f32::NAN, f32::INFINITY, -1., 0.] {
        let r = Rect { width: v, ..rect() };
        assert!(!valid_rect(r, 300., 500.));
        assert!(
            text_matrix(
                r,
                Frame {
                    left: 0.,
                    top: 0.,
                    width: 455.,
                    height: 731.
                },
                300.,
                500.,
                0
            )
            .is_err()
        );
        assert!(drag_rect(r, Point { x: 1., y: 1. }, Handle::Move, 300., 500.).is_none());
    }
    assert!(drag_rect(rect(), Point { x: f32::NAN, y: 1. }, Handle::E, 300., 500.).is_none());
    assert!(local_delta([1., 0., 0., 1., 0., 0.], Point { x: f32::NAN, y: 1. }).is_none());
    assert!(!valid_rect(Rect { x: 290., ..rect() }, 300., 500.));
    assert!(!valid_rect(rect(), f32::NAN, 500.));
}
#[test]
fn fractional_page_resize_is_positive_and_cannot_panic_or_flip() {
    let tiny = Rect {
        x: 0.1,
        y: 0.1,
        width: 0.2,
        height: 0.2,
    };
    for handle in [
        Handle::Nw,
        Handle::N,
        Handle::Ne,
        Handle::E,
        Handle::Se,
        Handle::S,
        Handle::Sw,
        Handle::W,
    ] {
        for delta in [Point { x: -100., y: -100. }, Point { x: 100., y: 100. }] {
            let changed = drag_rect(tiny, delta, handle, 0.75, 0.6).unwrap();
            assert!(valid_rect(changed, 0.75, 0.6), "{handle:?}: {changed:?}");
        }
    }
}
fn imported(flags: u32, appearance: Option<Appearance>) -> Owner {
    let mut o = owner();
    o.appearance_style = appearance;
    o.target = Target::Existing {
        original: Annotation {
            page_index: 0,
            annotation_index: 4,
            page_id: (9, 3),
            object_id: Some((18, 2)),
            bounds: rect(),
            contents: "原文".into(),
            style: ImportedStyle {
                font_resource: b"F1".to_vec(),
                font_size: 11.,
                color: Color {
                    r: 0.3,
                    g: 0.2,
                    b: 0.1,
                },
                alignment: Alignment::Right,
            },
            flags,
            revision: [19; 32],
        },
    };
    o
}
#[test]
fn imported_locks_64_128_512_and_unknown_style_preserve_original() {
    for flags in [64, 128, 512, 64 | 128 | 512] {
        let d = Draft::new(imported(flags, Some(style())));
        assert_eq!(
            d.body_locked(),
            flags & (64 | 512) != 0,
            "body flags {flags}"
        );
        assert_eq!(
            d.geometry_locked(),
            flags & (64 | 128) != 0,
            "geometry flags {flags}"
        );
        assert!(d.validate(300., 500.).is_ok());
        let mut changed = d.clone();
        changed.contents.push('!');
        assert_eq!(
            changed.validate(300., 500.).is_err(),
            flags & (64 | 512) != 0
        );
        let mut moved = d.clone();
        moved.bounds.x += 2.;
        assert_eq!(moved.validate(300., 500.).is_err(), flags & (64 | 128) != 0);
    }
    let mut unknown = Draft::new(imported(0, None));
    unknown.bounds.x += 5.;
    assert!(unknown.validate(300., 500.).is_ok());
    assert_eq!(unknown.payload().style, None);
    unknown.bounds.width += 5.;
    assert!(unknown.validate(300., 500.).is_err());
    unknown.style = Some(style());
    assert!(unknown.validate(300., 500.).is_ok());
}
#[test]
fn utf8_budget_keeps_overflow_body_and_recovers_after_one_chinese_character_deleted() {
    let mut d = Draft::new(owner());
    d.contents = "汉".repeat(21_846);
    assert!(d.validate(300., 500.).is_err());
    assert_eq!(d.contents.len(), 65_538);
    d.contents.pop();
    assert_eq!(d.contents.len(), 65_535);
    assert!(d.validate(300., 500.).is_ok());
}
#[test]
fn local_validation_rejection_discards_only_failed_action_and_retains_draft() {
    let mut s = Session::<u8>::default();
    let o = owner();
    let ticket = s.begin(7, 0, o.request.clone(), o.target.clone()).unwrap();
    assert!(s.accept(&ticket, o));
    s.phase = Phase::Editing;
    s.draft.as_mut().unwrap().contents = "汉".repeat(21_846);
    let rejected = s.draft.as_ref().unwrap().payload();
    s.queue(Some(9));
    let error = s.freeze(300., 500.).unwrap_err();
    assert_eq!(s.intent, None, "locally refused action must be discarded");
    assert!(!s.finish_requested);
    assert_eq!(s.phase, Phase::Editing);
    assert_eq!(s.frozen, None);
    assert_eq!(s.error.as_ref(), Some(&error));
    assert_eq!(s.draft.as_ref().unwrap().payload(), rejected);
    s.draft.as_mut().unwrap().contents.pop();
    s.queue(Some(10));
    assert!(s.freeze(300., 500.).unwrap().is_some());
    assert_eq!(s.finish(&ticket, 7, true), Some(10));
}
#[test]
fn begin_admits_exact_owner_and_rejects_old_context_or_cancelled_begin() {
    let mut s = Session::<u8>::default();
    let o = owner();
    let ticket = s.begin(7, 0, o.request.clone(), o.target.clone()).unwrap();
    let mut wrong = ticket.clone();
    wrong.document_id = 8;
    assert!(!s.accept(&wrong, o.clone()));
    assert!(s.accept(&ticket, o.clone()));
    assert_eq!(s.draft.as_ref().unwrap().contents, "");
    assert_eq!(s.phase, Phase::Preparing);
    s.phase = Phase::Cancelling;
    assert!(!s.accept(&ticket, o));
}
#[test]
fn same_owner_read_only_refresh_matches_full_identity_without_replacing_local_draft() {
    let mut s = Session::<u8>::default();
    let o = owner();
    let ticket = s.begin(7, 0, o.request.clone(), o.target.clone()).unwrap();
    assert!(s.accept(&ticket, o.clone()));
    s.phase = Phase::Editing;
    s.draft.as_mut().unwrap().contents = "local unsaved".into();
    assert!(s.matches_owner(7, &o));
    assert!(!s.matches_owner(8, &o));
    let mut other = o.clone();
    other.id += 1;
    assert!(!s.matches_owner(7, &other));
    other = o.clone();
    other.appearance_style = None;
    assert!(!s.matches_owner(7, &other));
    assert_eq!(s.draft.as_ref().unwrap().contents, "local unsaved");
    assert_eq!(s.phase, Phase::Editing);
}
#[test]
fn uncertain_retry_freezes_complete_payload_including_none_style_and_geometry() {
    let mut s = Session::<u8>::default();
    let o = imported(0, None);
    let ticket = s.begin(7, 0, o.request.clone(), o.target.clone()).unwrap();
    assert!(s.accept(&ticket, o));
    s.phase = Phase::Editing;
    s.draft.as_mut().unwrap().bounds.x += 2.;
    s.queue(Some(9));
    let payload = s
        .freeze(300., 500.)
        .unwrap()
        .expect("editing must freeze actual payload");
    assert_eq!(payload.style, None);
    s.uncertain(&ticket, "原生响应超时；结果未知".into());
    s.reject_local("must not replace uncertain result".into());
    assert_eq!(s.phase, Phase::Uncertain);
    assert_eq!(s.error.as_deref(), Some("原生响应超时；结果未知"));
    s.queue(Some(10));
    assert_eq!(
        s.intent,
        Some(9),
        "uncertain result retains its original action"
    );
    let draft = s.draft.as_mut().unwrap();
    draft.contents = "later".into();
    draft.bounds.y += 4.;
    draft.style = Some(style());
    let retry = s.retry().expect("uncertain exact retry");
    assert_eq!(retry, payload);
    assert_eq!(
        serde_json::to_vec(&retry).unwrap(),
        serde_json::to_vec(&payload).unwrap()
    );
    assert_eq!(s.phase, Phase::Committing);
    assert_eq!(s.finish(&ticket, 7, true), Some(9));
}
#[test]
fn ime_defers_explicit_intent_and_terminal_reply_runs_it_once_not_blur_or_old_ticket() {
    let mut s = Session::<u8>::default();
    let o = owner();
    let ticket = s.begin(7, 0, o.request.clone(), o.target.clone()).unwrap();
    assert!(s.accept(&ticket, o));
    s.phase = Phase::Editing;
    s.composing = true;
    s.queue(Some(9));
    s.queue(None);
    s.queue(Some(10));
    assert_eq!(s.freeze(300., 500.).unwrap(), None);
    s.reject_local("composition is not an admitted attempt".into());
    assert_eq!(s.phase, Phase::Editing);
    assert_eq!(s.intent, Some(9));
    s.composing = false;
    assert!(s.freeze(300., 500.).unwrap().is_some());
    assert!(s.freeze(300., 500.).unwrap().is_none());
    let mut stale = ticket.clone();
    stale.generation += 1;
    assert_eq!(s.finish(&stale, 7, true), None);
    assert!(s.active());
    assert_eq!(s.finish(&ticket, 8, true), None);
    assert!(s.active());
    assert_eq!(s.finish(&ticket, 7, false), None);
    assert!(s.active());
    assert_eq!(s.finish(&ticket, 7, true), Some(9));
    assert!(!s.active());
    assert_eq!(s.finish(&ticket, 7, true), None);
}
