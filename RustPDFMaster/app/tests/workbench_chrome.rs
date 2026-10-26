use pdfmaster_ui::workbench::{Action, Chrome, Mode, PRIMARY_MODES, Panel, fit_page_zoom, tools};

#[test]
fn original_primary_modes_and_secondary_orders_are_not_reinvented() {
    assert_eq!(
        PRIMARY_MODES.map(|p| p.0),
        ["批注", "编辑", "导出", "填写和签名", "扫描和 OCR", "度量"]
    );
    assert_eq!(
        tools(Mode::Annotation)
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        [
            "highlight",
            "underline",
            "strikeout",
            "pen",
            "eraser",
            "text",
            "shapes",
            "note",
            "stamp",
            "signature",
            "selection"
        ]
    );
    assert_eq!(
        tools(Mode::Editing)
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        [
            "content-text",
            "content-image",
            "content-link",
            "content-redact"
        ]
    );
    assert_eq!(
        tools(Mode::Pages).iter().map(|t| t.id).collect::<Vec<_>>(),
        [
            "insert",
            "attach",
            "rotate",
            "copy-page",
            "paste-page",
            "extract",
            "delete-page"
        ]
    );
    assert_eq!(tools(Mode::Drawing), tools(Mode::Annotation));
}

#[test]
fn choosing_a_mode_closes_only_the_context_panel_not_document_state() {
    let initial = Chrome::default();
    assert_eq!(initial.mode, None);
    assert_eq!(initial.panel, None);
    assert!(initial.sidebar);
    let search = initial.select_panel(Some(Panel::Search), false);
    assert_eq!(search.panel, Some(Panel::Search));
    let annotation = search.select_mode(Mode::Annotation, false);
    assert_eq!(annotation.mode, Some(Mode::Annotation));
    assert_eq!(annotation.panel, None);
    assert!(annotation.sidebar);
    let notes = annotation.select_panel(Some(Panel::Notes), false);
    assert_eq!(notes.mode, annotation.mode);
    assert_eq!(notes.panel, Some(Panel::Notes));
    assert_eq!(notes.select_panel(None, false).panel, None);
}

#[test]
fn unfinished_edit_or_busy_work_cannot_be_hidden_by_chrome_changes() {
    let current = Chrome::default()
        .select_mode(Mode::Drawing, false)
        .select_panel(Some(Panel::Ink), false);
    for mode in [
        Mode::Annotation,
        Mode::Editing,
        Mode::Drawing,
        Mode::Pages,
        Mode::Signature,
        Mode::Export,
    ] {
        assert_eq!(current.select_mode(mode, true), current);
    }
    assert_eq!(current.select_panel(None, true), current);
    assert_eq!(current.select_panel(Some(Panel::Search), true), current);
    assert_eq!(current.toggle_sidebar(true), current);
    assert_ne!(current.toggle_sidebar(false).sidebar, current.sidebar);
}

#[test]
fn missing_tools_are_disabled_and_copy_is_not_mislabelled_duplicate() {
    for mode in [
        Mode::Editing,
        Mode::Annotation,
        Mode::Pages,
        Mode::Signature,
        Mode::Export,
    ] {
        for tool in tools(mode) {
            if matches!(
                tool.id,
                "copy-page"
                    | "paste-page"
                    | "signature"
                    | "selection"
                    | "password"
                    | "export-text"
                    | "highlight"
                    | "underline"
                    | "strikeout"
                    | "eraser"
                    | "stamp"
            ) || tool.id.starts_with("content-")
            {
                assert!(
                    tool.action.is_none(),
                    "{} must not invoke an unrelated edit",
                    tool.id
                );
            }
        }
    }
    assert_eq!(
        tools(Mode::Annotation)
            .iter()
            .find(|t| t.id == "note")
            .unwrap()
            .action,
        Some(Action::Panel(Panel::Notes))
    );
    assert_eq!(tools(Mode::Export)[0].action, Some(Action::Export));
    assert_eq!(tools(Mode::Pages)[2].action, Some(Action::Rotate));
}

#[test]
fn fit_page_uses_both_actual_content_dimensions_and_rotation() {
    assert!((fit_page_zoom(595., 842., 0, 600., 400.).unwrap() - 400. / 842.).abs() < 0.0001);
    assert!((fit_page_zoom(595., 842., 90, 600., 400.).unwrap() - 400. / 595.).abs() < 0.0001);
    assert!((fit_page_zoom(595., 842., 270, 300., 800.).unwrap() - 300. / 842.).abs() < 0.0001);
    assert_eq!(fit_page_zoom(595., 842., 0, 10000., 10000.), Some(3.));
    for value in [0., -1., f32::NAN, f32::INFINITY] {
        assert_eq!(fit_page_zoom(595., 842., 0, value, 400.), None);
        assert_eq!(fit_page_zoom(595., 842., 0, 600., value), None);
        assert_eq!(fit_page_zoom(value, 842., 0, 600., 400.), None);
        assert_eq!(fit_page_zoom(595., value, 0, 600., 400.), None);
    }
    assert_eq!(fit_page_zoom(595., 842., 45, 600., 400.), None);
}

#[test]
fn real_ui_replaces_the_always_open_inspector_with_context_panels() {
    let ui = include_str!("../src/workspace.rs");
    let index = include_str!("../index.html");
    assert!(ui.contains("<workbench::Toolbar model=m/>"));
    assert!(ui.contains("<workbench::SecondaryTools model=m/>"));
    assert!(ui.contains("hidden=move||m.chrome.get().panel!=Some(Panel::Notes)"));
    assert!(ui.contains("hidden=move||m.chrome.get().panel!=Some(Panel::Ink)"));
    assert!(ui.contains("hidden=move||m.chrome.get().panel.is_none()"));
    assert!(index.contains("href=\"workbench.css\""));
    assert!(!ui.contains("<nav class=\"commandbar\""));
}

#[test]
fn restored_editors_are_visible_before_transition_guards_lock_the_ui() {
    let initial = Chrome::default();
    let note = initial.document_opened(true, false, false);
    assert_eq!(note.panel, Some(Panel::Notes));
    assert_eq!(note.mode, Some(Mode::Annotation));
    let ink = initial.document_opened(false, true, false);
    assert_eq!(ink.panel, Some(Panel::Ink));
    assert_eq!(ink.mode, Some(Mode::Drawing));
    assert_eq!(note.document_opened(false, false, false), note);
    assert_eq!(note.document_opened(false, false, true), initial);
    let collapsed = note.toggle_sidebar(false);
    assert!(
        !collapsed.document_opened(false, false, true).sidebar,
        "replacing a document must not reopen the mobile sidebar over its first page"
    );
}

#[test]
fn home_annotation_opens_original_mode_not_legacy_text_debug_panel() {
    assert_eq!(
        pdfmaster_ui::shell::tool("annotate")
            .unwrap()
            .workbench_target,
        Some("mode-annotation")
    );
}

#[test]
fn zoom_readout_is_reactive_and_narrow_desktops_keep_control_groups_reachable() {
    let ui = include_str!("../src/workbench_view.rs");
    let css = include_str!("../workbench.css");
    assert!(ui.contains("prop:value=move||{let _=m.zoom.get();\"current\"}"));
    assert!(css.contains("@media(max-width:1050px)"));
    assert!(css.contains(".workbench-group{height:44px;overflow-x:auto"));
}

#[test]
fn choosing_popup_commands_dismisses_the_menu_before_showing_the_tool() {
    let ui = include_str!("../src/workspace.rs");
    assert!(ui.contains("fn close_chrome_popups(self)"));
    assert!(ui.contains(".workbench-toolbar details[open]"));
    for (start, end) in [
        ("fn select_mode(", "fn select_panel("),
        ("fn select_panel(", "fn toggle_sidebar("),
        ("fn fit_page(", "fn run_strip_action("),
        ("fn fit_width(", "fn add_text("),
    ] {
        let body = ui.split_once(start).unwrap().1.split_once(end).unwrap().0;
        assert!(body.contains("self.close_chrome_popups();"), "{start}");
        assert!(
            body.find("if self.blocked()")
                .or_else(|| body.find("if blocked"))
                .unwrap()
                < body.find("self.close_chrome_popups();").unwrap()
        );
    }
}
