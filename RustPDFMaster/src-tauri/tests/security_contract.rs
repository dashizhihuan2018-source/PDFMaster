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
