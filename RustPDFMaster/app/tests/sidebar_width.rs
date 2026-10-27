#[test]
fn sidebar_width_is_a_shared_testable_contract() {
    assert!(include_str!("../src/lib.rs").contains("pub mod sidebar_width;"));
}

use pdfmaster_ui::sidebar_width::{DEFAULT, MAX, MIN, READER_MIN, Width};

#[test]
fn restored_preference_survives_temporary_narrow_viewports() {
    let width = Width::restore(Some("320"));
    assert_eq!(width.preferred(), 320.);
    assert_eq!(width.effective(430.), 230.);
    assert_eq!(width.effective(1200.), 320.);
    assert_eq!(Width::restore(Some(&width.saved())).preferred(), 320.);
    // No mutation for modes, visibility or a window resize.
    assert_eq!(width.preferred(), 320.);
}

#[test]
fn invalid_storage_never_produces_nonfinite_or_unbounded_geometry() {
    for value in [
        None,
        Some("NaN"),
        Some("inf"),
        Some("-20"),
        Some("junk"),
        Some("9999999999999999999999999999999999999999999"),
    ] {
        assert_eq!(Width::restore(value).preferred(), DEFAULT);
    }
    assert_eq!(Width::restore(Some("9999")).preferred(), MAX);
    assert_eq!(Width::restore(Some("1")).preferred(), MIN);
}

#[test]
fn dragging_is_live_owned_by_one_pointer_and_saves_only_finished_width() {
    let mut width = Width::default();
    assert!(width.begin(7, 176., 1200.));
    assert!(width.dragging());
    assert!(!width.begin(8, 200., 1200.));
    assert!(!width.drag(8, 450., 1200.));
    assert!(!width.finish(8));
    assert!(width.drag(7, 300., 1200.));
    assert_eq!(width.effective(1200.), 300.);
    assert!(width.finish(7));
    assert!(!width.dragging());
    assert!(!width.finish(7));
    assert_eq!(width.saved(), "300");
}

#[test]
fn extreme_drags_and_window_shrink_leave_a_reading_region() {
    for viewport in [0., 120., 360., 430., 720., 1200., 4000.] {
        let mut width = Width::default();
        assert!(width.begin(1, 176., viewport));
        assert!(width.drag(1, 1e20, viewport));
        let (min, max) = Width::bounds(viewport);
        assert_eq!(width.effective(viewport), max);
        assert!(max <= MAX);
        assert!(max <= (viewport - READER_MIN).max(0.));
        assert!(width.drag(1, -1e20, viewport));
        assert_eq!(width.effective(viewport), min);
        assert!(min <= max);
    }
}

#[test]
fn cancellation_and_lost_capture_restore_pre_drag_preference() {
    let mut width = Width::restore(Some("400"));
    assert!(width.begin(4, 230., 430.));
    assert!(width.drag(4, 200., 430.));
    assert_eq!(width.preferred(), 200.);
    assert!(!width.cancel(5));
    assert!(width.cancel(4));
    assert_eq!(width.preferred(), 400.);
    assert!(width.begin(4, 400., 1200.));
    assert!(width.drag(4, 300., 1200.));
    width.cancel_active();
    assert_eq!(width.preferred(), 400.);
    assert!(!width.dragging());
}

#[test]
fn invalid_coordinates_and_keyboard_do_not_steal_an_active_drag() {
    let mut width = Width::default();
    assert!(!width.begin(1, f64::NAN, 1200.));
    assert!(width.begin(1, 176., 1200.));
    assert!(!width.drag(1, f64::INFINITY, 1200.));
    assert!(!width.key("End", 1200.));
    assert_eq!(width.preferred(), DEFAULT);
    width.cancel_active();
    assert!(width.key("ArrowRight", 1200.));
    assert_eq!(width.preferred(), 186.);
    assert!(width.key("ArrowLeft", 1200.));
    assert_eq!(width.preferred(), DEFAULT);
    assert!(width.key("End", 430.));
    assert_eq!(width.effective(430.), 230.);
    assert!(width.key("Home", 430.));
    assert_eq!(width.effective(430.), MIN);
    assert!(!width.key("a", 430.));
    for viewport in [f64::NAN, f64::INFINITY, -1.] {
        assert!(width.effective(viewport).is_finite());
        assert_eq!(width.effective(viewport), MIN);
    }
}

#[test]
fn actual_sidebar_wires_capture_cleanup_persistence_and_shared_geometry() {
    let workspace = include_str!("../src/workspace.rs");
    let sidebar = include_str!("../src/thumbnail_view.rs");
    let css = include_str!("../workbench.css");
    assert!(workspace.contains("sidebar_width: RwSignal"));
    assert!(workspace.contains("--reader-sidebar-width:"));
    assert!(sidebar.contains("role=\"separator\""));
    assert!(sidebar.contains("aria-orientation=\"vertical\""));
    assert!(sidebar.contains("set_pointer_capture"));
    assert!(sidebar.contains("release_pointer_capture"));
    assert!(sidebar.contains("on:lostpointercapture"));
    assert!(sidebar.contains("on:pointercancel"));
    assert!(sidebar.contains("on:keydown"));
    assert!(sidebar.contains("leptos::ev::blur"));
    assert!(sidebar.contains("local_storage()"));
    assert!(sidebar.contains("set_item(STORAGE_KEY"));
    assert!(sidebar.contains("resize.remove()"));
    assert!(sidebar.contains("blur.remove()"));
    assert!(css.contains("grid-template-columns:var(--reader-sidebar-width,176px) minmax(0,1fr)"));
    assert!(css.contains("width:var(--reader-sidebar-width,176px)"));
    assert!(css.contains("touch-action:none"));
    assert!(css.contains("cursor:col-resize"));
    // Changing visibility/mode cannot reset the shared preferred width.
    let toggle = workspace
        .split("fn toggle_sidebar")
        .nth(1)
        .unwrap()
        .split("\n    fn ")
        .next()
        .unwrap();
    assert!(!toggle.contains("sidebar_width"));
    let mode = sidebar
        .split("fn select_sidebar_mode")
        .nth(1)
        .unwrap()
        .split("\n    fn ")
        .next()
        .unwrap();
    assert!(!mode.contains("sidebar_width"));
}
