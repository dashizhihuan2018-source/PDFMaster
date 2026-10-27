use pdfmaster_ui::outline::{Cache, Row};

fn rows(page: usize) -> Vec<Row> {
    vec![
        Row {
            title: "第一章".into(),
            depth: 0,
            path: vec![0],
            page_index: Some(page),
        },
        Row {
            title: "外部目标".into(),
            depth: 0,
            path: vec![1],
            page_index: None,
        },
    ]
}

#[test]
fn only_current_projection_can_navigate_and_never_outside_current_page_count() {
    let mut cache = Cache::default();
    assert!(cache.can_begin());
    let ticket = cache.begin().expect("start read");
    assert!(!cache.can_begin());
    assert!(cache.begin().is_none());
    assert!(cache.finish(ticket, Ok(rows(2))));
    assert_eq!(cache.page(0, 3, false), Some(2));
    assert_eq!(cache.page(0, 2, false), None);
    assert_eq!(cache.page(0, 3, true), None);
    assert_eq!(cache.page(1, 3, false), None);
    assert_eq!(cache.page(2, 3, false), None);
    assert!(cache.begin().is_none());
    cache.invalidate();
    assert!(cache.can_begin());
    assert!(cache.result().is_none());
    assert_eq!(cache.page(0, 3, false), None);
}

#[test]
fn invalidation_keeps_the_inflight_slot_until_its_stale_reply_arrives() {
    let mut cache = Cache::default();
    let old = cache.begin().unwrap();
    cache.invalidate();
    assert!(cache.begin().is_none());
    assert!(!cache.finish(old, Ok(rows(0))));
    assert!(cache.result().is_none());
    let current = cache.begin().unwrap();
    assert_ne!(old, current);
    assert!(!cache.finish(old, Ok(rows(0))));
    assert!(cache.begin().is_none());
    assert!(cache.finish(current, Ok(rows(1))));
    assert_eq!(cache.page(0, 3, false), Some(1));
}

#[test]
fn timeouts_stop_followup_reads_even_after_document_replacement() {
    let mut cache = Cache::default();
    let ticket = cache.begin().unwrap();
    cache.suspend(ticket);
    assert!(cache.suspended());
    cache.invalidate();
    assert!(cache.begin().is_none());
    assert!(!cache.finish(ticket, Ok(rows(0))));
    assert!(cache.result().is_none());
}

#[test]
fn failed_reads_are_explicit_and_do_not_spin_until_document_changes() {
    let mut cache = Cache::default();
    let ticket = cache.begin().unwrap();
    assert!(cache.finish(ticket, Err("损坏的大纲".into())));
    assert_eq!(cache.result(), Some(&Err("损坏的大纲".into())));
    assert_eq!(cache.page(0, 3, false), None);
    assert!(cache.begin().is_none());
    cache.invalidate();
    assert!(cache.begin().is_some());
}

#[test]
fn outline_sidebar_has_guarded_local_navigation_and_document_invalidation() {
    let source = include_str!("../src/workspace.rs");
    let sidebar = include_str!("../src/thumbnail_view.rs");
    let css = include_str!("../workbench.css");
    assert!(source.contains("outline_cache: RwSignal<"));
    assert!(source.contains("self.outline_cache.update(|cache| cache.invalidate())"));
    assert!(sidebar.contains("\"outline_document\""));
    assert!(sidebar.contains("阅读器侧栏模式"));
    assert!(sidebar.contains("fn select_outline_row"));
    assert!(sidebar.contains("m.select_outline_row(index)"));
    assert!(
        sidebar.contains("cache.can_begin()"),
        "the reactive effect must not repeatedly notify an already-active slot"
    );
    assert!(sidebar.contains("此文档没有大纲"));
    assert!(sidebar.contains("外部目标不会打开"));
    assert!(css.contains(".outline-row{min-height:28px"));
}

#[test]
fn outline_projection_cache_is_a_testable_rust_state_machine() {
    assert!(include_str!("../src/lib.rs").contains("pub mod outline;"));
}

#[test]
fn outline_labels_follow_build46_compact_single_line_contract() {
    let source = include_str!("../src/thumbnail_view.rs");
    let css = include_str!("../workbench.css");
    assert!(source.contains("row.depth.min(12)"));
    assert!(!source.contains("无本文件内目标"));
    let declarations = |selector: &str| {
        css.split('}')
            .find_map(|rule| {
                let (name, declarations) = rule.split_once('{')?;
                (name.trim() == selector).then_some(declarations)
            })
            .expect("outline CSS rule")
    };
    let row = declarations(".outline-row");
    for required in [
        "min-height:28px",
        "height:28px",
        "box-sizing:border-box",
        "padding:0 6px",
    ] {
        assert!(
            row.split(';').any(|value| value == required),
            "missing {required}"
        );
    }
    let title = declarations(".outline-title");
    for required in [
        "display:block",
        "min-width:0",
        "white-space:pre",
        "overflow:hidden",
        "text-overflow:ellipsis",
        "font-size:12px",
    ] {
        assert!(
            title.split(';').any(|value| value == required),
            "missing {required}"
        );
    }
    assert!(!title.contains("line-clamp"));
    // Original tooltip is the full normalized title, without indentation or page suffix.
    assert!(source.contains("let tooltip=row.title.clone();"));
    assert!(source.contains("title=tooltip"));
    assert!(source.contains("page+1"));
}

#[test]
fn hovering_selected_outline_rows_does_not_hide_their_selection() {
    let css = include_str!("../workbench.css");
    assert!(!css.contains(".outline-row:hover:enabled{"));
    assert!(css.contains(".outline-row:hover:enabled:not(.selected){"));
}

#[test]
fn selected_outline_background_outranks_the_real_global_button_hover_rule() {
    let global = include_str!("../styles.css");
    let workbench = include_str!("../workbench.css");
    // Source-level cascade contract, not a substitute for native WebKit QA.
    // Both declarations apply to an enabled selected outline button on hover.
    // Compare the actual simple selectors' class/pseudo-class and type counts;
    // equal specificity is also safe because workbench.css is loaded last.
    let global_hover = global
        .split('}')
        .find_map(|rule| {
            let (selectors, declarations) = rule.split_once('{')?;
            selectors
                .split(',')
                .find(|selector| selector.trim() == "button:hover:enabled")
                .map(|selector| (selector.trim(), declarations))
        })
        .expect("global enabled hover rule must be included in this regression");
    assert!(global_hover.1.contains("background:"));
    let selected = workbench
        .split('}')
        .find_map(|rule| {
            let (selectors, declarations) = rule.split_once('{')?;
            selectors
                .split(',')
                .find(|selector| {
                    let selector = selector.trim();
                    selector.ends_with(".outline-row.selected")
                        && !selector.contains([':', '#', '['])
                })
                .map(|selector| (selector.trim(), declarations))
        })
        .expect("selected outline background declaration");
    assert!(selected.1.contains("background:#0a84ff26"));
    let specificity = |selector: &str| {
        (
            selector.matches('.').count() + selector.matches(':').count(),
            usize::from(selector.starts_with("button")),
        )
    };
    assert!(
        specificity(selected.0) >= specificity(global_hover.0),
        "selected rule {} must outrank global {}",
        selected.0,
        global_hover.0
    );
}
