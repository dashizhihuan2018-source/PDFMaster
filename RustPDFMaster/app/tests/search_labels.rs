use pdfmaster_ui::search_result_label;

#[test]
fn search_row_matches_original_physical_page_label_and_excerpt_contract() {
    assert_eq!(
        search_result_label(2, "附录-iv", "找到的片段"),
        "第3页 [附录-iv]\n找到的片段"
    );
    assert_eq!(
        search_result_label(0, "", "blank label"),
        "第1页 []\nblank label"
    );
    assert_eq!(search_result_label(0, "1", "first"), "第1页 [1]\nfirst");
}

#[test]
fn response_labels_are_rendered_and_keyed_instead_of_retaining_old_hit_text() {
    let model = include_str!("../src/workspace.rs");
    let sidebar = include_str!("../src/thumbnail_view.rs");
    assert!(model.contains("page_label: String,"));
    assert!(
        model.contains(
            "#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]\nstruct SearchHit"
        )
    );
    assert!(sidebar.contains("key=|(index,hit)|(*index,hit.clone())"));
    assert!(sidebar.contains("search_result_label(hit.page_index,&hit.page_label,&hit.snippet)"));
    assert!(!sidebar.contains("结果暂不含PDF页标签"));
}
