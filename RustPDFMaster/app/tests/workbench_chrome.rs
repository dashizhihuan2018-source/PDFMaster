use pdfmaster_ui::workbench::{
    Action, ActiveTool, Chrome, Mode, PRIMARY_MODES, Panel, fit_page_zoom, tools,
};

#[test]
fn shapes_is_a_direct_page_tool_not_a_parameter_panel() {
    let tool = tools(Mode::Annotation)
        .iter()
        .find(|t| t.id == "shapes")
        .unwrap();
    assert!(
        matches!(tool.action, Some(Action::Activate(_))),
        "Shapes must activate directly: {:?}",
        tool.action
    );
}

#[test]
fn text_is_a_direct_page_tool_not_a_parameter_panel() {
    let text = tools(Mode::Annotation)
        .iter()
        .find(|t| t.id == "text")
        .unwrap();
    assert!(
        matches!(text.action, Some(Action::Activate(_))),
        "Text must activate directly: {:?}",
        text.action
    );
    let chrome = Chrome::default().select_mode(Mode::Annotation, false);
    let text = chrome.activate_tool(ActiveTool::Text, false);
    assert_eq!(text.panel, None);
    assert_eq!(text.active_tool, Some(ActiveTool::Text));
    assert_eq!(text.activate_tool(ActiveTool::Text, false), chrome);
    let signing = Chrome::default().select_mode(Mode::Signature, false);
    assert_eq!(
        signing.activate_tool(ActiveTool::Text, false).mode,
        Some(Mode::Signature)
    );
}

#[test]
fn note_is_a_direct_mutually_exclusive_tool() {
    let idle = Chrome::default().select_mode(Mode::Annotation, false);
    let note = idle.activate_tool(ActiveTool::Note, false);
    assert_eq!(note.active_tool, Some(ActiveTool::Note));
    assert_eq!(note.panel, None);
    assert_eq!(note.mode, Some(Mode::Annotation));
    assert_eq!(note.activate_tool(ActiveTool::Note, false), idle);
    assert_eq!(
        note.activate_tool(ActiveTool::Ink, false).active_tool,
        Some(ActiveTool::Ink)
    );
    assert_eq!(note.select_panel(Some(Panel::Notes), true), note);
    assert_eq!(
        idle.select_panel(Some(Panel::Notes), false).active_tool,
        Some(ActiveTool::Note)
    );
}

#[test]
fn pen_activation_is_semantic_and_toggles_without_a_panel() {
    let initial = Chrome::default().select_mode(Mode::Annotation, false);
    let active = initial.activate_tool(ActiveTool::Ink, false);
    assert_eq!(active.mode, Some(Mode::Annotation));
    assert_eq!(active.active_tool, Some(ActiveTool::Ink));
    assert_eq!(active.panel, None);
    assert_eq!(active.activate_tool(ActiveTool::Ink, false), initial);
    assert_eq!(active.activate_tool(ActiveTool::Ink, true), active);
    assert_eq!(active.select_mode(Mode::Editing, true), active);
    assert_eq!(active.select_mode(Mode::Editing, false).active_tool, None);
    assert_eq!(
        active.activate_tool(ActiveTool::Shapes, false).active_tool,
        Some(ActiveTool::Shapes)
    );
}

#[test]
fn native_ink_restore_has_tool_state_but_never_a_parameter_panel() {
    let restored = Chrome::default().document_opened(false, true, false);
    assert_eq!(restored.active_tool, Some(ActiveTool::Ink));
    assert_eq!(restored.panel, None);
    assert_eq!(restored.mode, Some(Mode::Drawing));
    assert_eq!(restored.activate_tool(ActiveTool::Shapes, true), restored);
    assert_eq!(
        restored.document_opened(false, false, true),
        Chrome::default()
    );
}

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
    assert_eq!(notes.panel, None);
    assert_eq!(notes.active_tool, Some(ActiveTool::Note));
    assert_eq!(notes.select_panel(None, false).panel, None);
}

#[test]
fn unfinished_edit_or_busy_work_cannot_be_hidden_by_chrome_changes() {
    let current = Chrome::default()
        .select_mode(Mode::Drawing, false)
        .activate_tool(ActiveTool::Ink, false);
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
                "signature"
                    | "selection"
                    | "password"
                    | "highlight"
                    | "underline"
                    | "strikeout"
                    | "stamp"
            ) || tool.id.starts_with("content-")
            {
                assert!(
                    tool.action.is_none(),
                    "{} must not invoke an unrelated edit",
                    tool.id
                );
            }
            if tool.id == "eraser" {
                assert_eq!(tool.action, Some(Action::Activate(ActiveTool::Eraser)));
            }
        }
    }
    assert_eq!(
        tools(Mode::Export)
            .iter()
            .find(|tool| tool.id == "export-text")
            .unwrap()
            .action,
        Some(Action::ExportText(
            pdfmaster_ui::text_export::TextExportFormat::PlainText
        ))
    );
    assert_eq!(
        tools(Mode::Annotation)
            .iter()
            .find(|t| t.id == "note")
            .unwrap()
            .action,
        Some(Action::Activate(ActiveTool::Note))
    );
    assert_eq!(
        tools(Mode::Annotation)
            .iter()
            .find(|t| t.id == "pen")
            .unwrap()
            .action,
        Some(Action::Activate(ActiveTool::Ink))
    );
    assert_eq!(tools(Mode::Export)[0].action, Some(Action::Export));
    assert_eq!(tools(Mode::Pages)[2].action, Some(Action::Rotate));
    assert_eq!(tools(Mode::Pages)[3].action, Some(Action::CopyPages));
    assert_eq!(tools(Mode::Pages)[4].action, Some(Action::PastePages));
    let overview = include_str!("../src/overview_view.rs");
    assert!(overview.contains("\"copy_pages_document\""));
    assert!(overview.contains("\"paste_pages_document\""));
    assert!(!overview.contains("\"duplicate_page\""));
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
fn real_ui_replaces_the_inspector_with_actual_commands() {
    let ui = include_str!("../src/workspace.rs");
    let index = include_str!("../index.html");
    assert!(ui.contains("<workbench::Toolbar model=m/>"));
    assert!(ui.contains("<workbench::SecondaryTools model=m/>"));
    assert!(!ui.contains("id=\"tool-notes\""));
    assert!(!ui.contains("<aside class=\"inspector\""));
    let commands = include_str!("../src/workbench_view.rs");
    assert!(commands.contains("文档操作"));
    assert!(commands.contains("Action::Prompt(PageAction::Move)"));
    assert!(!ui.contains("id=\"tool-ink\""));
    assert!(ui.contains("<workbench::InkRecovery model=m/>"));
    assert!(index.contains("href=\"workbench.css\""));
    assert!(!ui.contains("<nav class=\"commandbar\""));
}

#[test]
fn restored_editors_are_visible_before_transition_guards_lock_the_ui() {
    let initial = Chrome::default();
    let note = initial.document_opened(true, false, false);
    assert_eq!(note.panel, None);
    assert_eq!(note.active_tool, Some(ActiveTool::Note));
    assert_eq!(note.mode, Some(Mode::Annotation));
    let ink = initial.document_opened(false, true, false);
    assert_eq!(ink.panel, None);
    assert_eq!(ink.active_tool, Some(ActiveTool::Ink));
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
        ("fn select_mode(", "fn activate_tool("),
        ("fn activate_tool(", "fn select_panel("),
        ("fn select_panel(", "fn toggle_sidebar("),
        ("fn fit_page(", "fn run_strip_action("),
        ("fn fit_width(", "#[component]"),
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
