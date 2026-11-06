use super::*;
use std::{fs, path::PathBuf};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("recent-documents-v1.json");
    let pdf = dir.path().join("synthetic.pdf");
    fs::write(
        &pdf,
        Session::blank(100., 200., 1).unwrap().bytes().unwrap(),
    )
    .unwrap();
    (dir, store, pdf)
}

#[test]
fn recents_record_reopen_favorite_and_projection_are_private() {
    let (_dir, path, pdf) = fixture();
    let mut store = recents::RecentStore::load(path.clone());
    store.record(&pdf, 10).unwrap();
    let id = store.view().entries[0].id;
    store.toggle(id).unwrap();
    store.record(&pdf, 20).unwrap();
    let view = store.view();
    assert_eq!(view.entries.len(), 1);
    assert_eq!(view.entries[0].id, id);
    assert!(view.entries[0].favorite);
    assert_eq!(view.entries[0].last_opened, Some(20));
    let wire = serde_json::to_value(view).unwrap();
    assert_eq!(wire["entries"][0].as_object().unwrap().len(), 5);
    assert!(
        !wire
            .to_string()
            .contains(pdf.parent().unwrap().to_str().unwrap())
    );
    assert_eq!(recents::RecentStore::load(path).view().entries[0].id, id);
}

#[test]
fn recents_trim_keep_favorite_remove_clear_and_ids_never_reuse() {
    let (dir, path, pdf) = fixture();
    let original = fs::read(&pdf).unwrap();
    let mut store = recents::RecentStore::load(path);
    store.record(&pdf, 1).unwrap();
    let favorite = store.view().entries[0].id;
    store.toggle(favorite).unwrap();
    for i in 0..55 {
        let file = dir.path().join(format!("{i}.pdf"));
        fs::write(&file, &original).unwrap();
        store.record(&file, 10 + i).unwrap();
    }
    assert_eq!(store.view().entries.len(), 51);
    store.remove(favorite).unwrap();
    let item = store
        .view()
        .entries
        .into_iter()
        .find(|e| e.id == favorite)
        .unwrap();
    assert!(item.favorite);
    assert_eq!(item.last_opened, None);
    let previous_max = store.view().entries.iter().map(|e| e.id).max().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert!(store.clear_confirmed(&snapshot, false).unwrap().is_none());
    assert_eq!(store.view().entries.len(), 51);
    store.clear_confirmed(&snapshot, true).unwrap().unwrap();
    assert_eq!(store.view().entries.len(), 1);
    assert_eq!(fs::read(&pdf).unwrap(), original);
    let file = dir.path().join("new.pdf");
    fs::write(&file, original).unwrap();
    store.record(&file, 100).unwrap();
    assert!(store.view().entries.iter().any(|e| e.id > previous_max));
}

#[test]
fn recents_failed_write_malformed_unknown_and_stale_clear_preserve_state() {
    let (dir, path, pdf) = fixture();
    let mut store = recents::RecentStore::load(path.clone());
    store.record(&pdf, 1).unwrap();
    let id = store.view().entries[0].id;
    let old = fs::read(&path).unwrap();
    assert!(store.toggle(id + 100).is_err());
    assert_eq!(fs::read(&path).unwrap(), old);
    let snapshot = store.snapshot().unwrap();
    store.toggle(id).unwrap();
    assert!(store.clear_confirmed(&snapshot, true).is_err());
    // Replacing the destination with a directory must not publish a candidate.
    fs::rename(&path, dir.path().join("old.json")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.toggle(id).is_err());
    assert!(store.view().entries[0].favorite);
    assert_eq!(
        fs::read(dir.path().join("old.json")).unwrap(),
        serde_json::to_vec(&store.snapshot().unwrap()).unwrap()
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, b"broken").unwrap();
    let mut malformed = recents::RecentStore::load(path.clone());
    assert!(malformed.view().warning.is_some());
    assert!(malformed.record(&pdf, 2).is_err());
    assert_eq!(fs::read(path).unwrap(), b"broken");
}

#[test]
fn recents_source_dedup_preserves_dirty_session_and_independent_tabs() {
    let (dir, path, pdf) = fixture();
    let mut w = Workspace {
        recents: Some(recents::RecentStore::load(path)),
        ..Default::default()
    };
    w.journal = Some(recovery::Journal::new(dir.path().join("draft.json")));
    let first = w.open_source(&pdf).unwrap().id;
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let dirty = w.session.as_ref().unwrap().bytes().unwrap();
    w.replace_document(
        Session::blank(50., 50., 1).unwrap(),
        "independent.pdf".into(),
    )
    .unwrap();
    let view = w.open_source(&pdf).unwrap();
    assert_eq!(view.id, first);
    assert_eq!(view.tabs.len(), 2);
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), dirty);
    assert!(view.info.dirty);
    let old = w.recents.as_ref().unwrap().view().entries.len();
    let invalid = dir.path().join("bad.pdf");
    fs::write(&invalid, b"not PDF").unwrap();
    assert!(w.open_source(&invalid).is_err());
    assert!(w.open_source(&dir.path().join("missing.pdf")).is_err());
    assert_eq!(w.recents.as_ref().unwrap().view().entries.len(), old);
    let snapshot = w.prepare_close(first).unwrap();
    w.close_confirmed(&snapshot).unwrap();
    assert!(!w.source_documents.contains_key(&first));
}

#[test]
fn recents_schema_bounds_and_unknown_source_are_refused_without_overwrite() {
    let (_dir, path, pdf) = fixture();
    let mut store = recents::RecentStore::load(path.clone());
    store.record(&pdf, 1).unwrap();
    assert!(store.source(0).is_err());
    assert!(store.source(u32::MAX).is_err());
    let valid: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut invalid = vec![];
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("next_id", serde_json::json!(0)),
    ] {
        let mut candidate = valid.clone();
        candidate[field] = value;
        invalid.push(candidate);
    }
    let mut duplicate = valid.clone();
    duplicate["entries"]
        .as_array_mut()
        .unwrap()
        .push(valid["entries"][0].clone());
    invalid.push(duplicate);
    let mut zero_id = valid.clone();
    zero_id["entries"][0]["id"] = serde_json::json!(0);
    invalid.push(zero_id);
    let mut relative = valid;
    relative["entries"][0]["path"] = serde_json::json!("relative.pdf");
    invalid.push(relative);
    for value in invalid {
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &bytes).unwrap();
        let mut loaded = recents::RecentStore::load(path.clone());
        assert!(loaded.view().warning.is_some());
        assert!(loaded.record(&pdf, 3).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, vec![b' '; 16 * 1024 * 1024 + 1]).unwrap();
    assert!(
        recents::RecentStore::load(path.clone())
            .view()
            .warning
            .is_some()
    );
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(recents::RecentStore::load(path).view().warning.is_some());
}

#[cfg(unix)]
#[test]
fn recents_source_alias_dedup_and_metadata_symlink_are_private_and_non_destructive() {
    let (dir, path, pdf) = fixture();
    let alias = dir.path().join("alias.pdf");
    std::os::unix::fs::symlink(&pdf, &alias).unwrap();
    let mut store = recents::RecentStore::load(path.clone());
    store.record(&pdf, 1).unwrap();
    store.record(&alias, 2).unwrap();
    assert_eq!(store.view().entries.len(), 1);
    let bytes = fs::read(&path).unwrap();
    let target = dir.path().join("original.json");
    fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(store.toggle(1).is_err());
    assert!(!store.view().entries[0].favorite);
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert!(recents::RecentStore::load(path).view().warning.is_some());
}

#[test]
fn recents_metadata_failure_keeps_successful_open_and_sticky_recovery_warning() {
    let (dir, path, pdf) = fixture();
    fs::write(&path, b"broken metadata").unwrap();
    let mut w = Workspace {
        recents: Some(recents::RecentStore::load(path.clone())),
        journal: Some(recovery::Journal::new(dir.path().join("draft.json"))),
        recovery_warning: Some("既有恢复警告".into()),
        ..Default::default()
    };
    let view = w.open_source(&pdf).unwrap();
    assert_eq!(view.info.page_count, 1);
    assert!(view.warning.as_deref().unwrap().contains("既有恢复警告"));
    assert!(
        view.warning
            .as_deref()
            .unwrap()
            .contains("最近记录存储不可用")
    );
    assert_eq!(w.recovery_warning.as_deref(), Some("既有恢复警告"));
    assert_eq!(fs::read(path).unwrap(), b"broken metadata");
}
