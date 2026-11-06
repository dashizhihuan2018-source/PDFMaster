use super::*;
use crate::image_export::{self, ImageExportJob};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use pdfmaster_core::{ImageExportFormat, Session};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

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

fn fixture_workspace() -> Workspace {
    Workspace {
        session: Some(Session::open(&fixture_pdf()).unwrap()),
        active_document_id: 7,
        next_document_id: 7,
        document_epoch: 11,
        name: "Fixture.pdf".into(),
        ..Default::default()
    }
}

fn prepared(format: Option<ImageExportFormat>) -> (Workspace, image_export::Handle) {
    let mut workspace = fixture_workspace();
    let handle = image_export::prepare(&mut workspace, format).unwrap();
    (workspace, handle)
}

fn claimed(format: Option<ImageExportFormat>) -> (Workspace, ImageExportJob) {
    let (mut workspace, handle) = prepared(format);
    let job = image_export::claim(&mut workspace, handle.id).unwrap();
    (workspace, job)
}

#[test]
fn prepare_reserves_without_changing_dirty_document() {
    let mut workspace = fixture_workspace();
    workspace.session.as_mut().unwrap().mark_saved();
    workspace
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
    let info = workspace.session.as_ref().unwrap().info();
    let handle = image_export::prepare(&mut workspace, None).unwrap();
    assert_eq!(workspace.export_reservation, Some(handle.id));
    assert!(workspace.ensure_editor_idle().is_err());
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), bytes);
    assert_eq!(workspace.session.as_ref().unwrap().info(), info);
}

#[test]
fn no_active_pdf_creates_no_reservation() {
    let mut workspace = Workspace::default();
    assert!(image_export::prepare(&mut workspace, None).is_err());
    assert_eq!(workspace.export_reservation, None);
    assert!(workspace.image_export.task.is_none());
}

#[test]
fn prepared_cancel_releases_and_stale_run_fails() {
    let (mut workspace, handle) = prepared(None);
    assert!(image_export::cancel(&mut workspace, handle.id));
    assert_eq!(workspace.export_reservation, None);
    assert!(workspace.image_export.task.is_none());
    assert!(image_export::claim(&mut workspace, handle.id).is_err());
    assert!(!image_export::cancel(&mut workspace, handle.id));
}

#[test]
fn run_cannot_claim_twice() {
    let (mut workspace, handle) = prepared(None);
    let _job = image_export::claim(&mut workspace, handle.id).unwrap();
    assert!(image_export::claim(&mut workspace, handle.id).is_err());
    assert_eq!(workspace.export_reservation, Some(handle.id));
}

#[test]
fn running_cancel_retains_reservation_and_prevents_output() {
    let (mut workspace, job) = claimed(None);
    let id = job.id();
    assert!(image_export::cancel(&mut workspace, id));
    assert_eq!(workspace.export_reservation, Some(id));
    assert!(workspace.image_export.task.is_some());
    let parent = tempfile::tempdir().unwrap();
    assert!(image_export::run_job(&job, parent.path(), || Ok(())).is_err());
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    workspace.release_export(id);
    assert_eq!(workspace.export_reservation, None);
    assert!(workspace.image_export.task.is_none());
}

#[test]
fn stale_old_cancel_and_release_cannot_affect_newer_task() {
    let (mut workspace, old) = prepared(None);
    assert!(image_export::cancel(&mut workspace, old.id));
    let new = image_export::prepare(&mut workspace, None).unwrap();
    workspace.release_export(old.id);
    assert!(!image_export::cancel(&mut workspace, old.id));
    assert_eq!(workspace.export_reservation, Some(new.id));
    assert_eq!(workspace.image_export.task.as_ref().unwrap().id, new.id);
}

#[test]
fn changed_token_document_epoch_or_bytes_rejects_publication() {
    for change in 0..4 {
        let (mut workspace, job) = claimed(None);
        match change {
            0 => workspace.export_reservation = Some(job.id() + 1),
            1 => workspace.active_document_id += 1,
            2 => workspace.document_epoch += 1,
            3 => {
                workspace.session = Some(Session::blank(90., 90., 2).unwrap());
            }
            _ => unreachable!(),
        }
        let parent = tempfile::tempdir().unwrap();
        let result = image_export::run_job(&job, parent.path(), || {
            image_export::validate_current(&workspace, &job)
        });
        assert!(result.is_err(), "change case {change} published");
        assert!(!parent.path().join(job.folder_name()).exists());
    }
}

#[test]
fn exact_two_page_png_and_jpeg_output_preserves_source() {
    for (format, extension) in [
        (ImageExportFormat::Png, "png"),
        (ImageExportFormat::Jpeg, "jpg"),
    ] {
        let (workspace, job) = claimed(Some(format));
        let before = workspace.session.as_ref().unwrap().bytes().unwrap();
        let info = workspace.session.as_ref().unwrap().info();
        let parent = tempfile::tempdir().unwrap();
        let receipt = image_export::run_job(&job, parent.path(), || {
            image_export::validate_current(&workspace, &job)
        })
        .unwrap();
        assert_eq!(receipt.id, job.id());
        assert_eq!(receipt.pages, 2);
        assert!(receipt.total_bytes > 0);
        assert_eq!(receipt.folder_name, "Fixture-images");
        assert!(
            parent
                .path()
                .join("Fixture-images")
                .join(format!("page-0001.{extension}"))
                .is_file()
        );
        assert!(
            parent
                .path()
                .join("Fixture-images")
                .join(format!("page-0002.{extension}"))
                .is_file()
        );
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(workspace.session.as_ref().unwrap().info(), info);
    }
}

#[test]
fn preexisting_nonempty_output_is_retained() {
    let (workspace, job) = claimed(None);
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join(job.folder_name());
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"keep").unwrap();
    assert!(
        image_export::run_job(&job, parent.path(), || {
            image_export::validate_current(&workspace, &job)
        })
        .is_err()
    );
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn injected_validation_failure_leaves_no_published_output() {
    let (_workspace, job) = claimed(None);
    let parent = tempfile::tempdir().unwrap();
    let result = image_export::run_job(&job, parent.path(), || {
        Err("injected validation failure".into())
    });
    assert!(result.is_err());
    assert!(!parent.path().join(job.folder_name()).exists());
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn portable_folder_name_never_traverses_or_uses_windows_reserved_stems() {
    for (name, expected) in [
        ("../evil.pdf", "evil-images"),
        ("CON.pdf", "document-images"),
        ("a<b>:c?.pdf", "a-b--c--images"),
        (".pdf", "document-images"),
    ] {
        let mut workspace = fixture_workspace();
        workspace.name = name.into();
        let handle = image_export::prepare(&mut workspace, None).unwrap();
        let job = image_export::claim(&mut workspace, handle.id).unwrap();
        assert_eq!(job.folder_name(), expected);
        assert!(!job.folder_name().contains(['/', '\\']));
    }
}

#[test]
fn adapter_admission_rejects_stale_and_duplicate_before_picker_or_cleanup() {
    let store = Mutex::new(fixture_workspace());
    let handle = {
        let mut workspace = store.lock().unwrap();
        image_export::prepare(&mut workspace, None).unwrap()
    };
    let picker_calls = Arc::new(AtomicUsize::new(0));
    let rejected_cleanup_calls = Arc::new(AtomicUsize::new(0));
    let picker_counter = picker_calls.clone();
    let mut accepted = image_export::admit_with_picker(
        handle.id,
        |id| image_export::claim(&mut store.lock().unwrap(), id),
        |action| image_export::cleanup(&mut store.lock().unwrap(), handle.id, action),
        move || {
            picker_counter.fetch_add(1, Ordering::SeqCst);
            Ok(Some(std::path::PathBuf::from("accepted-parent")))
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(picker_calls.load(Ordering::SeqCst), 1);

    for rejected in [handle.id, handle.id + 99] {
        let picker_counter = picker_calls.clone();
        let cleanup_counter = rejected_cleanup_calls.clone();
        assert!(
            image_export::admit_with_picker(
                rejected,
                |id| image_export::claim(&mut store.lock().unwrap(), id),
                move |_| {
                    cleanup_counter.fetch_add(1, Ordering::SeqCst);
                },
                move || {
                    picker_counter.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(std::path::PathBuf::from("must-not-open")))
                },
            )
            .is_err()
        );
    }
    assert_eq!(picker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(rejected_cleanup_calls.load(Ordering::SeqCst), 0);
    assert!(!accepted.job.is_cancelled());
    assert_eq!(store.lock().unwrap().export_reservation, Some(handle.id));
    accepted.guard.disarm();
    store.lock().unwrap().release_export(handle.id);
}

#[test]
fn adapter_picker_cancel_releases_admitted_reservation_before_worker() {
    let store = Mutex::new(fixture_workspace());
    let handle = {
        let mut workspace = store.lock().unwrap();
        image_export::prepare(&mut workspace, None).unwrap()
    };
    let result = image_export::admit_with_picker(
        handle.id,
        |id| image_export::claim(&mut store.lock().unwrap(), id),
        |action| image_export::cleanup(&mut store.lock().unwrap(), handle.id, action),
        || Ok(None),
    )
    .unwrap();
    assert!(result.is_none());
    let workspace = store.lock().unwrap();
    assert_eq!(workspace.export_reservation, None);
    assert!(workspace.image_export.task.is_none());
}

#[test]
fn adapter_worker_transfer_makes_outer_drop_cancel_without_early_release() {
    let store = Mutex::new(fixture_workspace());
    let handle = {
        let mut workspace = store.lock().unwrap();
        image_export::prepare(&mut workspace, None).unwrap()
    };
    let mut admission = image_export::admit_with_picker(
        handle.id,
        |id| image_export::claim(&mut store.lock().unwrap(), id),
        |action| image_export::cleanup(&mut store.lock().unwrap(), handle.id, action),
        || Ok(Some(std::path::PathBuf::from("accepted-parent"))),
    )
    .unwrap()
    .unwrap();
    let worker_job = admission.job.clone();
    admission.guard.transfer_to_worker();
    drop(admission);
    assert!(worker_job.is_cancelled());
    assert_eq!(store.lock().unwrap().export_reservation, Some(handle.id));
    store.lock().unwrap().release_export(handle.id);
    assert_eq!(store.lock().unwrap().export_reservation, None);
}

#[test]
fn unsupported_adapter_error_releases_own_prepared_task_but_not_stale_or_duplicate() {
    let store = Mutex::new(fixture_workspace());
    let first = {
        let mut workspace = store.lock().unwrap();
        image_export::prepare(&mut workspace, None).unwrap()
    };
    let error = image_export::reject_unsupported_after_admission(
        first.id,
        |id| image_export::claim(&mut store.lock().unwrap(), id),
        |action| image_export::cleanup(&mut store.lock().unwrap(), first.id, action),
    )
    .unwrap_err();
    assert!(error.contains("Windows"));
    assert_eq!(store.lock().unwrap().export_reservation, None);

    let second = {
        let mut workspace = store.lock().unwrap();
        image_export::prepare(&mut workspace, None).unwrap()
    };
    let accepted = {
        let mut workspace = store.lock().unwrap();
        image_export::claim(&mut workspace, second.id).unwrap()
    };
    for rejected in [first.id, second.id, second.id + 99] {
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let cleanup_counter = cleanup_calls.clone();
        let error = image_export::reject_unsupported_after_admission(
            rejected,
            |id| image_export::claim(&mut store.lock().unwrap(), id),
            move |_| {
                cleanup_counter.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap_err();
        assert!(error.contains("Windows"));
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 0);
        assert!(!accepted.is_cancelled());
        assert_eq!(store.lock().unwrap().export_reservation, Some(second.id));
    }
    store.lock().unwrap().release_export(second.id);
}
