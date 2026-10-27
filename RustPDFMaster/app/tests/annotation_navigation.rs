use pdfmaster_ui::annotation_navigation::{Axis, Context, Navigation, Page, Region, Target};
use pdfmaster_ui::annotations::{Bounds, Identity, Row};
use serde_json::json;

fn row() -> Row {
    serde_json::from_value(json!({"page_index":2,"annotation_index":7,"page_id":[90,0],"object_id":[91,1],"subtype":"Text","contents":"position","modified":null,"color":null,"bounds":{"x":20.,"y":30.,"width":40.,"height":20.}})).unwrap()
}
fn page(rotation: i32) -> Page {
    Page {
        width: 200.,
        height: 120.,
        rotation,
    }
}
fn context() -> Context {
    Context {
        generation: Some(4),
        page: 2,
        selection: Some(row().identity()),
    }
}
fn target() -> Target {
    Target::new(&row(), Some(4), page(0)).unwrap()
}
fn near(actual: Region, expected: [f64; 4]) {
    for (a, b) in [actual.left, actual.top, actual.right, actual.bottom]
        .into_iter()
        .zip(expected)
    {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
}

#[test]
fn real_component_waits_for_decoded_current_image_before_revealing_only_reader() {
    let view = include_str!("../src/annotation_view.rs").replace("\r\n", "\n");
    assert!(
        view.contains("Target::new("),
        "annotation click currently discards bounds"
    );
    assert!(view.contains("self.navigate_with_annotation(row.page_index, target)"));
    assert!(view.contains("request_animation_frame"));
    assert!(view.contains("natural_width()"));
    assert!(view.contains("current_src()"));
    assert!(view.contains("get_element_by_id(\"pdf-reader\")"));
    assert!(view.contains("reader.set_scroll_left("));
    assert!(view.contains("reader.set_scroll_top("));
    assert!(!view.contains("scroll_into_view"));
    let workspace = include_str!("../src/workspace.rs").replace("\r\n", "\n");
    assert!(workspace.contains("id=\"pdf-page-image\""));
    assert!(workspace.contains("self.queue_annotation_reveal();"));
    let render = workspace
        .split("async fn render(self)")
        .nth(1)
        .unwrap()
        .split("async fn refresh_notes")
        .next()
        .unwrap();
    assert!(render.contains("nav.rendered(ticket)"));
    assert!(render.contains("nav.abandon(ticket)"));
    let replace = workspace
        .split("fn replace_document_view")
        .nth(1)
        .unwrap()
        .split("fn close_chrome_popups")
        .next()
        .unwrap();
    assert!(replace.contains("nav.cancel()"));
    let css = include_str!("../workbench.css");
    assert!(
        css.contains(".application-shell .workspace .reader{justify-content:flex-start}"),
        "large pages must not hide their left half outside the scroll origin"
    );
}

#[test]
fn original_twelve_point_margin_maps_all_quarter_turns_without_using_device_pixels() {
    for (rotation, expected) in [
        (0, [8. / 200., 58. / 120., 72. / 200., 102. / 120.]),
        (90, [18. / 120., 8. / 200., 62. / 120., 72. / 200.]),
        (180, [128. / 200., 18. / 120., 192. / 200., 62. / 120.]),
        (270, [58. / 120., 128. / 200., 102. / 120., 192. / 200.]),
        (-90, [58. / 120., 128. / 200., 102. / 120., 192. / 200.]),
        (450, [18. / 120., 8. / 200., 62. / 120., 72. / 200.]),
    ] {
        let target = Target::new(&row(), Some(4), page(rotation)).unwrap();
        assert_eq!(target.page, 2);
        assert_eq!(target.identity, row().identity());
        near(target.region, expected);
    }
}

#[test]
fn partial_edge_rectangles_clip_to_page_and_missing_invalid_or_off_page_bounds_only_jump_page() {
    let mut r = row();
    r.bounds = Some(Bounds {
        x: -5.,
        y: 110.,
        width: 30.,
        height: 25.,
    });
    near(
        Target::new(&r, Some(4), page(0)).unwrap().region,
        [0., 0., 37. / 200., 22. / 120.],
    );
    for bounds in [
        None,
        Some(Bounds {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 5.,
        }),
        Some(Bounds {
            x: 201.,
            y: 30.,
            width: 4.,
            height: 5.,
        }),
        Some(Bounds {
            x: 0.,
            y: 120.,
            width: 4.,
            height: 5.,
        }),
        Some(Bounds {
            x: f32::NAN,
            y: 0.,
            width: 4.,
            height: 5.,
        }),
        Some(Bounds {
            x: 0.,
            y: 0.,
            width: f32::INFINITY,
            height: 5.,
        }),
        Some(Bounds {
            x: 0.,
            y: 0.,
            width: -1.,
            height: 5.,
        }),
    ] {
        r.bounds = bounds;
        assert!(Target::new(&r, Some(4), page(0)).is_none());
    }
    assert!(Target::new(&row(), None, page(0)).is_none());
    for invalid in [
        Page {
            width: 0.,
            ..page(0)
        },
        Page {
            height: f64::NAN,
            ..page(0)
        },
        page(45),
    ] {
        assert!(Target::new(&row(), Some(4), invalid).is_none());
    }
}

#[test]
fn very_large_finite_annotation_intersects_safely_instead_of_overflowing() {
    let mut r = row();
    r.bounds = Some(Bounds {
        x: -f32::MAX,
        y: -f32::MAX,
        width: f32::MAX,
        height: f32::MAX,
    });
    assert!(Target::new(&r, Some(4), page(0)).is_none());
    r.bounds = Some(Bounds {
        x: -1.,
        y: -1.,
        width: f32::MAX,
        height: f32::MAX,
    });
    near(
        Target::new(&r, Some(4), page(0)).unwrap().region,
        [0., 0., 1., 1.],
    );
}

#[test]
fn reveal_uses_live_css_image_offsets_borders_and_scroll_on_each_axis() {
    let axis = Axis {
        scroll: 100.,
        viewport: 80.,
        viewport_start: 50.,
        border: 2.,
        page_start: 12.,
        page_extent: 200.,
    };
    assert_eq!(axis.reveal(0.2, 0.4), Some(100)); // [100,140] already visible
    assert_eq!(axis.reveal(0.8, 0.9), Some(160)); // [220,240], nearest end
    assert_eq!(axis.reveal(0., 0.1), Some(60));
    assert_eq!(axis.reveal(0., 1.), Some(60)); // oversized target: start, no oscillation
    let scaled = Axis {
        page_start: 72.,
        page_extent: 400.,
        ..axis
    };
    assert_eq!(scaled.reveal(0.8, 0.9), Some(400)); // actual CSS size, not DPR
    assert_eq!(
        Axis {
            page_start: -1000.,
            ..axis
        }
        .reveal(0., 0.1),
        Some(0)
    );
    for invalid in [
        Axis {
            viewport: 0.,
            ..axis
        },
        Axis {
            scroll: f64::NAN,
            ..axis
        },
        Axis {
            page_extent: -1.,
            ..axis
        },
    ] {
        assert_eq!(invalid.reveal(0., 1.), None);
    }
    assert_eq!(axis.reveal(-0.1, 0.5), None);
    assert_eq!(axis.reveal(0.5, 0.4), None);
}

#[test]
fn deferred_reveal_requires_render_success_decode_and_current_document_page_selection() {
    let mut nav = Navigation::default();
    let ticket = nav.begin(Some(target())).unwrap();
    assert_eq!(nav.ticket(), Some(ticket));
    assert_eq!(nav.schedule(context(), true), None); // old image may still be decoded
    nav.rendered(ticket);
    assert_eq!(nav.schedule(context(), false), None);
    assert_eq!(nav.schedule(context(), true), Some(ticket));
    assert_eq!(nav.schedule(context(), true), None); // one frame only
    assert_eq!(nav.take(ticket, context()), Some(target()));
    assert_eq!(nav.take(ticket, context()), None);
    for stale in [
        Context {
            generation: Some(5),
            ..context()
        },
        Context {
            page: 1,
            ..context()
        },
        Context {
            selection: None,
            ..context()
        },
        Context {
            selection: Some(Identity((100, 0), None, 0)),
            ..context()
        },
    ] {
        let ticket = nav.begin(Some(target())).unwrap();
        nav.rendered(ticket);
        assert_eq!(nav.schedule(stale, true), None);
        assert_eq!(nav.schedule(context(), true), Some(ticket));
        assert_eq!(nav.take(ticket, stale), None);
        assert_eq!(nav.ticket(), None);
    }
}

#[test]
fn same_row_reselection_rejects_old_render_frame_and_failure_without_clearing_new_owner() {
    let mut nav = Navigation::default();
    let old = nav.begin(Some(target())).unwrap();
    nav.rendered(old);
    assert_eq!(nav.schedule(context(), true), Some(old));
    let current = nav.begin(Some(target())).unwrap();
    assert_ne!(old, current);
    nav.rendered(old);
    nav.abandon(old);
    assert_eq!(nav.take(old, context()), None);
    assert_eq!(nav.schedule(context(), true), None);
    assert_eq!(nav.ticket(), Some(current));
    nav.rendered(current);
    assert_eq!(nav.schedule(context(), true), Some(current));
    assert!(nav.take(current, context()).is_some());
    let failed = nav.begin(Some(target())).unwrap();
    nav.abandon(failed);
    assert_eq!(nav.ticket(), None);
    nav.begin(Some(target()));
    nav.cancel();
    assert_eq!(nav.ticket(), None);
    nav.begin(Some(target()));
    nav.begin(None);
    assert_eq!(nav.ticket(), None);
}

#[test]
fn image_events_capture_the_published_owner_instead_of_cancelling_the_current_navigation() {
    let source = include_str!("../src/workspace.rs").replace("\r\n", "\n");
    assert!(
        source.contains("m.page_image_failed(owner, &failed_source)"),
        "old image error must abandon only its captured owner"
    );
    assert!(
        source.contains("children=move|(source,owner)|"),
        "each image element must capture its own source and owner"
    );
    let error = source
        .split("fn page_image_failed(")
        .nth(1)
        .unwrap()
        .split("fn ink_image_decoded")
        .next()
        .unwrap();
    assert!(error.contains("nav.abandon(ticket)"));
    assert!(!error.contains("nav.cancel()"));
    assert!(error.contains("self.annotation_image_ticket.get_untracked() == owner"));
    let mut nav = Navigation::default();
    let displayed_a = nav.begin(Some(target())).unwrap();
    nav.rendered(displayed_a);
    let waiting_b = nav.begin(Some(target())).unwrap();
    nav.abandon(displayed_a); // A's late decode error while B's RPC is pending
    assert_eq!(nav.ticket(), Some(waiting_b));
    nav.rendered(waiting_b);
    assert_eq!(nav.schedule(context(), true), Some(waiting_b));
    nav.abandon(displayed_a); // even after B has published a new element
    assert!(nav.take(waiting_b, context()).is_some());
}

#[test]
fn unchanged_ink_cancellation_keeps_decoded_fast_path_and_main_first_decode_can_finish() {
    let source = include_str!("../src/workspace.rs").replace("\r\n", "\n");
    let render = source
        .split("async fn render(self)")
        .nth(1)
        .unwrap()
        .split("async fn refresh_notes")
        .next()
        .unwrap();
    let cached = render
        .find("let same_decoded_image")
        .expect("capture actual decoded same-image state before changing its owner");
    assert!(
        cached
            < render
                .find("self.annotation_image_ticket.set(annotation_ticket)")
                .unwrap()
    );
    assert!(render.contains("if same_decoded_image"));
    let staged = source
        .split("fn ink_image_decoded")
        .nth(1)
        .unwrap()
        .split("fn ink_image_failed")
        .next()
        .unwrap();
    assert!(staged.contains("annotation_view::decoded_page_image(self).is_some()"));
    assert!(
        staged.contains(
            "self.image_loaded(Some(image), self.annotation_image_ticket.get_untracked())"
        )
    );
    let mut handoff = pdfmaster_ui::ink::InkImageHandoff {
        previous: "same".into(),
        next: "same".into(),
        revision: 9,
        decoded: false,
    };
    assert!(!handoff.loaded(9, "same")); // main decoded first, staging not ready yet
    assert_eq!(handoff.decoded(9), Some("same".into()));
    assert!(handoff.loaded(9, "same")); // must not require another main load event
    assert!(!handoff.loaded(8, "same"));
}
