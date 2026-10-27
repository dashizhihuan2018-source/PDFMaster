use pdfmaster_ui::overview::{Layout, Selection, scale};
use pdfmaster_ui::thumbnails::Window;

#[test]
fn keyboard_range_navigation_uses_the_last_clicked_page_not_the_largest_index() {
    let mut s = Selection::default();
    s.reset(5, 10);
    s.click(3, 10, true, false);
    assert_eq!(s.current(), Some(3));
    assert_eq!(s.pages(), vec![3, 4, 5]);
    s.click(2, 10, true, false);
    assert_eq!(s.current(), Some(2));
}

#[test]
fn overview_raster_window_covers_grid_pages_with_a_fixed_memory_bound() {
    assert_eq!(Window::for_pages(20..50, 100).rows(), 20..50);
    assert_eq!(Window::for_pages(20..5000, 4096).rows(), 20..276);
    assert_eq!(Window::for_pages(90..120, 100).rows(), 90..100);
    let reversed = std::ops::Range { start: 30, end: 20 };
    assert!(Window::for_pages(reversed, 100).rows().is_empty());
}

#[test]
fn overview_clicks_follow_the_original_anchor_and_modifier_rules() {
    let mut s = Selection::default();
    s.reset(2, 8);
    assert_eq!(s.pages(), vec![2]);
    s.click(5, 8, true, false);
    assert_eq!(s.pages(), vec![2, 3, 4, 5]);
    s.click(7, 8, false, true);
    assert_eq!(s.pages(), vec![2, 3, 4, 5, 7]);
    s.click(6, 8, true, false);
    assert_eq!(s.pages(), vec![6, 7]);
    s.click(3, 8, true, true);
    assert_eq!(s.pages(), vec![3, 4, 5, 6, 7]);
    s.click(5, 8, false, true);
    assert_eq!(s.pages(), vec![3, 4, 6, 7]);
    s.click(4, 8, false, false);
    assert_eq!(s.pages(), vec![4]);
    s.click(99, 8, false, false);
    assert_eq!(s.pages(), vec![4]);
}

#[test]
fn overview_move_plan_rejects_noops_and_restores_the_moved_range() {
    let mut s = Selection::default();
    s.restore(&[3, 1], 5);
    let plan = s.movement(2, 5).unwrap();
    assert_eq!(plan.pages, vec![1, 3]);
    assert_eq!(plan.insertion, 2);
    assert_eq!(plan.selected, 1..3);
    assert!(s.movement(6, 5).is_none());
    s.restore(&[2, 1], 5);
    for boundary in 1..=3 {
        assert!(s.movement(boundary, 5).is_none());
    }
    assert_eq!(s.movement(5, 5).unwrap().selected, 3..5);
    s.restore(&[0, 1, 2, 3, 4], 5);
    assert!(s.movement(5, 5).is_none());
    s.restore(&[90], 5);
    assert!(s.pages().is_empty());
    s.reset(9, 5);
    assert_eq!(s.pages(), vec![4]);
    s.reset(0, 0);
    assert!(s.pages().is_empty());
}

#[test]
fn overview_layout_uses_original_spacing_scale_and_rotated_page_aspects() {
    let pages = vec![
        (200., 400., 0),
        (200., 400., 90),
        (200., 200., 0),
        (200., 400., 0),
    ];
    let layout = Layout::new(800., 1., &pages);
    assert_eq!(layout.columns, 4);
    assert_eq!(layout.tiles[0].left, 20.);
    assert_eq!(layout.tiles[0].top, 20.);
    assert_eq!(layout.tiles[0].width, 178.);
    assert_eq!(layout.tiles[0].height, 384.);
    assert_eq!(layout.tiles[1].left, 214.);
    assert_eq!(layout.tiles[1].height, 117.);
    assert_eq!(layout.height, 424.);
    assert_eq!(scale(f64::NAN), 1.);
    assert_eq!(scale(0.1), 0.5);
    assert_eq!(scale(99.), 2.);
    assert_eq!(Layout::new(800., 2., &pages).columns, 2);
    let narrow = Layout::new(20., 1., &pages);
    assert_eq!(narrow.columns, 1);
    assert_eq!(narrow.tiles[0].width, 72.);
}

#[test]
fn overview_virtual_rows_and_hit_testing_keep_real_original_boundaries() {
    let layout = Layout::new(800., 1., &vec![(200., 200., 0); 4000]);
    assert!(layout.visible(10000., 600.).len() < 64);
    assert!(layout.visible(10000., 600.).start > 100);
    assert_eq!(layout.insertion(25., 30.), Some(0));
    assert_eq!(layout.insertion(190., 30.), Some(1));
    assert_eq!(layout.insertion(220., 30.), Some(1));
    assert_eq!(layout.insertion(500., layout.height + 1.), Some(4000));
    assert_eq!(layout.insertion(f64::NAN, 20.), None);
    assert_eq!(layout.visible(f64::NAN, f64::INFINITY).start, 0);
    assert_eq!(Layout::new(800., 1., &[]).visible(0., 600.), 0..0);
}

#[test]
fn background_below_short_tiles_and_in_gutters_appends_like_the_original() {
    let layout = Layout::new(
        800.,
        1.,
        &[
            (200., 400., 0),
            (200., 400., 90),
            (200., 200., 0),
            (200., 400., 0),
        ],
    );
    assert_eq!(layout.insertion(230., 300.), Some(4));
    assert_eq!(layout.insertion(205., 30.), Some(4));
    assert_eq!(layout.insertion(10., 30.), Some(4));
}
