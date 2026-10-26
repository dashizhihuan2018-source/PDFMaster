use pdfmaster_ui::{adjacent_search_hit, reveal_scroll};

#[test]
fn selection_reveal_scrolls_only_when_outside_the_viewport() {
    assert_eq!(reveal_scroll(0., 200., 250., 100.), 150.);
    assert_eq!(reveal_scroll(150., 0., 50., 100.), 0.);
    assert_eq!(reveal_scroll(100., 120., 160., 100.), 100.);
    assert_eq!(reveal_scroll(0., 50., 250., 100.), 50.);
    assert_eq!(reveal_scroll(150., 200., 250., 0.), 150.);
    assert_eq!(reveal_scroll(150., f64::NAN, 250., 100.), 150.);
    assert_eq!(reveal_scroll(150., 300., 200., 100.), 150.);
    assert_eq!(reveal_scroll(f64::INFINITY, 0., 20., 100.), 0.);
}

#[test]
fn no_selection_starts_at_the_directional_edge() {
    assert_eq!(adjacent_search_hit(None, 3, false), Some(0));
    assert_eq!(adjacent_search_hit(None, 3, true), Some(2));
    assert_eq!(adjacent_search_hit(Some(9), 3, false), Some(0));
    assert_eq!(adjacent_search_hit(Some(9), 3, true), Some(2));
}

#[test]
fn search_navigation_wraps_hits_not_page_numbers() {
    assert_eq!(adjacent_search_hit(Some(0), 3, true), Some(2));
    assert_eq!(adjacent_search_hit(Some(2), 3, false), Some(0));
    assert_eq!(adjacent_search_hit(Some(1), 3, false), Some(2));
    assert_eq!(adjacent_search_hit(Some(1), 3, true), Some(0));
}

#[test]
fn empty_and_single_hit_lists_are_safe_at_integer_edges() {
    assert_eq!(adjacent_search_hit(None, 0, true), None);
    assert_eq!(adjacent_search_hit(Some(usize::MAX), 0, false), None);
    assert_eq!(adjacent_search_hit(Some(0), 1, false), Some(0));
    assert_eq!(adjacent_search_hit(Some(0), 1, true), Some(0));
    assert_eq!(
        adjacent_search_hit(Some(usize::MAX - 1), usize::MAX, false),
        Some(0)
    );
    assert_eq!(
        adjacent_search_hit(Some(0), usize::MAX, true),
        Some(usize::MAX - 1)
    );
}

#[test]
fn search_controls_use_one_guarded_hit_selection_path() {
    let workspace = include_str!("../src/workspace.rs");
    let toolbar = include_str!("../src/workbench_view.rs");
    assert!(workspace.contains("fn select_search_hit(self, index: usize)"));
    assert!(workspace.contains("fn step_search_hit(self, backward: bool)"));
    assert!(workspace.contains("self.search_selection.set(None)"));
    assert!(workspace.contains("m.select_search_hit(index)"));
    assert!(workspace.contains("class:selected=move||m.search_selection.get()==Some(index)"));
    assert!(toolbar.contains("m.step_search_hit(true)"));
    assert!(toolbar.contains("m.step_search_hit(false)"));
    assert!(!toolbar.contains("逐命中工具栏导航尚未迁移"));
}

#[test]
fn search_hit_ordering_has_an_independent_testable_contract() {
    assert!(include_str!("../src/lib.rs").contains("pub fn adjacent_search_hit("));
}

#[test]
fn selected_result_is_visible_in_its_own_scroll_region() {
    let workspace = include_str!("../src/workspace.rs");
    assert!(workspace.contains("id=\"search-results\""));
    assert!(workspace.contains("id=format!(\"search-hit-{index}\")"));
    assert!(workspace.contains("fn reveal_search_hit(index: usize)"));
    assert!(workspace.contains("container.set_scroll_top"));
    let css = include_str!("../workbench.css");
    assert!(css.contains(".search-results{position:relative}"));
    assert!(css.contains(".search-hit.selected"));
}

#[test]
fn nested_inspector_scrolling_is_explicit_not_a_whole_page_scroll() {
    let source = include_str!("../src/workspace.rs");
    assert!(source.contains("container.closest(\".inspector\")"));
    assert!(source.contains("inspector.set_scroll_top"));
    assert!(include_str!("../src/lib.rs").contains("pub fn reveal_scroll("));
    assert!(!source.contains("scroll_into_view"));
}
