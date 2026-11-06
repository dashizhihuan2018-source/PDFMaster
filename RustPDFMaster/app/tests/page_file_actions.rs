#[test]
fn actual_page_file_commands_have_no_parameter_inspector() {
    let workspace = include_str!("../src/workspace.rs");
    assert!(
        !workspace.contains("<aside class=\"inspector\""),
        "generic parameter inspector must be physically removed"
    );
    let view = include_str!("../src/workbench_view.rs");
    assert!(view.contains("移动当前页…"));
    assert!(view.contains("文档操作"));
    assert!(!workspace.contains("page_range:"));
}

#[test]
fn page_operations_use_explicit_native_wire_values() {
    use pdfmaster_ui::workbench::PageAction;
    for (operation, wire) in [
        (PageAction::Extract, "extract"),
        (PageAction::RotateRange, "rotate_range"),
        (PageAction::DeleteRange, "delete_range"),
        (PageAction::Move, "move"),
    ] {
        assert_eq!(
            serde_json::to_value(operation).unwrap(),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<PageAction>(serde_json::json!(wire)).unwrap(),
            operation
        );
    }
    assert!(serde_json::from_value::<PageAction>(serde_json::json!("generic_panel")).is_err());
}
