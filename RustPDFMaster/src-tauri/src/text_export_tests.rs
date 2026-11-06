use super::*;
use crate::text_export::{self, Target};
use pdfmaster_core::TextExportFormat as F;

fn workspace() -> Workspace {
    Workspace {
        session: Some(Session::blank(200., 120., 1).unwrap()),
        active_document_id: 7,
        document_epoch: 9,
        ..Default::default()
    }
}

#[test]
fn text_export_snapshot_ownership_staleness_and_cancel() {
    let mut w = workspace();
    let job = text_export::begin(&mut w).unwrap();
    assert!(w.begin_export().is_err());
    assert!(text_export::validate_current(&w, &job).is_ok());
    w.document_epoch += 1;
    assert!(text_export::validate_current(&w, &job).is_err());
    w.document_epoch -= 1;
    w.active_document_id += 1;
    assert!(text_export::validate_current(&w, &job).is_err());
    w.active_document_id -= 1;
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(text_export::validate_current(&w, &job).is_err());
    w.release_export(job.id);
    let next = text_export::begin(&mut w).unwrap();
    w.release_export(job.id);
    assert_eq!(w.export_reservation, Some(next.id));
    w.release_export(next.id);
    assert_eq!(w.export_reservation, None);
    let store = Store::new(workspace());
    let job = text_export::begin(&mut store.lock().unwrap()).unwrap();
    let id = job.id;
    let selected = text_export::with_picker(
        job,
        || store.lock().unwrap().release_export(id),
        || Ok(None),
    )
    .unwrap();
    assert!(selected.is_none());
    assert_eq!(store.lock().unwrap().export_reservation, None);
}

#[test]
fn text_export_publication_new_replacement_changed_and_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("摘要.txt");
    let target = Target::capture(&path, F::SummaryText, &[]).unwrap();
    let receipt = target.publish(F::SummaryText, "中文", || Ok(())).unwrap();
    assert_eq!(receipt.total_bytes, 6);
    assert_eq!(receipt.file_name, "摘要.txt");
    assert!(
        !serde_json::to_string(&receipt)
            .unwrap()
            .contains(&dir.path().to_string_lossy().to_string())
    );
    let target = Target::capture(&path, F::SummaryText, &[]).unwrap();
    target.publish(F::SummaryText, "", || Ok(())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    let target = Target::capture(&path, F::SummaryText, &[]).unwrap();
    std::fs::write(&path, b"parallel").unwrap();
    assert!(
        target
            .publish(F::SummaryText, "replace", || Ok(()))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"parallel");
    let target = Target::capture(&path, F::SummaryText, &[]).unwrap();
    assert!(
        target
            .publish(F::SummaryText, "replace", || Err("stale".into()))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"parallel");
    let new = dir.path().join("new.txt");
    let target = Target::capture(&new, F::PlainText, &[]).unwrap();
    std::fs::write(&new, b"unexpected").unwrap();
    assert!(target.publish(F::PlainText, "replace", || Ok(())).is_err());
    assert_eq!(std::fs::read(&new).unwrap(), b"unexpected");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn text_export_rejects_sources_extension_links_and_editor() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.txt");
    std::fs::write(&source, b"PDF source").unwrap();
    assert!(Target::capture(&source, F::PlainText, std::slice::from_ref(&source)).is_err());
    assert!(Target::capture(&dir.path().join("source.pdf"), F::PlainText, &[]).is_err());
    assert!(Target::capture(dir.path(), F::PlainText, &[]).is_err());
    #[cfg(unix)]
    {
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(Target::capture(&link, F::PlainText, &[]).is_err());
        let hard = dir.path().join("hard.txt");
        std::fs::hard_link(&source, &hard).unwrap();
        assert!(Target::capture(&hard, F::PlainText, std::slice::from_ref(&source)).is_err());
    }
    let mut w = workspace();
    w.ink_editor = Some(InkEditor {
        id: 1,
        request: "test".into(),
        page: 0,
        color: pdfmaster_core::Color {
            r: 0.,
            g: 0.,
            b: 0.,
        },
        line_width: 1.,
        document_epoch: 9,
        snapshot: vec![],
    });
    assert!(text_export::begin(&mut w).is_err());
    assert_eq!(w.export_reservation, None);
}

#[test]
fn text_export_revalidates_target_after_document_validation_and_cleans_staging() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.txt");
    std::fs::write(&path, b"confirmed").unwrap();
    let target = Target::capture(&path, F::PlainText, &[]).unwrap();
    let result = target.publish(F::PlainText, "new", || {
        std::fs::write(&path, b"parallel edit").unwrap();
        Ok(())
    });
    assert!(
        result.is_err(),
        "a target changed during final document validation must not be replaced"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"parallel edit");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn text_export_worker_guard_stays_owned_until_worker_drains_on_dropped_receiver() {
    let store = std::sync::Arc::new(Store::new(workspace()));
    let job = text_export::begin(&mut store.lock().unwrap()).unwrap();
    let id = job.id;
    let cleanup_store = store.clone();
    let (_job, _path, guard) = text_export::with_picker(
        job,
        move || cleanup_store.lock().unwrap().release_export(id),
        || Ok(Some(std::path::PathBuf::from("synthetic.txt"))),
    )
    .unwrap()
    .unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (drain_tx, drain_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        let _guard = guard;
        ready_tx.send(()).unwrap();
        drain_rx.recv().unwrap();
        let _ = result_tx.send(());
    });
    ready_rx.recv().unwrap();
    drop(result_rx);
    assert_eq!(store.lock().unwrap().export_reservation, Some(id));
    assert!(store.lock().unwrap().begin_export().is_err());
    drain_tx.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(store.lock().unwrap().export_reservation, None);
}

#[test]
fn text_export_picker_failure_and_worker_panic_release_only_owned_token() {
    let store = Store::new(workspace());
    let job = text_export::begin(&mut store.lock().unwrap()).unwrap();
    let id = job.id;
    let error = text_export::with_picker(
        job,
        || store.lock().unwrap().release_export(id),
        || Err("native picker failed".into()),
    );
    assert!(error.is_err());
    assert_eq!(store.lock().unwrap().export_reservation, None);
    let job = text_export::begin(&mut store.lock().unwrap()).unwrap();
    let id = job.id;
    let (_job, _path, guard) = text_export::with_picker(
        job,
        || store.lock().unwrap().release_export(id),
        || Ok(Some(std::path::PathBuf::from("synthetic.txt"))),
    )
    .unwrap()
    .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = guard;
            panic!("synthetic worker panic");
        }))
        .is_err()
    );
    assert_eq!(store.lock().unwrap().export_reservation, None);
}

#[test]
fn text_export_replacement_refuses_equal_bytes_different_identity_and_keeps_source_links() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    let target_path = dir.path().join("output.txt");
    std::fs::write(&source, b"source bytes").unwrap();
    std::fs::hard_link(&source, &target_path).unwrap();
    // An unrelated hardlink replacement is atomic; it never opens/truncates
    // the underlying inode, including if the source identity wasn't supplied.
    Target::capture(&target_path, F::PlainText, &[])
        .unwrap()
        .publish(F::PlainText, "text", || Ok(()))
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
    assert_eq!(std::fs::read(&target_path).unwrap(), b"text");
    let target = Target::capture(&target_path, F::PlainText, &[]).unwrap();
    let old = dir.path().join("moved.txt");
    std::fs::rename(&target_path, &old).unwrap();
    std::fs::write(&target_path, b"text").unwrap();
    assert!(target.publish(F::PlainText, "changed", || Ok(())).is_err());
    assert_eq!(std::fs::read(&target_path).unwrap(), b"text");
}

#[cfg(windows)]
#[test]
fn text_export_external_target_handle_fails_safely_then_closed_target_retries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.txt");
    std::fs::write(&path, b"original").unwrap();
    let external = std::fs::File::open(&path).unwrap();
    let target = Target::capture(&path, F::PlainText, &[]).unwrap();
    assert!(
        target
            .publish(F::PlainText, "replacement", || Ok(()))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    drop(external);
    Target::capture(&path, F::PlainText, &[])
        .unwrap()
        .publish(F::PlainText, "replacement", || Ok(()))
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn text_export_real_worker_serialization_preserves_dirty_and_inactive_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = workspace();
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    w.inactive_documents.push(tabs::Parked {
        id: 6,
        session: Session::blank(100., 100., 1).unwrap(),
        name: "inactive.pdf".into(),
        restored: true,
    });
    w.recovery_warning = Some("sticky synthetic warning".into());
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let inactive = w.inactive_documents[0].session.bytes().unwrap();
    let job = text_export::begin(&mut w).unwrap();
    let path = dir.path().join("empty.txt");
    let receipt = text_export::run_job(&job, &path, F::PlainText, || {
        text_export::validate_current(&w, &job)
    })
    .unwrap();
    assert_eq!(receipt.total_bytes, 0);
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(w.inactive_documents[0].session.bytes().unwrap(), inactive);
    assert!(w.inactive_documents[0].restored);
    assert_eq!(
        w.recovery_warning.as_deref(),
        Some("sticky synthetic warning")
    );
    assert_eq!(w.export_reservation, Some(job.id));
    w.release_export(job.id);
    assert_eq!(w.export_reservation, None);
}
