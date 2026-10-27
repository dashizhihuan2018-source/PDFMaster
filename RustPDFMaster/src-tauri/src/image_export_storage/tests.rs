use super::*;
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

fn fixture_pdf() -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::new();
    for content in [Vec::new(), b"1 0 0 rg 10 10 30 20 re f\n".to_vec()] {
        let content_id = document.add_object(Stream::new(Dictionary::new(), content));
        page_ids.push(document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 60.into()],
            "Resources" => Dictionary::new(),
            "Contents" => content_id,
        }));
    }
    document.objects.insert(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }
        .into(),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(unix)]
#[test]
fn stages_two_selected_png_pages_and_publishes_one_complete_new_directory() {
    let source = fixture_pdf();
    let session = pdfmaster_core::Session::open(&source).unwrap();
    let before_bytes = session.bytes().unwrap();
    let before_info = session.info();
    let parent = tempfile::tempdir().unwrap();
    let control = ImageExportControl::new();
    let request = pdfmaster_core::ImageExportRequest {
        pages: vec![1, 0, 1],
        ..Default::default()
    };

    let stage = PreparedImageExport::render_in(
        parent.path(),
        std::ffi::OsStr::new("images"),
        &session,
        &request,
        &control,
    );
    assert!(stage.is_ok(), "a valid export must stage complete images");
    let stage = stage.unwrap();
    let payload = stage.path().to_path_buf();
    assert_ne!(payload, parent.path().join("images"));
    assert_eq!(stage.summary().pages, 2);
    let mut names = std::fs::read_dir(&payload)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            std::ffi::OsString::from("page-0001.png"),
            std::ffi::OsString::from("page-0002.png")
        ]
    );

    let published = stage.publish_new(&parent.path().join("images")).unwrap();
    assert_eq!(published.pages, 2);
    assert_eq!(
        published.total_bytes,
        published_directory_bytes(&parent.path().join("images"))
    );
    assert!(published.durability_warning.is_none());
    assert!(control.is_published());
    assert!(
        !control.cancel(),
        "a completed publication cannot become cancelled"
    );
    assert_eq!(session.bytes().unwrap(), before_bytes);
    assert_eq!(session.info(), before_info);
}

fn staged(
    parent: &tempfile::TempDir,
    session: &pdfmaster_core::Session,
    control: &ImageExportControl,
) -> PreparedImageExport {
    PreparedImageExport::render_in(
        parent.path(),
        std::ffi::OsStr::new("images"),
        session,
        &pdfmaster_core::ImageExportRequest {
            pages: vec![1, 0, 1],
            ..Default::default()
        },
        control,
    )
    .unwrap()
}

fn assert_session_unchanged(
    session: &pdfmaster_core::Session,
    bytes: &[u8],
    info: &pdfmaster_core::DocumentInfo,
) {
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(&session.info(), info);
}

#[test]
fn invalid_parent_basename_and_mismatched_targets_fail_without_public_output() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let parent = tempfile::tempdir().unwrap();
    for invalid in [
        "",
        ".",
        "..",
        "nested/images",
        "/absolute",
        "bad\nname",
        "bad\0name",
        "bad\u{0085}name",
    ] {
        let control = ImageExportControl::new();
        assert!(
            PreparedImageExport::render_in(
                parent.path(),
                std::ffi::OsStr::new(invalid),
                &session,
                &pdfmaster_core::ImageExportRequest {
                    pages: vec![0],
                    ..Default::default()
                },
                &control,
            )
            .is_err(),
            "invalid basename accepted: {invalid:?}"
        );
    }
    let overlong = "x".repeat(256);
    assert!(
        PreparedImageExport::render_in(
            parent.path(),
            std::ffi::OsStr::new(&overlong),
            &session,
            &pdfmaster_core::ImageExportRequest {
                pages: vec![0],
                ..Default::default()
            },
            &ImageExportControl::new(),
        )
        .is_err()
    );
    let missing = parent.path().join("missing");
    let missing_error = PreparedImageExport::render_in(
        &missing,
        std::ffi::OsStr::new("images"),
        &session,
        &pdfmaster_core::ImageExportRequest {
            pages: vec![0],
            ..Default::default()
        },
        &ImageExportControl::new(),
    )
    .err()
    .expect("an absent parent must fail");
    assert!(!missing_error.contains(missing.to_string_lossy().as_ref()));

    let other = tempfile::tempdir().unwrap();
    let stage = staged(&parent, &session, &ImageExportControl::new());
    assert!(stage.publish_new(&other.path().join("images")).is_err());
    assert!(!other.path().join("images").exists());
    let stage = staged(&parent, &session, &ImageExportControl::new());
    assert!(
        stage
            .publish_new(&parent.path().join("other-name"))
            .is_err()
    );
    assert!(!parent.path().join("other-name").exists());
}

#[test]
fn cancellation_before_render_and_after_staging_prevent_publication() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let before = session.bytes().unwrap();
    let before_info = session.info();

    let cancelled_parent = tempfile::tempdir().unwrap();
    let cancelled = ImageExportControl::new();
    assert!(cancelled.cancel());
    assert!(cancelled.is_cancelled());
    assert!(
        PreparedImageExport::render_in(
            cancelled_parent.path(),
            std::ffi::OsStr::new("images"),
            &session,
            &pdfmaster_core::ImageExportRequest {
                pages: vec![0],
                ..Default::default()
            },
            &cancelled,
        )
        .is_err()
    );
    assert!(!cancelled_parent.path().join("images").exists());
    assert_eq!(
        std::fs::read_dir(cancelled_parent.path()).unwrap().count(),
        0
    );

    let cancel_after_parent = tempfile::tempdir().unwrap();
    let cancel_after = ImageExportControl::new();
    let stage = staged(&cancel_after_parent, &session, &cancel_after);
    let captured_clone = cancel_after.clone();
    assert!(captured_clone.cancel());
    assert!(
        stage
            .publish_new(&cancel_after_parent.path().join("images"))
            .is_err()
    );
    assert!(!cancel_after_parent.path().join("images").exists());
    assert_eq!(
        std::fs::read_dir(cancel_after_parent.path())
            .unwrap()
            .count(),
        0
    );

    assert_session_unchanged(&session, &before, &before_info);
}

#[test]
fn cancel_waits_while_publish_owns_the_gate_then_observes_published() {
    use std::{
        sync::mpsc::{self, TryRecvError},
        thread,
    };

    let control = ImageExportControl::new();
    let publishing = control.clone();
    let cancelling = control.clone();
    let (rename_entered_tx, rename_entered_rx) = mpsc::sync_channel(0);
    let (release_rename_tx, release_rename_rx) = mpsc::sync_channel(0);
    let (publish_result_tx, publish_result_rx) = mpsc::sync_channel(0);
    let publisher = thread::spawn(move || {
        let result = publishing.publish(|| {
            rename_entered_tx.send(()).unwrap();
            release_rename_rx.recv().unwrap();
            Ok(())
        });
        publish_result_tx.send(result).unwrap();
    });
    rename_entered_rx.recv().unwrap();

    let (cancel_calling_tx, cancel_calling_rx) = mpsc::sync_channel(0);
    let (cancel_result_tx, cancel_result_rx) = mpsc::sync_channel(0);
    let canceller = thread::spawn(move || {
        cancel_calling_tx.send(()).unwrap();
        cancel_result_tx.send(cancelling.cancel()).unwrap();
    });
    cancel_calling_rx.recv().unwrap();
    assert!(matches!(
        cancel_result_rx.try_recv(),
        Err(TryRecvError::Empty)
    ));

    release_rename_tx.send(()).unwrap();
    assert!(publish_result_rx.recv().unwrap().is_ok());
    assert!(!cancel_result_rx.recv().unwrap());
    publisher.join().unwrap();
    canceller.join().unwrap();
    assert!(control.is_published());
    assert!(!control.is_cancelled());
}

#[test]
fn cancel_first_skips_rename_and_rename_failure_leaves_gate_active() {
    let cancel_first = ImageExportControl::new();
    assert!(cancel_first.cancel());
    let mut rename_called = false;
    let cancelled = cancel_first.publish(|| {
        rename_called = true;
        Ok(())
    });
    assert!(matches!(
        cancelled,
        Err(control::PublicationGateError::Cancelled)
    ));
    assert!(!rename_called);

    let rename_failure = ImageExportControl::new();
    let failed = rename_failure.publish(|| {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected rename failure",
        ))
    });
    assert!(matches!(
        failed,
        Err(control::PublicationGateError::Io(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied
    ));
    assert!(!rename_failure.is_cancelled());
    assert!(!rename_failure.is_published());
    assert!(rename_failure.publish(|| Ok(())).is_ok());
    assert!(rename_failure.is_published());
}

#[cfg(windows)]
#[test]
fn unsupported_windows_payload_directory_sync_fails_closed_before_rename() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let control = ImageExportControl::new();
    let target = parent.path().join("images");

    let error = staged(&parent, &session, &control)
        .publish_new(&target)
        .err()
        .expect("unsupported Windows directory sync must prevent publication");

    assert!(error.contains("Unsupported"));
    assert!(!target.exists());
    assert!(!control.is_published());
    assert!(control.cancel());
}

#[test]
fn render_failure_and_every_existing_target_kind_never_overwrite() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let before = session.bytes().unwrap();
    let before_info = session.info();
    let parent = tempfile::tempdir().unwrap();
    let bad = PreparedImageExport::render_in(
        parent.path(),
        std::ffi::OsStr::new("invalid-export"),
        &session,
        &pdfmaster_core::ImageExportRequest::default(),
        &ImageExportControl::new(),
    );
    assert!(bad.is_err());
    assert!(!parent.path().join("invalid-export").exists());
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);

    for empty in [true, false] {
        let case = tempfile::tempdir().unwrap();
        let target = case.path().join("images");
        std::fs::create_dir(&target).unwrap();
        if !empty {
            std::fs::write(target.join("keep"), b"keep").unwrap();
        }
        let control = ImageExportControl::new();
        assert!(
            staged(&case, &session, &control)
                .publish_new(&target)
                .is_err()
        );
        assert!(!control.is_published());
        if !empty {
            assert_eq!(std::fs::read(target.join("keep")).unwrap(), b"keep");
        }
    }

    let file_case = tempfile::tempdir().unwrap();
    let target = file_case.path().join("images");
    std::fs::write(&target, b"keep").unwrap();
    assert!(
        staged(&file_case, &session, &ImageExportControl::new())
            .publish_new(&target)
            .is_err()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");

    let late_case = tempfile::tempdir().unwrap();
    let stage = staged(&late_case, &session, &ImageExportControl::new());
    let late_target = late_case.path().join("images");
    std::fs::create_dir(&late_target).unwrap();
    assert!(stage.publish_new(&late_target).is_err());
    assert!(late_target.is_dir());

    #[cfg(unix)]
    {
        let link_case = tempfile::tempdir().unwrap();
        let retained = link_case.path().join("retained");
        std::fs::create_dir(&retained).unwrap();
        let link = link_case.path().join("images");
        std::os::unix::fs::symlink(&retained, &link).unwrap();
        assert!(
            staged(&link_case, &session, &ImageExportControl::new())
                .publish_new(&link)
                .is_err()
        );
        assert!(retained.is_dir());
    }
    assert_session_unchanged(&session, &before, &before_info);
}

#[test]
fn exact_readback_rejects_changed_truncated_extra_directory_and_symlink_entries() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let stage_parent = tempfile::tempdir().unwrap();
    let stage = staged(&stage_parent, &session, &ImageExportControl::new());

    let copy = stage.disposable_copy_in(stage_parent.path()).unwrap();
    stage.verify_copy(copy.path()).unwrap();
    let changed_path = copy.path().join("page-0001.png");
    let mut changed = std::fs::read(&changed_path).unwrap();
    changed[0] ^= 0xff;
    std::fs::write(&changed_path, changed).unwrap();
    assert!(stage.verify_copy(copy.path()).is_err());
    assert!(stage.path().join("page-0001.png").is_file());

    let copy = stage.disposable_copy_in(stage_parent.path()).unwrap();
    let page = copy.path().join("page-0001.png");
    let file = std::fs::OpenOptions::new().write(true).open(&page).unwrap();
    file.set_len(1).unwrap();
    assert!(stage.verify_copy(copy.path()).is_err());

    let copy = stage.disposable_copy_in(stage_parent.path()).unwrap();
    std::fs::write(copy.path().join("extra.png"), b"extra").unwrap();
    assert!(stage.verify_copy(copy.path()).is_err());

    let copy = stage.disposable_copy_in(stage_parent.path()).unwrap();
    std::fs::remove_file(copy.path().join("page-0002.png")).unwrap();
    assert!(stage.verify_copy(copy.path()).is_err());

    let copy = stage.disposable_copy_in(stage_parent.path()).unwrap();
    std::fs::remove_file(copy.path().join("page-0002.png")).unwrap();
    std::fs::create_dir(copy.path().join("page-0002.png")).unwrap();
    assert!(stage.verify_copy(copy.path()).is_err());

    #[cfg(unix)]
    {
        let copy = stage.disposable_copy_in(stage_parent.path()).unwrap();
        std::fs::remove_file(copy.path().join("page-0002.png")).unwrap();
        std::os::unix::fs::symlink(
            stage.path().join("page-0002.png"),
            copy.path().join("page-0002.png"),
        )
        .unwrap();
        assert!(stage.verify_copy(copy.path()).is_err());
    }
}

#[test]
fn disposable_copy_is_independent_and_each_private_root_has_private_unix_modes() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let stage = staged(&parent, &session, &ImageExportControl::new());
    let stage_path = stage.path().to_path_buf();
    let disposable = stage.disposable_copy_in(cache.path()).unwrap();
    let disposable_path = disposable.path().to_path_buf();
    assert_ne!(stage_path, disposable_path);
    stage.verify_copy(&disposable_path).unwrap();
    drop(disposable);
    assert!(!disposable_path.exists());
    assert!(stage_path.join("page-0001.png").is_file());

    let dropped = staged(&parent, &session, &ImageExportControl::new());
    let dropped_path = dropped.path().to_path_buf();
    drop(dropped);
    assert!(!dropped_path.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(stage_path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&stage_path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(stage_path.join("page-0001.png"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn completed_publication_survives_injected_parent_sync_failure_as_warning() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let control = ImageExportControl::new();
    let target = parent.path().join("images");
    let publication = staged(&parent, &session, &control)
        .publish_new_with_sync(&target, |_| {
            Err(std::io::Error::other("injected sync failure"))
        })
        .unwrap();
    assert!(publication.durability_warning.is_some());
    assert!(control.is_published());
    assert!(!control.cancel());
    assert!(target.join("page-0001.png").is_file());
    assert!(target.join("page-0002.png").is_file());
}

#[test]
fn payload_directory_sync_failure_prevents_rename_and_publication() {
    let session = pdfmaster_core::Session::open(&fixture_pdf()).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let control = ImageExportControl::new();
    let target = parent.path().join("images");
    let result = staged(&parent, &session, &control).publish_new_with_syncs(
        &target,
        |_| Err(std::io::Error::other("injected payload sync failure")),
        |_| Ok(()),
    );
    assert!(result.is_err());
    assert!(!target.exists());
    assert!(!control.is_published());
    assert!(control.cancel());
}

#[cfg(unix)]
#[test]
fn retained_parent_identity_rejects_a_replacement_at_the_same_canonical_path() {
    let container = tempfile::tempdir().unwrap();
    let chosen = container.path().join("chosen");
    let moved = container.path().join("moved");
    std::fs::create_dir(&chosen).unwrap();
    let retained = platform::ParentDirectory::capture(&chosen).unwrap();
    std::fs::rename(&chosen, &moved).unwrap();
    std::fs::create_dir(&chosen).unwrap();
    assert!(!retained.matches(&chosen).unwrap());
}

#[cfg(unix)]
fn published_directory_bytes(path: &Path) -> usize {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len() as usize)
        .sum()
}
