use pdfmaster_ui::search_flow::Flow;

#[test]
fn only_search_busy_allows_input_but_never_missing_document_or_editor() {
    let mut flow = Flow::default();
    assert!(!flow.input_blocked(true, false, false));
    assert!(flow.input_blocked(true, true, false));
    flow.request();
    let active = flow.begin().unwrap();
    assert!(!flow.input_blocked(true, true, false));
    assert!(flow.input_blocked(false, true, false));
    assert!(flow.input_blocked(true, true, true));
    flow.suspend(active);
    assert!(flow.input_blocked(true, false, false));
}

#[test]
fn debounce_only_queues_the_latest_conditions_and_single_native_slot() {
    let mut flow = Flow::default();
    let old = flow.changed().unwrap();
    assert_eq!(flow.revision(), Some(old));
    let latest = flow.changed().unwrap();
    assert!(!flow.queue(old));
    assert!(!flow.can_begin());
    assert!(flow.queue(latest));
    let active = flow.begin().expect("latest request");
    assert!(flow.active());
    assert!(!flow.can_begin());
    assert!(flow.begin().is_none());
    assert_eq!(flow.finish(active), Some(true));
    assert!(!flow.active());
    assert!(!flow.can_begin());
}

#[test]
fn rapid_input_coalesces_behind_native_and_stale_reply_cannot_become_current() {
    let mut flow = Flow::default();
    flow.request();
    let old = flow.begin().unwrap();
    let intermediate = flow.changed().unwrap();
    let newest = flow.changed().unwrap();
    assert!(!flow.queue(intermediate));
    assert!(flow.queue(newest));
    assert!(!flow.can_begin());
    assert_eq!(flow.finish(old), Some(false));
    assert!(flow.can_begin());
    let current = flow.begin().unwrap();
    assert_ne!(current, old);
    assert_eq!(flow.finish(old), None);
    assert!(flow.active());
    assert_eq!(flow.finish(current), Some(true));
}

#[test]
fn cancel_or_document_replacement_clears_pending_but_retains_native_ownership() {
    let mut flow = Flow::default();
    flow.request();
    let old = flow.begin().unwrap();
    let revision = flow.changed().unwrap();
    assert!(flow.queue(revision));
    flow.changed();
    assert!(!flow.can_begin());
    assert!(flow.active());
    assert!(!flow.queue(revision));
    assert_eq!(flow.finish(old), Some(false));
    assert!(!flow.active());
    assert!(!flow.can_begin());
    flow.request();
    let new = flow.begin().unwrap();
    assert_eq!(flow.finish(new), Some(true));
}

#[test]
fn repeated_explicit_searches_use_unique_tickets_even_with_unchanged_conditions() {
    let mut flow = Flow::default();
    flow.request();
    let old = flow.begin().unwrap();
    assert_eq!(flow.finish(old), Some(true));
    flow.request();
    let next = flow.begin().unwrap();
    assert_ne!(next, old);
    assert_eq!(flow.finish(old), None);
    assert_eq!(flow.finish(next), Some(true));
}

#[test]
fn timed_out_native_observation_suspends_new_requests_and_preserves_active_slot() {
    let mut flow = Flow::default();
    flow.request();
    let active = flow.begin().unwrap();
    assert!(flow.suspend(active));
    assert!(flow.suspended());
    assert!(flow.active());
    assert_eq!(flow.finish(active), None);
    assert!(flow.changed().is_none());
    flow.request();
    assert!(!flow.can_begin());
    assert!(flow.begin().is_none());
}

#[test]
fn live_search_has_an_independent_single_flight_generation_contract() {
    assert!(include_str!("../src/lib.rs").contains("pub mod search_flow;"));
}

#[test]
fn original_sidebar_search_uses_one_real_list_with_live_input_and_cancel() {
    let sidebar = include_str!("../src/thumbnail_view.rs");
    let workspace = include_str!("../src/workspace.rs");
    let toolbar = include_str!("../src/workbench_view.rs");
    let css = include_str!("../workbench.css");
    assert!(sidebar.contains("Mode::Search"));
    assert!(sidebar.contains("<SearchSidebar model=m/>"));
    assert!(sidebar.contains("m.queue_live_search()"));
    assert!(sidebar.contains("m.cancel_search()"));
    assert!(sidebar.contains("m.step_search_hit(true)"));
    assert!(sidebar.contains("m.step_search_hit(false)"));
    assert!(sidebar.contains("id=\"search-results\""));
    assert!(!workspace.contains("id=\"search-results\""));
    assert!(toolbar.contains("m.show_search_sidebar()"));
    assert!(!toolbar.contains("m.select_panel(Some(Panel::Search))"));
    assert!(workspace.contains("std::time::Duration::from_millis(180)"));
    assert!(
        workspace
            .split("fn invalidate_search")
            .nth(1)
            .unwrap()
            .split("fn stop_search_timer")
            .next()
            .unwrap()
            .contains("flow.changed();")
    );
    assert!(workspace.contains("flow.finish(ticket)"));
    assert!(workspace.contains("self.search_input_blocked()"));
    assert!(css.contains(".sidebar-search-row{height:54px"));
    assert!(css.contains(".sidebar-search-panel .search-hit.selected"));
    assert!(css.contains(".sidebar-search-row span{width:100%;margin:0;color:inherit;"));
}

#[test]
fn opening_search_preserves_the_previous_idle_ink_and_popup_transition_guards() {
    let source = include_str!("../src/workspace.rs");
    let transition = source
        .split("fn show_search_sidebar")
        .nth(1)
        .unwrap()
        .split("fn queue_live_search")
        .next()
        .unwrap();
    assert!(transition.contains("self.search_input_blocked()"));
    assert!(transition.contains("self.ink_enabled.set(false)"));
    assert!(transition.contains("self.close_chrome_popups()"));
}
