use pdfmaster_ui::thumbnails::{Cache, Image, Window, thumbnail_scale};

#[test]
fn timed_out_observation_never_opens_another_native_request_slot() {
    let mut cache = Cache::default();
    let window = Window::new(0., 600., 100);
    let ticket = cache.begin(0, window).unwrap();
    cache.suspend(ticket);
    assert!(cache.suspended());
    assert!(cache.next(window).is_none());
    assert!(!cache.retry(0));
    assert!(!cache.invalidate());
    assert!(cache.next(window).is_none());
}

#[test]
fn real_sidebar_uses_virtualized_rasters_and_invalidates_every_document_reply() {
    let ui = include_str!("../src/workspace.rs");
    let sidebar = include_str!("../src/thumbnail_view.rs");
    let css = include_str!("../workbench.css");
    assert!(ui.contains("<thumbnail_view::Sidebar model=m/>"));
    assert!(ui.contains("fn replace_document_view(self, document: Option<Document>)"));
    assert_eq!(
        ui.matches("self.document.set(").count(),
        1,
        "all document replacements must go through the cache invalidation wrapper"
    );
    assert!(sidebar.contains("cache.suspend(ticket)"));
    assert!(sidebar.contains("\"render_page\""));
    assert!(sidebar.contains("thumbnail_scale("));
    assert!(sidebar.contains("window.rows()"));
    assert!(sidebar.contains("window.before()"));
    assert!(sidebar.contains("window.after()"));
    assert!(sidebar.contains("aria-current"));
    assert!(css.contains(".thumbnail-paper{width:124px;height:164px"));
}

#[test]
fn resizing_remeasures_the_scroll_view_and_removes_its_listener_on_unmount() {
    let sidebar = include_str!("../src/thumbnail_view.rs");
    assert!(sidebar.contains("window_event_listener(leptos::ev::resize"));
    assert!(sidebar.contains("on_cleanup(move || resize.remove())"));
}

#[test]
fn mobile_sidebar_keeps_a_height_bounded_flex_scroll_view() {
    let css = include_str!("../workbench.css");
    let mobile = css.split_once("@media(max-width:720px)").unwrap().1;
    assert!(mobile.contains(".workspace .pages{display:flex;position:absolute"));
    assert!(!mobile.contains(".workspace .pages{display:block"));
}

#[test]
fn mobile_sidebar_anchors_to_the_reader_grid_area_not_a_second_toolbar_offset() {
    let css = include_str!("../workbench.css");
    let mobile = css.split_once("@media(max-width:720px)").unwrap().1;
    let sidebar = mobile
        .split_once(".workspace .pages{")
        .unwrap()
        .1
        .split_once('}')
        .unwrap()
        .0;
    assert!(sidebar.contains("grid-area:3/1/4/-1"));
    assert!(sidebar.contains("top:0;bottom:0"));
    assert!(!sidebar.contains("top:calc("));
    assert!(!sidebar.contains("bottom:68px"));
}

#[test]
fn visible_rows_are_bounded_and_spacers_preserve_real_scroll_height() {
    let first = Window::new(0., 600., 100);
    assert_eq!((first.start, first.end), (0, 5));
    assert_eq!((first.before(), first.after()), (0., 19000.));
    let middle = Window::new(1000., 600., 100);
    assert_eq!((middle.start, middle.end), (3, 10));
    assert_eq!(
        middle.before() + middle.rows().len() as f64 * 200. + middle.after(),
        20000.
    );
    for height in [0., -1., f64::NAN, f64::INFINITY, 1000000.] {
        let window = Window::new(f64::NAN, height, 2000);
        assert!(window.rows().len() <= 16);
        assert_eq!(
            window.before() + window.rows().len() as f64 * 200. + window.after(),
            400000.
        );
    }
    assert_eq!(Window::new(1000000., 600., 3).rows(), 0..3);
    assert!(Window::new(0., 600., 0).rows().is_empty());
}

#[test]
fn only_one_request_runs_and_old_generation_cannot_paint_a_replaced_document() {
    let window = Window::new(0., 600., 100);
    let mut cache = Cache::default();
    let old = cache.begin(0, window).unwrap();
    assert!(cache.begin(1, window).is_none());
    assert!(cache.invalidate());
    assert!(
        cache.next(window).is_none(),
        "keep the old in-flight slot until it actually resolves"
    );
    assert!(!cache.finish(old, window, Ok("data:image/png;base64,old".into())));
    assert!(cache.image(0).is_none());
    let new = cache.begin(0, window).unwrap();
    assert_ne!(old, new);
    assert!(!cache.finish(old, window, Ok("data:image/png;base64,stale".into())));
    assert!(
        cache.next(window).is_none(),
        "duplicate old completion must not release the new slot"
    );
    assert!(cache.finish(new, window, Ok("data:image/png;base64,new".into())));
    assert_eq!(
        cache.image(0),
        Some(&Image::Ready("data:image/png;base64,new".into()))
    );
}

#[test]
fn moving_window_evicts_offscreen_images_and_late_offscreen_results() {
    let mut cache = Cache::default();
    for top in (0..200000).step_by(2000) {
        let window = Window::new(top as f64, 600., 2000);
        while let Some(page) = cache.next(window) {
            let ticket = cache.begin(page, window).unwrap();
            assert!(cache.finish(ticket, window, Ok("data:image/png;base64,ok".into())));
        }
        assert!(cache.len() <= 16);
        assert!(cache.begin(window.end, window).is_none());
    }
    let first = Window::new(0., 600., 2000);
    let last = Window::new(300000., 600., 2000);
    cache.invalidate();
    let ticket = cache.begin(0, first).unwrap();
    assert!(!cache.finish(ticket, last, Ok("data:image/png;base64,late".into())));
    assert!(cache.image(0).is_none());
}

#[test]
fn failures_and_oversize_images_are_explicit_bounded_and_retryable() {
    let mut cache = Cache::default();
    let window = Window::new(0., 600., 3);
    for result in [
        Err("x".repeat(10000)),
        Ok("data:image/png;base64,".to_owned() + &"x".repeat(262144)),
        Ok("https://example.invalid/not-a-thumbnail".into()),
    ] {
        let ticket = cache.begin(0, window).unwrap();
        assert!(cache.finish(ticket, window, result));
        let Some(Image::Failed(message)) = cache.image(0) else {
            panic!("must show an explicit failure")
        };
        assert!(!message.is_empty() && message.chars().count() <= 240);
        assert_ne!(
            cache.next(window),
            Some(0),
            "do not spin retry a failing renderer"
        );
        assert!(cache.retry(0));
        assert_eq!(cache.next(window), Some(0));
    }
}

#[test]
fn thumbnail_scale_matches_original_124_by_164_box_without_retina_layout_growth() {
    let portrait = thumbnail_scale(595., 842., 0, 2.).unwrap();
    assert!((portrait - 328. / 842.).abs() < 0.0001);
    let landscape = thumbnail_scale(595., 842., 90, 2.).unwrap();
    assert!((landscape - 248. / 842.).abs() < 0.0001);
    assert_eq!(thumbnail_scale(595., 842., 0, 20.), Some(portrait));
    assert_eq!(
        thumbnail_scale(595., 842., 0, f32::NAN),
        thumbnail_scale(595., 842., 0, 1.)
    );
    for value in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(thumbnail_scale(value, 842., 0, 1.).is_none());
        assert!(thumbnail_scale(595., value, 0, 1.).is_none());
    }
    assert!(thumbnail_scale(595., 842., 45, 1.).is_none());
}
