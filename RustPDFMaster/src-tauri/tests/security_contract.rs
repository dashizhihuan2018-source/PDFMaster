use serde_json::Value;

#[test]
fn unified_annotation_deletion_uses_the_native_confirmation_and_snapshot_path() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("async fn delete_annotation_document("));
    let body = source
        .split("async fn delete_annotation_document(")
        .nth(1)
        .unwrap()
        .split("#[tauri::command")
        .next()
        .unwrap();
    assert!(body.contains("prepare_annotation_delete("));
    assert!(body.contains("dialog.show("));
    assert!(body.contains("receiver"));
    assert!(body.contains("return Ok(None)"));
    assert!(body.contains("apply_confirmed_annotation_delete("));
    let handlers = source
        .rsplit_once(".invoke_handler(tauri::generate_handler![")
        .unwrap()
        .1;
    assert!(handlers.contains("delete_annotation_document"));
}

#[test]
fn wasm_can_load_only_from_the_packaged_origin() {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let csp = config["app"]["security"]["csp"].as_str().unwrap();
    let connect = csp
        .split(';')
        .map(str::trim)
        .find(|s| s.starts_with("connect-src "))
        .unwrap();
    assert!(
        connect.split_whitespace().any(|s| s == "'self'"),
        "WASM fetch needs its packaged origin"
    );
    assert!(!connect.contains('*'));
    assert!(!connect.contains("https:"));
    assert!(!csp.contains("'unsafe-eval'"));
    assert!(csp.contains("object-src 'none'"));
}

#[test]
fn windows_publication_uses_write_through_move_with_explicit_replace_policy() {
    let storage = include_str!("../src/storage.rs");
    let recovery = include_str!("../src/recovery.rs");
    assert!(storage.contains("MoveFileExW"));
    assert!(storage.contains("MOVEFILE_WRITE_THROUGH"));
    assert!(storage.contains("move_file_write_through(&temp, &target, false)"));
    assert!(recovery.contains("move_file_write_through(&staged_path, &self.path, true)"));
}

#[test]
fn ios_readback_requires_scope_and_coordinates_only_the_staged_move_on_a_worker() {
    let source = include_str!("../src/ios_export.rs");
    assert!(source.contains("PreparedExport::new(&cache, name, bytes)"));
    let constructor = source
        .split("let picker = UIDocumentPickerViewController::initForExportingURLs_asCopy(")
        .nth(1)
        .unwrap()
        .split(");")
        .next()
        .unwrap();
    assert!(constructor.contains("false,"));
    assert!(!constructor.contains("true,"));
    assert!(source.contains("if !scoped"));
    assert!(source.contains("stopAccessingSecurityScopedResource"));
    assert!(source.contains("coordinateReadingItemAtURL_options_error_byAccessor"));
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("coordinated_url.path()"));
    assert!(source.contains("std::thread::Builder::new()"));
    assert!(source.contains("verify_completion(true, Some(&bytes), &expected)"));
    assert!(!source.contains("File::open"));
}

#[test]
fn all_native_exit_paths_include_editor_state_and_history_export_require_idle_editor() {
    // Source-wiring regression, complementary to real Workspace state tests.
    let source = include_str!("../src/lib.rs");
    let cocoa = include_str!("../src/macos_termination.rs");
    assert!(cocoa.contains("workspace.has_unsaved_work()"));
    assert!(source.matches("|w| w.has_unsaved_work()").count() >= 1);
    assert!(source.contains("|w|w.has_unsaved_work()"));
    for command in ["undo_document", "redo_document", "export_document"] {
        let body = source
            .split(&format!("async fn {command}("))
            .nth(1)
            .unwrap()
            .split("#[tauri::command")
            .next()
            .unwrap();
        assert!(body.contains("ensure_editor_idle()?"), "{command}");
    }
}
