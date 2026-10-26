use pdfmaster_ui::shell::{HomeSection, Route, document_reply_route, switch_route};

#[test]
fn home_navigation_keeps_document_available_without_replacing_it() {
    let home = Route::Home(HomeSection::Home);
    assert_eq!(switch_route(Route::Document, home, true, false), home);
    assert_eq!(
        switch_route(home, Route::Document, true, false),
        Route::Document
    );
    assert_eq!(switch_route(home, Route::Document, false, false), home);
}

#[test]
fn unresolved_editor_or_busy_command_cannot_be_hidden() {
    for section in HomeSection::ALL {
        assert_eq!(
            switch_route(Route::Document, Route::Home(section), true, true),
            Route::Document
        );
    }
}

#[test]
fn only_successful_document_acquisition_enters_reader() {
    let tools = Route::Home(HomeSection::Tools);
    for command in ["open_document", "new_document", "document_info"] {
        assert_eq!(document_reply_route(tools, command, true), Route::Document);
        assert_eq!(document_reply_route(tools, command, false), tools);
    }
    assert_eq!(document_reply_route(tools, "export_document", true), tools);
}

#[test]
fn original_sidebar_order_and_titles_are_preserved() {
    assert_eq!(
        HomeSection::ALL.map(|s| s.title()),
        [
            "主界面",
            "工具",
            "无边记",
            "无限画布",
            "最近",
            "收藏",
            "账号详情",
            "设置",
            "获取支持"
        ]
    );
}

#[test]
fn catalog_preserves_all_original_groups_and_recommended_order() {
    use pdfmaster_ui::shell::{RECOMMENDED, TOOLS, tool};
    assert_eq!(TOOLS.len(), 28);
    assert_eq!(
        RECOMMENDED,
        ["edit", "annotate", "pages", "measure", "sign", "copilot"]
    );
    assert_eq!(
        (0..5)
            .map(|g| TOOLS.iter().filter(|t| t.group == g).count())
            .collect::<Vec<_>>(),
        [6, 6, 8, 4, 4]
    );
    assert_eq!(tool("blank").unwrap().title, "空白 PDF");
    assert!(tool("not-a-tool").is_none());
}

#[test]
fn unsupported_tools_never_route_to_unrelated_workbench_actions() {
    use pdfmaster_ui::shell::tool;
    for id in [
        "edit",
        "sign",
        "ocr",
        "copilot",
        "measure",
        "password",
        "redact",
        "translate",
        "pdf-to-word",
    ] {
        assert!(tool(id).unwrap().workbench_target.is_none());
    }
    assert_eq!(tool("draw").unwrap().workbench_target, Some("tool-ink"));
    assert_eq!(tool("pages").unwrap().workbench_target, Some("tool-pages"));
}

#[test]
fn mobile_hidden_reader_uses_visible_shell_width_not_a_one_pixel_fit() {
    use pdfmaster_ui::shell::reader_fit_available;
    assert_eq!(reader_fit_available(0, Some(390), true), Some(350.));
    assert_eq!(reader_fit_available(0, Some(350), true), Some(310.));
    assert_eq!(reader_fit_available(390, Some(500), true), Some(350.));
    assert_eq!(reader_fit_available(0, Some(1280), false), None);
    assert_eq!(reader_fit_available(0, None, true), None);
    assert_eq!(reader_fit_available(0, Some(40), true), None);
}
