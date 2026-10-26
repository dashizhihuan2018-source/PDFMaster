use serde_json::Value;

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
