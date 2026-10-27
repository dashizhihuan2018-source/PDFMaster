mod export_staging;
#[cfg(target_os = "ios")]
mod ios_export;
#[cfg(target_os = "macos")]
mod macos_termination;
mod recovery;
mod search;
mod storage;
use base64::{Engine, engine::general_purpose::STANDARD};
use pdfmaster_core::{DocumentInfo, Edit, PageRange, Session};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path, sync::Mutex};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct Workspace {
    session: Option<Session>,
    document_epoch: u64,
    name: String,
    journal: Option<recovery::Journal>,
    _journal_lock: Option<File>,
    restored: bool,
    recovery_fault: bool,
    recovery_warning: Option<String>,
    note_editor: Option<NoteEditor>,
    ink_editor: Option<InkEditor>,
    next_ink_id: u32,
    finished_ink: Option<(u32, u64, bool)>,
    cancelled_ink_requests: std::collections::BTreeSet<String>,
    next_editor_id: u32,
    export_reservation: Option<u32>,
    next_export_id: u32,
}
#[derive(Clone, Serialize)]
struct NoteEditor {
    id: u32,
    page: usize,
    original: pdfmaster_core::TextNote,
}
struct InkEditor {
    id: u32,
    request: String,
    page: usize,
    color: pdfmaster_core::Color,
    line_width: f32,
    document_epoch: u64,
    snapshot: Vec<u8>,
}
#[derive(Clone, Serialize)]
struct InkEditorView {
    id: u32,
    request: String,
    page: usize,
    color: pdfmaster_core::Color,
    line_width: f32,
}
impl InkEditor {
    fn view(&self) -> InkEditorView {
        InkEditorView {
            id: self.id,
            request: self.request.clone(),
            page: self.page,
            color: self.color,
            line_width: self.line_width,
        }
    }
}
#[derive(Clone, Serialize)]
struct DocumentView {
    name: String,
    info: DocumentInfo,
    restored: bool,
    warning: Option<String>,
    note_editor: Option<NoteEditor>,
    ink_editor: Option<InkEditorView>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
enum PageBatchOperation {
    #[serde(rename = "rotate_90")]
    Rotate90,
    #[serde(rename = "delete")]
    Delete,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PageBatchReply {
    Cancelled,
    ConfirmationRequired {
        count: usize,
        page_count: usize,
    },
    Applied {
        document: Box<DocumentView>,
        affected_count: usize,
    },
}
type Store = Mutex<Workspace>;
type Reply<T> = Result<T, String>;

// Never hold the workspace mutex across a picker/provider await. This scope
// keeps mutations mutually exclusive and releases only its own token on every
// return path, including cancellation, failure and future cancellation.
struct ExportReservation<'a> {
    store: &'a Store,
    id: u32,
}
impl Drop for ExportReservation<'_> {
    fn drop(&mut self) {
        if let Ok(mut workspace) = self.store.lock() {
            workspace.release_export(self.id);
        }
    }
}

fn workspace_notes(workspace: &Workspace, page: usize) -> Reply<pdfmaster_core::NoteList> {
    workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .notes(page)
        .map_err(|error| error.to_string())
}

fn workspace_shapes(workspace: &Workspace, page: usize) -> Reply<pdfmaster_core::ShapeList> {
    workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .shapes(page)
        .map_err(|error| error.to_string())
}

fn workspace_annotations(workspace: &Workspace) -> Reply<Vec<pdfmaster_core::AnnotationRow>> {
    workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .annotations()
        .map_err(|error| error.to_string())
}

fn read_insertion_files(paths: &[std::path::PathBuf]) -> Reply<Vec<Vec<u8>>> {
    if paths.is_empty() || paths.len() > pdfmaster_core::MAX_INSERT_FILES {
        return Err("请选择 1–32 份 PDF；整批未修改当前文档。".into());
    }
    let mut declared = 0u64;
    for path in paths {
        let metadata =
            std::fs::metadata(path).map_err(|_| "无法检查所选 PDF；整批未修改当前文档。")?;
        if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
            return Err("所选 PDF 须为普通文件且每份不超过 32MiB；整批未修改当前文档。".into());
        }
        declared = declared
            .checked_add(metadata.len())
            .ok_or("批量 PDF 总输入超过 64MiB；文档未修改。")?;
        if declared > pdfmaster_core::MAX_INSERT_BYTES as u64 {
            return Err("批量 PDF 总输入超过 64MiB；文档未修改。".into());
        }
    }
    let mut files = Vec::with_capacity(paths.len());
    let mut actual = 0usize;
    for path in paths {
        // Metadata is only a preflight. The opened handle and actual read size
        // are checked again; a changed file cannot bypass the aggregate bound.
        let bytes = export_staging::readback_pdf(path)
            .map_err(|_| "无法读取所选 PDF；整批未修改当前文档。")?;
        actual = actual
            .checked_add(bytes.len())
            .ok_or("批量 PDF 总输入超过 64MiB；文档未修改。")?;
        if actual > pdfmaster_core::MAX_INSERT_BYTES {
            return Err("批量 PDF 总输入超过 64MiB；文档未修改。".into());
        }
        files.push(bytes);
    }
    Ok(files)
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    #[test]
    fn ink_request_cancel_orders_before_or_after_begin_without_releasing_a_newer_stroke() {
        let (mut w, _) = note_workspace();
        let color = pdfmaster_core::Color {
            r: 1.,
            g: 0.,
            b: 0.,
        };
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        w.cancel_ink_request("unsettled-before").unwrap();
        assert!(w.begin_ink("unsettled-before", 0, color, 4.).is_err());
        let first = w.begin_ink("unsettled-after", 0, color, 4.).unwrap();
        w.cancel_ink_request("unsettled-after").unwrap();
        assert!(w.commit_ink(first.id, vec![]).is_err());
        let next = w.begin_ink("current-stroke", 0, color, 4.).unwrap();
        assert!(w.cancel_ink_request("unsettled-after").is_err());
        assert_eq!(w.ink_editor.as_ref().unwrap().id, next.id);
        w.cancel_ink_request("current-stroke").unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }
    #[test]
    fn ink_request_cancel_after_lost_commit_ack_preserves_saved_stroke_and_refuses_late_begin() {
        let (mut w, _) = note_workspace();
        let e = w
            .begin_ink(
                "commit-lost-ack",
                0,
                pdfmaster_core::Color {
                    r: 0.,
                    g: 0.,
                    b: 1.,
                },
                2.,
            )
            .unwrap();
        w.commit_ink(
            e.id,
            vec![vec![
                pdfmaster_core::Point { x: 10., y: 10. },
                pdfmaster_core::Point { x: 80., y: 80. },
            ]],
        )
        .unwrap();
        let changed = w.session.as_ref().unwrap().bytes().unwrap();
        w.cancel_ink_request("commit-lost-ack").unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), changed);
        assert!(
            w.begin_ink(
                "commit-lost-ack",
                0,
                pdfmaster_core::Color {
                    r: 0.,
                    g: 0.,
                    b: 1.
                },
                2.
            )
            .is_err()
        );
    }
    #[test]
    fn ink_guard_blocks_close_mutation_export_and_commits_once_with_checkpoint_and_undo() {
        let (mut w, directory) = note_workspace();
        w.session.as_mut().unwrap().mark_saved();
        w.checkpoint().unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let e = w
            .begin_ink(
                "guard-test",
                0,
                pdfmaster_core::Color {
                    r: 1.,
                    g: 0.,
                    b: 0.,
                },
                4.,
            )
            .unwrap();
        assert!(w.has_unsaved_work());
        assert!(w.clean().is_err());
        assert!(w.begin_export().is_err());
        assert!(w.begin_note_editor(0, 0, "原便笺 🦀").is_err());
        assert!(
            apply_single_edit(
                &mut w,
                Edit::Rotate {
                    page: 0,
                    degrees: 90
                }
            )
            .is_err()
        );
        let points = vec![vec![
            pdfmaster_core::Point { x: 10., y: 10. },
            pdfmaster_core::Point { x: 80., y: 80. },
        ]];
        let v = w.commit_ink(e.id, points.clone()).unwrap();
        assert!(v.info.dirty);
        assert!(v.ink_editor.is_none());
        assert!(directory.path().join("draft.json").is_file());
        let changed = w.session.as_ref().unwrap().bytes().unwrap();
        w.commit_ink(e.id, points).unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), changed);
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }
    #[test]
    fn ink_cancel_bad_geometry_and_stale_ids_cannot_lose_or_replace_current_stroke() {
        let (mut w, _) = note_workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let color = pdfmaster_core::Color {
            r: 1.,
            g: 0.,
            b: 0.,
        };
        let a = w.begin_ink("first-test", 0, color, 4.).unwrap();
        assert!(w.commit_ink(a.id, vec![vec![]]).is_err());
        assert!(w.ink_editor.is_some());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        w.cancel_ink(a.id).unwrap();
        w.cancel_ink(a.id).unwrap();
        let b = w.begin_ink("second-test", 0, color, 4.).unwrap();
        assert!(w.cancel_ink(a.id).is_err());
        assert!(w.commit_ink(a.id, vec![]).is_err());
        assert_eq!(w.ink_editor.as_ref().unwrap().id, b.id);
        w.cancel_ink(b.id).unwrap();
        assert!(w.commit_ink(b.id, vec![]).is_err());
    }
    #[test]
    fn shape_edits_use_shared_journal_history_and_editor_reservation_guards() {
        let (mut w, directory) = note_workspace();
        w.checkpoint().unwrap();
        let request = serde_json::json!({"kind":"add_shape","page":0,"shape":"rectangle","bounds":{"x":10.,"y":10.,"width":50.,"height":40.},"stroke":{"r":0.,"g":0.,"b":1.},"fill":null,"line_width":2.});
        let edit: Edit = serde_json::from_value(request).unwrap();
        let original = w.session.as_ref().unwrap().bytes().unwrap();
        let reservation = w.begin_insertion(1).unwrap();
        assert!(apply_single_edit(&mut w, edit.clone()).is_err());
        w.release_export(reservation);
        let view = apply_single_edit(&mut w, edit).unwrap();
        assert!(view.info.dirty);
        assert!(view.warning.is_none());
        assert_eq!(
            w.session
                .as_ref()
                .unwrap()
                .notes(0)
                .unwrap()
                .unsupported_count,
            1
        );
        assert!(directory.path().join("draft.json").is_file());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
    }
    #[test]
    fn batch_insertion_checkpoints_once_and_keeps_late_failure_atomic() {
        let (mut w, directory) = note_workspace();
        w.checkpoint().unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let first = Session::blank(120., 100., 2).unwrap().bytes().unwrap();
        let second = Session::blank(150., 100., 1).unwrap().bytes().unwrap();
        let journal_before = std::fs::read(directory.path().join("draft.json")).unwrap();
        let id = w.begin_insertion(1).unwrap();
        assert!(
            w.finish_insertions(id, 1, &[first.as_slice(), b"bad PDF"])
                .is_err()
        );
        assert!(w.export_reservation.is_none());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(
            std::fs::read(directory.path().join("draft.json")).unwrap(),
            journal_before
        );
        let id = w.begin_insertion(1).unwrap();
        let view = w.finish_insertions(id, 1, &[&first, &second]).unwrap();
        assert_eq!(view.info.page_count, 4);
        assert!(view.info.dirty);
        assert!(view.warning.is_none());
        assert_eq!(view.info.pages[1].width, 120.);
        assert_eq!(view.info.pages[3].width, 150.);
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }

    #[test]
    fn native_batch_reader_keeps_returned_order_and_rejects_missing_files_or_count() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("中文 first.pdf");
        let second = directory.path().join("second.pdf");
        let a = Session::blank(120., 100., 1).unwrap().bytes().unwrap();
        let b = Session::blank(150., 100., 2).unwrap().bytes().unwrap();
        std::fs::write(&first, &a).unwrap();
        std::fs::write(&second, &b).unwrap();
        assert_eq!(
            read_insertion_files(&[second.clone(), first.clone(), second.clone()]).unwrap(),
            vec![b.clone(), a.clone(), b.clone()]
        );
        assert!(read_insertion_files(&[]).is_err());
        assert!(read_insertion_files(&vec![first.clone(); 33]).is_err());
        assert!(
            read_insertion_files(&[first.clone(), directory.path().join("missing.pdf")]).is_err()
        );
        assert!(read_insertion_files(&[directory.path().to_path_buf()]).is_err());
        assert_eq!(std::fs::read(&first).unwrap(), a);
        assert_eq!(std::fs::read(&second).unwrap(), b);
    }

    #[test]
    fn native_batch_reader_rejects_aggregate_size_before_reading_payloads() {
        let directory = tempfile::tempdir().unwrap();
        let large = directory.path().join("large.pdf");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(32 * 1024 * 1024)
            .unwrap();
        let tiny = directory.path().join("tiny.pdf");
        std::fs::write(&tiny, b"x").unwrap();
        assert!(
            read_insertion_files(&[large.clone(), large.clone(), tiny])
                .unwrap_err()
                .contains("64MiB")
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(&large)
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        assert!(read_insertion_files(&[large]).is_err());
    }

    #[test]
    fn insertion_is_reserved_checkpointed_and_one_undo_transaction() {
        let (mut w, directory) = note_workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let incoming = Session::blank(150., 200., 2).unwrap().bytes().unwrap();
        let id = w.begin_insertion(1).unwrap();
        assert!(w.clean().is_err());
        assert!(w.begin_export().is_err());
        assert!(w.begin_note_editor(0, 0, "原便笺 🦀").is_err());
        let view = w.finish_insertion(id, 1, &incoming).unwrap();
        assert_eq!(view.info.page_count, 3);
        assert!(view.info.dirty);
        assert!(view.warning.is_none());
        assert!(directory.path().join("draft.json").is_file());
        assert!(w.export_reservation.is_none());
        assert!(w.has_unsaved_work());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }

    #[test]
    fn insertion_cancel_invalid_and_stale_tokens_preserve_current_document() {
        let (w, _directory) = note_workspace();
        let store = Mutex::new(w);
        let before = store
            .lock()
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .bytes()
            .unwrap();
        assert!(store.lock().unwrap().begin_insertion(2).is_err());
        let first = store.lock().unwrap().begin_insertion(0).unwrap();
        {
            let _scope = ExportReservation {
                store: &store,
                id: first,
            };
        }
        let mut w = store.lock().unwrap();
        assert!(!w.has_unsaved_work());
        let second = w.begin_insertion(0).unwrap();
        assert!(w.finish_insertion(first, 0, b"bad").is_err());
        assert_eq!(w.export_reservation, Some(second));
        assert!(w.finish_insertion(second, 0, b"bad").is_err());
        assert!(w.export_reservation.is_none());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(!w.has_unsaved_work());
    }

    #[test]
    fn insertion_keeps_applied_bytes_and_warning_if_checkpoint_fails() {
        let (mut w, _directory) = note_workspace();
        let incoming = Session::blank(150., 200., 1).unwrap().bytes().unwrap();
        let id = w.begin_insertion(0).unwrap();
        w.journal = None;
        let view = w.finish_insertion(id, 0, &incoming).unwrap();
        assert_eq!(view.info.page_count, 2);
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        assert!(w.clean().is_err());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().info().page_count, 1);
    }
    #[test]
    fn extraction_exports_a_new_document_without_acknowledging_source_or_recovery() {
        let (mut w, directory) = note_workspace();
        w.name = "测试原文.pdf".into();
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::InsertBlank {
                index: 1,
                width: 100.,
                height: 100.,
            })
            .unwrap();
        w.checkpoint().unwrap();
        w.restored = true;
        w.recovery_fault = true;
        w.recovery_warning = Some("保留恢复警告".into());
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let journal = std::fs::read(directory.path().join("draft.json")).unwrap();
        let (id, name, bytes) = w.begin_extraction("2").unwrap();
        assert_eq!(name, "测试原文-提取页面.pdf");
        assert_eq!(Session::open(&bytes).unwrap().info().page_count, 1);
        assert!(w.begin_export().is_err());
        assert!(w.begin_extraction("1").is_err());
        assert!(w.clean().is_err());
        let view = w.finish_extraction(id).unwrap();
        assert!(view.info.dirty);
        assert!(view.restored);
        assert_eq!(view.warning.as_deref(), Some("保留恢复警告"));
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(
            std::fs::read(directory.path().join("draft.json")).unwrap(),
            journal
        );
        assert!(w.export_reservation.is_none());
        assert!(w.has_unsaved_work());
    }

    #[test]
    fn extraction_cancel_invalid_range_and_stale_completion_preserve_source() {
        let (w, _directory) = note_workspace();
        let store = Mutex::new(w);
        let before = store
            .lock()
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .bytes()
            .unwrap();
        assert!(store.lock().unwrap().begin_extraction("0").is_err());
        assert!(store.lock().unwrap().export_reservation.is_none());
        let (first, _, _) = store.lock().unwrap().begin_extraction("1").unwrap();
        {
            let _scope = ExportReservation {
                store: &store,
                id: first,
            };
        }
        assert!(!store.lock().unwrap().has_unsaved_work());
        let mut w = store.lock().unwrap();
        let (second, _, _) = w.begin_extraction("1").unwrap();
        assert!(w.finish_extraction(first).is_err());
        assert_eq!(w.export_reservation, Some(second));
        let view = w.finish_extraction(second).unwrap();
        assert!(!view.info.dirty);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert!(w.begin_extraction("1").is_err());
        assert_eq!(w.note_editor.as_ref().unwrap().id, editor.id);
    }
    #[test]
    fn exhausted_document_epoch_never_replaces_the_current_session() {
        let (mut w, _directory) = note_workspace();
        w.document_epoch = u64::MAX;
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let name = w.name.clone();
        assert!(
            w.replace_document(Session::blank(100., 100., 2).unwrap(), "new.pdf".into())
                .is_err()
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.name, name);
        assert_eq!(w.document_epoch, u64::MAX);
    }

    #[test]
    fn page_confirmation_cannot_delete_an_identical_but_reopened_document() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        };
        let snapshot = w.session.as_ref().unwrap().bytes().unwrap();
        let epoch = w.document_epoch;
        w.replace_document(Session::open(&snapshot).unwrap(), "another.pdf".into())
            .unwrap();
        assert!(apply_confirmed_delete(&mut w, "1", 1, &snapshot, epoch).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), snapshot);
    }

    #[test]
    fn unified_sidebar_delete_cannot_bypass_native_confirmation() {
        let (mut workspace, _directory) = note_workspace();
        let expected = workspace
            .session
            .as_ref()
            .unwrap()
            .annotations()
            .unwrap()
            .remove(0);
        let bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
        let info = workspace.session.as_ref().unwrap().info();
        assert!(
            apply_single_edit(
                &mut workspace,
                Edit::DeleteAnnotation {
                    expected: Box::new(expected)
                }
            )
            .is_err()
        );
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(workspace.session.as_ref().unwrap().info(), info);
    }

    #[test]
    fn unified_annotation_confirmation_preflight_is_readonly_and_preserves_exact_undo() {
        let (mut workspace, _directory) = note_workspace();
        let expected = workspace
            .session
            .as_ref()
            .unwrap()
            .annotations()
            .unwrap()
            .remove(0);
        let before = workspace.session.as_ref().unwrap().bytes().unwrap();
        let info = workspace.session.as_ref().unwrap().info();
        let selected = prepare_annotation_delete(&workspace, expected.clone()).unwrap();
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(workspace.session.as_ref().unwrap().info(), info);
        let mut stale = expected;
        stale.contents.push('!');
        assert!(prepare_annotation_delete(&workspace, stale).is_err());
        let view = apply_confirmed_annotation_delete(&mut workspace, &selected).unwrap();
        assert!(view.info.dirty);
        assert!(
            workspace
                .session
                .as_ref()
                .unwrap()
                .annotations()
                .unwrap()
                .is_empty()
        );
        assert!(apply_confirmed_annotation_delete(&mut workspace, &selected).is_err());
        workspace.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(workspace.session.as_ref().unwrap().info().can_redo);
    }

    #[test]
    fn unified_annotation_confirmation_rejects_editor_export_mutation_and_reopen() {
        let (mut workspace, _directory) = note_workspace();
        let expected = workspace
            .session
            .as_ref()
            .unwrap()
            .annotations()
            .unwrap()
            .remove(0);
        let selected = prepare_annotation_delete(&workspace, expected.clone()).unwrap();
        let editor = workspace.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert!(prepare_annotation_delete(&workspace, expected.clone()).is_err());
        assert!(apply_confirmed_annotation_delete(&mut workspace, &selected).is_err());
        workspace.cancel_note_editor(editor.id).unwrap();
        let (export, _, _) = workspace.begin_export().unwrap();
        assert!(prepare_annotation_delete(&workspace, expected).is_err());
        assert!(apply_confirmed_annotation_delete(&mut workspace, &selected).is_err());
        workspace.release_export(export);
        apply_single_edit(
            &mut workspace,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let changed = workspace.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_confirmed_annotation_delete(&mut workspace, &selected).is_err());
        assert_eq!(
            workspace.session.as_ref().unwrap().bytes().unwrap(),
            changed
        );
        workspace.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(
            workspace.session.as_ref().unwrap().bytes().unwrap(),
            selected.snapshot
        );
        workspace
            .replace_document(
                Session::open(&selected.snapshot).unwrap(),
                "identical.pdf".into(),
            )
            .unwrap();
        assert!(apply_confirmed_annotation_delete(&mut workspace, &selected).is_err());
        assert_eq!(
            workspace.session.as_ref().unwrap().bytes().unwrap(),
            selected.snapshot
        );
    }

    #[test]
    fn unified_annotation_checkpoint_failure_retains_deleted_state_and_warns() {
        let (mut workspace, directory) = note_workspace();
        workspace.journal = Some(recovery::Journal::new(
            directory.path().join("missing/draft.json"),
        ));
        let expected = workspace
            .session
            .as_ref()
            .unwrap()
            .annotations()
            .unwrap()
            .remove(0);
        let selected = prepare_annotation_delete(&workspace, expected).unwrap();
        let view = apply_confirmed_annotation_delete(&mut workspace, &selected).unwrap();
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        assert!(workspace.has_unsaved_work());
        assert!(
            workspace
                .session
                .as_ref()
                .unwrap()
                .annotations()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn note_confirmation_cannot_delete_an_identical_but_reopened_document() {
        let (mut w, _directory) = note_workspace();
        let selected = prepare_note_delete(&w, 0, 0, "原便笺 🦀").unwrap();
        let identical = Session::open(&selected.snapshot).unwrap();
        w.replace_document(identical, "another.pdf".into()).unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_confirmed_note_delete(&mut w, &selected).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }

    #[test]
    fn note_deletion_requires_a_native_snapshot_and_cannot_use_generic_edit() {
        let (mut w, _directory) = note_workspace();
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let selected = prepare_note_delete(&w, 0, 0, "原便笺 🦀").unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert!(prepare_note_delete(&w, 0, 0, "stale").is_err());
        assert!(
            apply_single_edit(
                &mut w,
                Edit::DeleteNote {
                    page: 0,
                    annotation_index: 0,
                    expected_contents: "原便笺 🦀".into()
                }
            )
            .is_err()
        );
        let view = apply_confirmed_note_delete(&mut w, &selected).unwrap();
        assert!(view.info.dirty);
        assert!(workspace_notes(&w, 0).unwrap().notes.is_empty());
        assert!(apply_confirmed_note_delete(&mut w, &selected).is_err());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(w.session.as_ref().unwrap().info().can_undo, info.can_undo);
        assert!(w.session.as_ref().unwrap().info().can_redo);
    }

    #[test]
    fn note_delete_rejects_changed_document_editor_and_export_without_mutation() {
        let (mut w, _directory) = note_workspace();
        let selected = prepare_note_delete(&w, 0, 0, "原便笺 🦀").unwrap();
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert!(prepare_note_delete(&w, 0, 0, "原便笺 🦀").is_err());
        assert!(apply_confirmed_note_delete(&mut w, &selected).is_err());
        w.cancel_note_editor(editor.id).unwrap();
        let (export, _, _) = w.begin_export().unwrap();
        assert!(prepare_note_delete(&w, 0, 0, "原便笺 🦀").is_err());
        assert!(apply_confirmed_note_delete(&mut w, &selected).is_err());
        w.release_export(export);
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let changed = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_confirmed_note_delete(&mut w, &selected).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), changed);
        assert_eq!(
            workspace_notes(&w, 0).unwrap().notes[0].contents,
            "原便笺 🦀"
        );
    }

    #[test]
    fn confirmed_note_delete_preserves_dirty_state_when_checkpoint_fails() {
        let (mut w, directory) = note_workspace();
        w.journal = Some(recovery::Journal::new(
            directory.path().join("missing/draft.json"),
        ));
        let selected = prepare_note_delete(&w, 0, 0, "原便笺 🦀").unwrap();
        let view = apply_confirmed_note_delete(&mut w, &selected).unwrap();
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        assert!(w.has_unsaved_work());
        assert!(workspace_notes(&w, 0).unwrap().notes.is_empty());
    }

    fn shape_workspace() -> (Workspace, tempfile::TempDir) {
        let (mut w, directory) = note_workspace();
        let edit = serde_json::from_value(serde_json::json!({"kind":"add_shape","page":0,"shape":"ellipse","bounds":{"x":10.,"y":10.,"width":60.,"height":50.},"stroke":{"r":0.,"g":0.,"b":0.},"fill":null,"line_width":2.})).unwrap();
        apply_single_edit(&mut w, edit).unwrap();
        w.session.as_mut().unwrap().mark_saved();
        (w, directory)
    }

    #[test]
    fn shape_delete_requires_snapshot_and_cannot_bypass_native_confirmation() {
        let (mut w, _directory) = shape_workspace();
        let expected = workspace_shapes(&w, 0).unwrap().shapes[0].clone();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let selected = prepare_shape_delete(&w, 0, expected.clone()).unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert!(apply_single_edit(&mut w, Edit::DeleteShape { page: 0, expected }).is_err());
        let view = apply_confirmed_shape_delete(&mut w, &selected).unwrap();
        assert!(view.info.dirty);
        assert!(workspace_shapes(&w, 0).unwrap().shapes.is_empty());
        assert_eq!(workspace_notes(&w, 0).unwrap().notes.len(), 1);
        assert!(apply_confirmed_shape_delete(&mut w, &selected).is_err());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }

    #[test]
    fn line_edits_checkpoint_and_reuse_native_delete_epoch_snapshot_and_undo() {
        let (mut w, directory) = note_workspace();
        let original = w.session.as_ref().unwrap().bytes().unwrap();
        let edit=serde_json::from_value(serde_json::json!({"kind":"add_line","page":0,"start":{"x":20.,"y":20.},"end":{"x":80.,"y":80.},"endings":["none","open_arrow"],"stroke":{"r":1.,"g":0.,"b":0.},"fill":null,"line_width":4.})).unwrap();
        assert!(apply_single_edit(&mut w, edit).unwrap().info.dirty);
        assert!(directory.path().join("draft.json").is_file());
        let expected = workspace_shapes(&w, 0).unwrap().shapes[0].clone();
        assert!(expected.line.is_some());
        let selected = prepare_shape_delete(&w, 0, expected.clone()).unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_single_edit(&mut w, Edit::DeleteShape { page: 0, expected }).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        apply_confirmed_shape_delete(&mut w, &selected).unwrap();
        assert!(workspace_shapes(&w, 0).unwrap().shapes.is_empty());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        // Reopening is allowed only after the current synthetic revision is saved.
        w.session.as_mut().unwrap().mark_saved();
        w.checkpoint().unwrap();
        w.replace_document(Session::open(&before).unwrap(), "reopened.pdf".into())
            .unwrap();
        assert!(apply_confirmed_shape_delete(&mut w, &selected).is_err());
        assert_eq!(workspace_shapes(&w, 0).unwrap().shapes.len(), 1);
        assert_ne!(original, before);
    }

    #[test]
    fn shape_delete_rejects_reopened_changed_exporting_and_editing_documents() {
        let (mut w, _directory) = shape_workspace();
        let expected = workspace_shapes(&w, 0).unwrap().shapes[0].clone();
        let selected = prepare_shape_delete(&w, 0, expected.clone()).unwrap();
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert!(prepare_shape_delete(&w, 0, expected.clone()).is_err());
        assert!(apply_confirmed_shape_delete(&mut w, &selected).is_err());
        w.cancel_note_editor(editor.id).unwrap();
        let (export, _, _) = w.begin_export().unwrap();
        assert!(prepare_shape_delete(&w, 0, expected.clone()).is_err());
        assert!(apply_confirmed_shape_delete(&mut w, &selected).is_err());
        w.release_export(export);
        w.replace_document(
            Session::open(&selected.snapshot).unwrap(),
            "reopened.pdf".into(),
        )
        .unwrap();
        assert!(apply_confirmed_shape_delete(&mut w, &selected).is_err());
        let selected = prepare_shape_delete(&w, 0, expected).unwrap();
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let changed = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_confirmed_shape_delete(&mut w, &selected).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), changed);
    }

    #[test]
    fn shape_delete_checkpoint_failure_keeps_committed_dirty_document() {
        let (mut w, directory) = shape_workspace();
        let expected = workspace_shapes(&w, 0).unwrap().shapes[0].clone();
        let selected = prepare_shape_delete(&w, 0, expected).unwrap();
        w.journal = Some(recovery::Journal::new(
            directory.path().join("missing/draft.json"),
        ));
        let view = apply_confirmed_shape_delete(&mut w, &selected).unwrap();
        assert!(view.warning.is_some());
        assert!(w.has_unsaved_work());
        assert!(workspace_shapes(&w, 0).unwrap().shapes.is_empty());
    }

    fn note_workspace() -> (Workspace, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let mut session = Session::blank(100., 100., 1).unwrap();
        session
            .apply(Edit::AddNote {
                page: 0,
                bounds: pdfmaster_core::Rect {
                    x: 10.,
                    y: 10.,
                    width: 20.,
                    height: 20.,
                },
                contents: "原便笺 🦀".into(),
                color: pdfmaster_core::Color {
                    r: 1.,
                    g: 1.,
                    b: 0.,
                },
            })
            .unwrap();
        session.mark_saved();
        let journal = recovery::Journal::new(directory.path().join("draft.json"));
        (
            Workspace {
                session: Some(session),
                journal: Some(journal),
                ..Default::default()
            },
            directory,
        )
    }

    #[test]
    fn editor_guard_blocks_exit_without_claiming_pdf_bytes_are_dirty() {
        let (mut w, _directory) = note_workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        assert!(!w.has_unsaved_work());
        assert!(w.begin_note_editor(0, 0, "stale").is_err());
        assert!(!w.has_unsaved_work());
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert!(w.has_unsaved_work());
        assert!(w.clean().is_err());
        assert!(!w.dirty());
        assert!(!w.view().unwrap().info.dirty);
        assert_eq!(w.view().unwrap().note_editor.unwrap().id, editor.id);
        assert!(w.begin_note_editor(0, 0, "原便笺 🦀").is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
    }

    #[test]
    fn stale_editor_ids_cannot_cancel_or_commit_a_new_editor() {
        let (mut w, _directory) = note_workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let first = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert!(w.cancel_note_editor(first.id + 1).is_err());
        w.cancel_note_editor(first.id).unwrap();
        assert!(!w.has_unsaved_work());
        let second = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        assert_ne!(first.id, second.id);
        assert!(w.cancel_note_editor(first.id).is_err());
        assert!(w.commit_note_editor(first.id, "stale".into()).is_err());
        assert!(w.has_unsaved_work());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        w.cancel_note_editor(second.id).unwrap();
        assert!(!w.has_unsaved_work());
    }

    #[test]
    fn document_view_recovers_an_unacknowledged_editor_id_without_mutating_pdf() {
        let (mut w, _directory) = note_workspace();
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        // Simulate losing the begin reply: recover only through the same view
        // serialized by document_info, never through the original return value.
        w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        let reply = serde_json::to_value(w.view().unwrap()).unwrap();
        let id = reply["note_editor"]["id"].as_u64().unwrap() as u32;
        assert_eq!(reply["note_editor"]["page"], 0);
        assert_eq!(reply["note_editor"]["original"]["contents"], "原便笺 🦀");
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert!(w.has_unsaved_work());
        assert!(w.cancel_note_editor(id).unwrap().note_editor.is_none());
        assert!(!w.has_unsaved_work());
    }

    #[test]
    fn retry_after_lost_cancel_or_commit_ack_refreshes_view_without_reapplying() {
        let (mut w, _directory) = note_workspace();
        let original = w.session.as_ref().unwrap().bytes().unwrap();
        let cancelled = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        w.cancel_note_editor(cancelled.id).unwrap(); // Reply deliberately discarded.
        let reply = w.cancel_note_editor(cancelled.id).unwrap();
        assert!(reply.note_editor.is_none());
        assert!(!reply.info.dirty);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);

        let committed = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        w.commit_note_editor(committed.id, "已提交 🦀".into())
            .unwrap();
        let updated = w.session.as_ref().unwrap().bytes().unwrap();
        let reply = w.cancel_note_editor(committed.id).unwrap();
        assert!(reply.note_editor.is_none());
        assert!(reply.info.dirty);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), updated);
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
    }

    #[test]
    fn invalid_or_conflicting_operations_keep_editor_guard_and_pdf_unchanged() {
        let (mut w, _directory) = note_workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        for contents in [
            String::new(),
            " \n\t".into(),
            "原便笺 🦀".into(),
            "x".repeat(65537),
        ] {
            assert!(w.commit_note_editor(editor.id, contents).is_err());
            assert!(w.has_unsaved_work());
        }
        assert!(
            apply_single_edit(
                &mut w,
                Edit::Rotate {
                    page: 0,
                    degrees: 90
                }
            )
            .is_err()
        );
        assert!(apply_page_batch(&mut w, "1", PageBatchOperation::Rotate90, None).is_err());
        assert!(w.finish_verified_export(&before).is_err());
        assert!(w.ensure_editor_idle().is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(w.view().unwrap().note_editor.unwrap().id, editor.id);
    }

    #[test]
    fn editor_commit_is_one_undo_and_clears_only_its_own_guard() {
        let (mut w, _directory) = note_workspace();
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        let view = w
            .commit_note_editor(editor.id, "修改 🦀\n<script>literal</script>".into())
            .unwrap();
        assert!(view.note_editor.is_none());
        assert!(view.info.dirty);
        assert!(w.has_unsaved_work());
        assert!(w.ensure_editor_idle().is_ok());
        assert_eq!(
            workspace_notes(&w, 0).unwrap().notes[0].contents,
            "修改 🦀\n<script>literal</script>"
        );
        assert!(w.commit_note_editor(editor.id, "again".into()).is_err());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(
            workspace_notes(&w, 0).unwrap().notes[0].contents,
            "原便笺 🦀"
        );
        assert!(!w.has_unsaved_work());
    }

    #[test]
    fn editor_commit_journal_failure_keeps_pdf_dirty_and_exit_blocked() {
        let (mut w, directory) = note_workspace();
        w.journal = Some(recovery::Journal::new(
            directory.path().join("missing/draft.json"),
        ));
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        let view = w.commit_note_editor(editor.id, "仍在内存".into()).unwrap();
        assert!(view.note_editor.is_none());
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        assert!(w.has_unsaved_work());
        assert!(w.clean().is_err());
        assert_eq!(
            workspace_notes(&w, 0).unwrap().notes[0].contents,
            "仍在内存"
        );
    }

    #[test]
    fn core_rejection_keeps_the_same_editor_and_current_bytes() {
        let (mut w, _directory) = note_workspace();
        let editor = w.begin_note_editor(0, 0, "原便笺 🦀").unwrap();
        // Simulate an internal mutation outside guarded IPC. The core expected
        // contents binding must still reject this now-stale editor atomically.
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::UpdateNote {
                page: 0,
                annotation_index: 0,
                expected_contents: "原便笺 🦀".into(),
                contents: "外部内部更新".into(),
            })
            .unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        assert!(w.commit_note_editor(editor.id, "不能覆盖".into()).is_err());
        assert_eq!(w.view().unwrap().note_editor.unwrap().id, editor.id);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
    }

    #[test]
    fn export_reservation_excludes_editors_mutation_and_parallel_exports() {
        let (mut w, _directory) = note_workspace();
        let (id, _, bytes) = w.begin_export().unwrap();
        assert!(w.has_unsaved_work());
        assert!(!w.dirty());
        assert!(w.begin_note_editor(0, 0, "原便笺 🦀").is_err());
        assert!(w.begin_export().is_err());
        assert!(w.clean().is_err());
        assert!(
            apply_single_edit(
                &mut w,
                Edit::Rotate {
                    page: 0,
                    degrees: 90
                }
            )
            .is_err()
        );
        assert!(w.finish_verified_export(&bytes).is_err());
        w.release_export(id + 1);
        assert!(w.has_unsaved_work());
        w.release_export(id);
        assert!(!w.has_unsaved_work());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    }

    #[test]
    fn stale_export_completion_cannot_release_a_new_reservation() {
        let (mut w, _directory) = note_workspace();
        let (first, _, bytes) = w.begin_export().unwrap();
        w.release_export(first);
        let (second, _, _) = w.begin_export().unwrap();
        assert_ne!(first, second);
        assert!(w.finish_reserved_export(first, &bytes).is_err());
        w.release_export(first);
        assert!(w.has_unsaved_work());
        let view = w.finish_reserved_export(second, &bytes).unwrap();
        assert!(!view.info.dirty);
        assert!(!w.has_unsaved_work());
    }

    #[test]
    fn export_scope_releases_reservation_on_early_error_or_cancel() {
        let (w, _directory) = note_workspace();
        let store = Mutex::new(w);
        let (id, _, _) = store.lock().unwrap().begin_export().unwrap();
        {
            let _reservation = ExportReservation { store: &store, id };
            assert!(store.lock().unwrap().has_unsaved_work());
        }
        assert!(!store.lock().unwrap().has_unsaved_work());
        assert!(
            store
                .lock()
                .unwrap()
                .begin_note_editor(0, 0, "原便笺 🦀")
                .is_ok()
        );
    }

    #[test]
    fn notes_read_current_memory_without_checkpointing_or_changing_history() {
        let mut w = Workspace::default();
        assert!(workspace_notes(&w, 0).is_err());
        w.session = Some(Session::blank(100., 100., 2).unwrap());
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::AddNote {
                page: 1,
                bounds: pdfmaster_core::Rect {
                    x: 10.,
                    y: 10.,
                    width: 20.,
                    height: 20.,
                },
                contents: "当前内存 🦀".into(),
                color: pdfmaster_core::Color {
                    r: 1.,
                    g: 0.8,
                    b: 0.,
                },
            })
            .unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        assert!(workspace_notes(&w, 0).unwrap().notes.is_empty());
        assert_eq!(
            workspace_notes(&w, 1).unwrap().notes[0].contents,
            "当前内存 🦀"
        );
        assert!(workspace_notes(&w, 2).is_err());
        assert!(!w.recovery_fault);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
    }
    #[test]
    fn restored_bytes_stay_unsaved_until_export() {
        let w = Workspace {
            session: Some(Session::blank(100., 100., 1).unwrap()),
            restored: true,
            ..Default::default()
        };
        assert!(w.dirty());
        assert!(w.clean().is_err());
        assert!(w.view().unwrap().info.dirty);
        assert!(w.view().unwrap().restored);
    }
    #[test]
    fn persistence_failure_keeps_bytes_and_blocks_exit() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 1).unwrap()),
            ..Default::default()
        };
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(w.persist().is_err());
        assert!(w.dirty());
        assert!(w.clean().is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
    }
    #[test]
    fn persistence_failure_after_edit_returns_current_dirty_view_with_warning() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 1).unwrap()),
            ..Default::default()
        };
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let view = w.checkpoint().unwrap();
        assert_eq!(view.info.pages[0].rotation, 90);
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        assert!(w.clean().is_err());
    }
    #[test]
    fn verified_current_export_with_durable_checkpoint_recovers_transient_fault() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 1).unwrap()),
            ..Default::default()
        };
        assert!(w.persist().is_err());
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        w.journal = Some(recovery::Journal::new(dir.path().join("draft.json")));
        // Merely writing a later checkpoint does not acknowledge export safety.
        assert!(w.checkpoint().unwrap().info.dirty);
        let view = w.finish_verified_export(&bytes).unwrap();
        assert!(!view.info.dirty);
        assert!(view.warning.is_none());
        assert!(w.clean().is_ok());
        assert!(w.journal.as_ref().unwrap().load().unwrap().is_none());
    }
    #[test]
    fn export_cannot_clear_fault_for_failed_checkpoint_or_stale_snapshot() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 1).unwrap()),
            ..Default::default()
        };
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        let view = w.finish_verified_export(&bytes).unwrap();
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        let dir = tempfile::tempdir().unwrap();
        w.journal = Some(recovery::Journal::new(dir.path().join("draft.json")));
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let view = w.finish_verified_export(&bytes).unwrap();
        assert_eq!(view.info.pages[0].rotation, 90);
        assert!(view.info.dirty);
        assert!(view.warning.is_some());
        assert!(w.clean().is_err());
    }
    #[test]
    fn search_reads_only_the_current_in_memory_session() {
        let mut session = Session::blank(200., 200., 1).unwrap();
        session
            .apply(Edit::AddText {
                page: 0,
                bounds: pdfmaster_core::Rect {
                    x: 20.,
                    y: 80.,
                    width: 160.,
                    height: 40.,
                },
                text: "Current Session Search".into(),
                font_size: 12.,
                color: pdfmaster_core::Color {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                },
            })
            .unwrap();
        let w = Workspace {
            session: Some(session),
            ..Default::default()
        };
        let result = search::search_session(w.session.as_ref().unwrap(), "session", false).unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].page_index, 0);
    }
    #[test]
    fn search_command_wire_contract_is_explicitly_snake_case() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        assert!(
            source.contains(
                "#[tauri::command(rename_all = \"snake_case\")]\nasync fn search_document"
            )
        );
    }
    #[test]
    fn outline_command_reads_the_current_session_without_editing_or_opening_targets() {
        let original = include_str!("lib.rs").replace("\r\n", "\n");
        for source in [original.clone(), original.replace('\n', "\r\n")] {
            let source = source.replace("\r\n", "\n");
            let command = source
                .split_once("#[tauri::command]\nasync fn outline_document(")
                .expect("outline RPC must exist")
                .1
                .split_once("#[tauri::command")
                .unwrap()
                .0;
            assert!(command.contains("session.outline()"));
            assert!(command.contains("workspace.session.as_ref()"));
            assert!(!command.contains("apply("));
            assert!(!command.contains("checkpoint("));
            assert!(!command.contains("opener"));
            let handlers = source
                .rsplit_once(".invoke_handler(tauri::generate_handler![")
                .unwrap()
                .1
                .split_once("])")
                .unwrap()
                .0;
            assert!(handlers.contains("outline_document"));
        }
    }
    #[test]
    fn annotations_project_the_current_workspace_without_claiming_an_editor_or_mutating() {
        assert!(workspace_annotations(&Workspace::default()).is_err());
        let (w, _directory) = note_workspace();
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let rows = workspace_annotations(&w).unwrap();
        assert!(!rows.is_empty());
        assert_eq!(rows[0].subtype, "Text");
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert!(w.note_editor.is_none());
        assert!(w.ink_editor.is_none());
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let command = source
            .split_once("#[tauri::command]\nasync fn annotations_document(")
            .expect("annotation RPC")
            .1
            .split_once("#[tauri::command")
            .unwrap()
            .0;
        assert!(command.contains("workspace_annotations(&workspace)"));
        assert!(!command.contains("checkpoint("));
        assert!(!command.contains("apply("));
        assert!(
            source
                .rsplit_once(".invoke_handler(tauri::generate_handler![")
                .unwrap()
                .1
                .split_once("])")
                .unwrap()
                .0
                .contains("annotations_document")
        );
    }
    #[test]
    fn page_batch_command_wire_contract_is_explicitly_snake_case() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        assert!(
            source.contains("#[tauri::command(rename_all = \"snake_case\")]\nasync fn batch_pages")
        );
    }
    #[test]
    fn confirmed_delete_rejects_a_changed_document_snapshot() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        };
        let snapshot = w.session.as_ref().unwrap().bytes().unwrap();
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let changed = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_confirmed_delete(&mut w, "1", 1, &snapshot, 0).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), changed);
        assert_eq!(w.session.as_ref().unwrap().info().page_count, 3);
    }
    #[test]
    fn confirmed_snapshot_deletes_once_and_undo_restores_all_pages() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        };
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(matches!(
            apply_confirmed_delete(&mut w, "3,1", 2, &before, 0).unwrap(),
            PageBatchReply::Applied {
                affected_count: 2,
                ..
            }
        ));
        assert_eq!(w.session.as_ref().unwrap().info().page_count, 1);
        assert!(apply_confirmed_delete(&mut w, "1", 1, &before, 0).is_err());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(
            serde_json::to_value(PageBatchReply::Cancelled).unwrap(),
            serde_json::json!({"kind":"cancelled"})
        );
    }
    #[test]
    fn rotate_batch_is_one_checkpoint_and_one_undo_even_when_draft_write_fails() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        };
        let response = apply_page_batch(&mut w, "1,3", PageBatchOperation::Rotate90, None).unwrap();
        let PageBatchReply::Applied {
            document,
            affected_count,
        } = response
        else {
            panic!("rotation must apply without confirmation")
        };
        assert_eq!(affected_count, 2);
        assert_eq!(
            document
                .info
                .pages
                .iter()
                .map(|page| page.rotation)
                .collect::<Vec<_>>(),
            vec![90, 0, 90]
        );
        assert!(document.warning.is_some());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(
            w.session
                .as_ref()
                .unwrap()
                .info()
                .pages
                .iter()
                .map(|page| page.rotation)
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(w.session.as_mut().unwrap().undo().is_err());
    }
    #[test]
    fn delete_batch_requires_exact_count_confirmation_and_never_deletes_all() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        };
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let response = apply_page_batch(&mut w, "1,3", PageBatchOperation::Delete, None).unwrap();
        assert!(matches!(
            response,
            PageBatchReply::ConfirmationRequired {
                count: 2,
                page_count: 3
            }
        ));
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(apply_page_batch(&mut w, "1,3", PageBatchOperation::Delete, Some(1)).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(apply_page_batch(&mut w, "1-3", PageBatchOperation::Delete, None).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        let response =
            apply_page_batch(&mut w, "1,3", PageBatchOperation::Delete, Some(2)).unwrap();
        assert!(matches!(
            response,
            PageBatchReply::Applied {
                affected_count: 2,
                ..
            }
        ));
        assert_eq!(w.session.as_ref().unwrap().info().page_count, 1);
    }
    #[test]
    fn invalid_or_unsupported_delete_batch_is_atomic() {
        let mut session = Session::blank(100., 100., 3).unwrap();
        session
            .apply(Edit::AddNote {
                page: 1,
                bounds: pdfmaster_core::Rect {
                    x: 10.,
                    y: 10.,
                    width: 20.,
                    height: 20.,
                },
                contents: "keep".into(),
                color: pdfmaster_core::Color {
                    r: 1.,
                    g: 1.,
                    b: 0.,
                },
            })
            .unwrap();
        let mut w = Workspace {
            session: Some(session),
            ..Default::default()
        };
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(apply_page_batch(&mut w, "1,x", PageBatchOperation::Rotate90, None).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(apply_page_batch(&mut w, "1-2", PageBatchOperation::Delete, Some(2)).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }
    #[test]
    fn generic_edit_route_cannot_bypass_page_batch_confirmation() {
        let mut w = Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        };
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        for edit in [
            Edit::RotatePages {
                pages: vec![0, 1],
                degrees: 90,
            },
            Edit::DeletePages { pages: vec![0, 1] },
            Edit::DeletePage { page: 0 },
        ] {
            assert!(apply_single_edit(&mut w, edit).is_err());
            assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        }
    }
}

impl Workspace {
    fn cancel_ink_request(&mut self, request: &str) -> Reply<DocumentView> {
        Self::validate_ink_request(request)?;
        if !self.cancelled_ink_requests.contains(request)
            && self.cancelled_ink_requests.len() >= 16_384
        {
            return Err("手绘恢复请求预算已用尽，请先保存文档并正常重启。".into());
        }
        self.cancelled_ink_requests.insert(request.into());
        if let Some(editor) = &self.ink_editor {
            if editor.request != request {
                return Err("该恢复请求不属于当前笔画，未取消新手绘。".into());
            }
            return self.cancel_ink(editor.id);
        }
        self.view()
    }
    fn validate_ink_request(request: &str) -> Reply<()> {
        if !(8..=64).contains(&request.len())
            || !request
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("手绘请求身份无效。".into());
        }
        Ok(())
    }
    fn begin_ink(
        &mut self,
        request: &str,
        page: usize,
        color: pdfmaster_core::Color,
        width: f32,
    ) -> Reply<InkEditorView> {
        Self::validate_ink_request(request)?;
        if self.cancelled_ink_requests.contains(request) {
            return Err("该手绘请求已取消，未重新开始。".into());
        }
        self.ensure_editor_idle()?;
        let s = self.session.as_ref().ok_or("请先打开 PDF")?;
        if page >= s.info().page_count
            || !width.is_finite()
            || !(0.25..=32.).contains(&width)
            || [color.r, color.g, color.b]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
        {
            return Err("手绘页面或样式无效。".into());
        }
        let id = self
            .next_ink_id
            .checked_add(1)
            .ok_or("手绘会话编号已耗尽，请安全保存后重启。")?;
        let e = InkEditor {
            id,
            request: request.into(),
            page,
            color,
            line_width: width,
            document_epoch: self.document_epoch,
            snapshot: s.bytes().map_err(|e| e.to_string())?,
        };
        self.next_ink_id = id;
        let view = e.view();
        self.ink_editor = Some(e);
        Ok(view)
    }
    fn commit_ink(
        &mut self,
        id: u32,
        strokes: Vec<Vec<pdfmaster_core::Point>>,
    ) -> Reply<DocumentView> {
        if self.ink_editor.is_none() && self.finished_ink == Some((id, self.document_epoch, true)) {
            return self.view();
        }
        let e = self
            .ink_editor
            .as_ref()
            .filter(|e| e.id == id)
            .ok_or("手绘会话已改变，请同步状态。")?;
        let s = self.session.as_mut().ok_or("请先打开 PDF")?;
        if e.document_epoch != self.document_epoch
            || s.bytes().map_err(|e| e.to_string())? != e.snapshot
        {
            return Err("手绘期间文档已改变，未提交笔画。".into());
        }
        s.apply(Edit::AddInk {
            page: e.page,
            strokes,
            color: e.color,
            line_width: e.line_width,
            smoothed: true,
        })
        .map_err(|e| e.to_string())?;
        self.ink_editor = None;
        self.finished_ink = Some((id, self.document_epoch, true));
        self.checkpoint()
    }
    fn cancel_ink(&mut self, id: u32) -> Reply<DocumentView> {
        if self.ink_editor.is_none()
            && self
                .finished_ink
                .is_some_and(|(last, epoch, _)| last == id && epoch == self.document_epoch)
        {
            return self.view();
        }
        if self.ink_editor.as_ref().is_none_or(|e| e.id != id) {
            return Err("手绘会话已改变，未取消新笔画。".into());
        }
        self.ink_editor = None;
        self.finished_ink = Some((id, self.document_epoch, false));
        self.view()
    }
    fn replace_document(&mut self, session: Session, name: String) -> Reply<DocumentView> {
        self.clean()?;
        let epoch = self
            .document_epoch
            .checked_add(1)
            .ok_or("文档会话编号已耗尽，请正常退出后重试。")?;
        self.session = Some(session);
        self.name = name;
        self.document_epoch = epoch;
        self.checkpoint()
    }
    fn has_unsaved_work(&self) -> bool {
        self.dirty()
            || self.note_editor.is_some()
            || self.ink_editor.is_some()
            || self.export_reservation.is_some()
    }
    fn ensure_editor_idle(&self) -> Reply<()> {
        if self.export_reservation.is_some() {
            Err("正在处理 PDF 文件，请等待文件面板和读写完成或取消操作。".into())
        } else if self.note_editor.is_some() {
            Err("请先保存或取消便笺编辑；编辑框内容尚未写入 PDF。".into())
        } else if self.ink_editor.is_some() {
            Err("手绘笔画尚未结束，请抬笔提交或取消手绘。".into())
        } else {
            Ok(())
        }
    }
    fn begin_export(&mut self) -> Reply<(u32, String, Vec<u8>)> {
        self.ensure_editor_idle()?;
        let bytes = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|error| error.to_string())?;
        let id = self
            .next_export_id
            .checked_add(1)
            .ok_or("导出会话编号已耗尽，请先安全保存并重启应用。")?;
        self.next_export_id = id;
        self.export_reservation = Some(id);
        Ok((id, self.name.clone(), bytes))
    }
    fn release_export(&mut self, id: u32) {
        if self.export_reservation == Some(id) {
            self.export_reservation = None;
        }
    }
    fn begin_extraction(&mut self, range: &str) -> Reply<(u32, String, Vec<u8>)> {
        self.ensure_editor_idle()?;
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        let pages = PageRange::parse(range, session.info().page_count)
            .map_err(|error| format!("页码范围无效：{error}"))?;
        let bytes = session
            .extract_pages(&pages)
            .map_err(|error| error.to_string())?;
        let id = self
            .next_export_id
            .checked_add(1)
            .ok_or("导出会话编号已耗尽，请先安全保存并重启应用。")?;
        let name = format!(
            "{}-提取页面.pdf",
            Path::new(&self.name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
        );
        self.next_export_id = id;
        self.export_reservation = Some(id);
        Ok((id, name, bytes))
    }
    fn finish_extraction(&mut self, id: u32) -> Reply<DocumentView> {
        if self.export_reservation != Some(id) {
            return Err("提取会话已改变，不能确认本次导出。".into());
        }
        // Derived exports never mark the source saved or checkpoint a clean
        // journal, even when every page was selected.
        self.release_export(id);
        self.view()
    }
    fn begin_insertion(&mut self, index: usize) -> Reply<u32> {
        self.ensure_editor_idle()?;
        let count = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .info()
            .page_count;
        if index > count {
            return Err("插入位置超出当前文档页数。".into());
        }
        let id = self
            .next_export_id
            .checked_add(1)
            .ok_or("文件操作编号已耗尽，请先安全保存并重启应用。")?;
        self.next_export_id = id;
        self.export_reservation = Some(id);
        Ok(id)
    }
    fn finish_insertion(&mut self, id: u32, index: usize, bytes: &[u8]) -> Reply<DocumentView> {
        self.finish_insertions(id, index, &[bytes])
    }
    fn finish_insertions(&mut self, id: u32, index: usize, files: &[&[u8]]) -> Reply<DocumentView> {
        if self.export_reservation != Some(id) {
            return Err("插入会话已改变，文档未修改。".into());
        }
        // Release only this token while still holding the mutex. The core
        // publishes an atomic transaction; persistence failure stays visible.
        self.release_export(id);
        self.session
            .as_mut()
            .ok_or("请先打开 PDF")?
            .insert_pdfs(files, index)
            .map_err(|error| error.to_string())?;
        self.checkpoint()
    }
    fn finish_reserved_export(&mut self, id: u32, bytes: &[u8]) -> Reply<DocumentView> {
        if self.export_reservation != Some(id) {
            return Err("导出会话已改变，不能确认本次保存。".into());
        }
        self.release_export(id);
        // Still holding the same mutex: no editor/mutation can start between
        // releasing this token and acknowledging the exact exported snapshot.
        self.finish_verified_export(bytes)
    }
    fn begin_note_editor(
        &mut self,
        page: usize,
        annotation_index: usize,
        expected: &str,
    ) -> Reply<NoteEditor> {
        self.ensure_editor_idle()?;
        if expected.len() > 1024 * 1024 {
            return Err("便笺原始内容超出预算。".into());
        }
        let original = workspace_notes(self, page)?
            .notes
            .into_iter()
            .find(|note| note.annotation_index == annotation_index && note.contents == expected)
            .ok_or("便笺已改变，请重新选择。")?;
        let id = self
            .next_editor_id
            .checked_add(1)
            .ok_or("编辑会话编号已耗尽，请先安全保存并重启应用。")?;
        let editor = NoteEditor { id, page, original };
        self.next_editor_id = id;
        self.note_editor = Some(editor.clone());
        Ok(editor)
    }
    fn active_editor(&self, id: u32) -> Reply<NoteEditor> {
        self.note_editor
            .as_ref()
            .filter(|editor| editor.id == id)
            .cloned()
            .ok_or_else(|| "便笺编辑会话已改变，请刷新当前文档。".into())
    }
    fn cancel_note_editor(&mut self, id: u32) -> Reply<DocumentView> {
        // An acknowledgement can be lost after commit/cancel. Releasing the
        // same finished ID is safe, but can never release a later editor.
        if self.note_editor.is_none() && id != 0 && id == self.next_editor_id {
            return self.view();
        }
        self.active_editor(id)?;
        self.note_editor = None;
        self.view()
    }
    fn commit_note_editor(&mut self, id: u32, contents: String) -> Reply<DocumentView> {
        let editor = self.active_editor(id)?;
        if contents.trim().is_empty()
            || contents.len() > 64 * 1024
            || contents == editor.original.contents
        {
            return Err("便笺修改不能为空、超过64KiB或与原内容相同。".into());
        }
        self.session
            .as_mut()
            .ok_or("请先打开 PDF")?
            .apply(Edit::UpdateNote {
                page: editor.page,
                annotation_index: editor.original.annotation_index,
                expected_contents: editor.original.contents,
                contents,
            })
            .map_err(|error| error.to_string())?;
        // The PDF is already dirty if the subsequent journal write fails.
        // Clear the editor only after the atomic core mutation succeeded.
        self.note_editor = None;
        self.checkpoint()
    }
    fn view(&self) -> Reply<DocumentView> {
        let mut info = self.session.as_ref().ok_or("请先打开 PDF")?.info();
        info.dirty = self.dirty();
        Ok(DocumentView {
            name: self.name.clone(),
            info,
            restored: self.restored,
            warning: self.recovery_warning.clone(),
            note_editor: self.note_editor.clone(),
            ink_editor: self.ink_editor.as_ref().map(InkEditor::view),
        })
    }
    fn clean(&self) -> Reply<()> {
        self.ensure_editor_idle()?;
        if self.dirty() {
            Err("当前文档尚未保存。请先导出副本，再打开或新建文档。".into())
        } else {
            Ok(())
        }
    }
    fn dirty(&self) -> bool {
        self.restored || self.recovery_fault || self.session.as_ref().is_some_and(|s| s.is_dirty())
    }
    fn persist(&mut self) -> Reply<()> {
        let result = (|| {
            let session = self.session.as_ref().ok_or("当前没有文档")?;
            let bytes = session.bytes().map_err(|e| e.to_string())?;
            self.journal
                .as_ref()
                .ok_or("草稿存储不可用")?
                .save(&self.name, &bytes, session.is_dirty() || self.restored)
                .map_err(|_| {
                    "本地草稿写入失败。修改仍保留在内存，请立即导出副本，勿关闭应用。".to_string()
                })
        })();
        if let Err(error) = &result {
            // Persistence faults are sticky for this process. A later write
            // cannot prove that the already-failed checkpoint was durable.
            self.recovery_fault = true;
            self.recovery_warning = Some(error.clone());
        }
        result
    }
    fn checkpoint(&mut self) -> Reply<DocumentView> {
        // The error is not discarded: persist records it as a sticky fault and
        // the returned view exposes the warning while preserving current bytes.
        let _ = self.persist();
        self.view()
    }
    fn finish_verified_export(&mut self, bytes: &[u8]) -> Reply<DocumentView> {
        self.ensure_editor_idle()?;
        if let Some(session) = self.session.as_mut()
            && session.bytes().map_err(|e| e.to_string())? == bytes
        {
            session.mark_saved();
            self.restored = false;
            // Only this exact, verified export plus its successful clean
            // journal checkpoint can acknowledge a transient recovery fault.
            if self.persist().is_ok() {
                self.recovery_fault = false;
                self.recovery_warning = None;
            }
            return self.view();
        }
        self.checkpoint()
    }
}

fn apply_page_batch(
    workspace: &mut Workspace,
    range: &str,
    operation: PageBatchOperation,
    confirmed_count: Option<usize>,
) -> Reply<PageBatchReply> {
    workspace.ensure_editor_idle()?;
    let page_count = workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .info()
        .page_count;
    let pages =
        PageRange::parse(range, page_count).map_err(|error| format!("页码范围无效：{error}"))?;
    let affected_count = pages.len();
    let edit = match operation {
        PageBatchOperation::Rotate90 => Edit::RotatePages { pages, degrees: 90 },
        PageBatchOperation::Delete => {
            if affected_count >= page_count {
                return Err("不能删除文档的全部页面。".into());
            }
            match confirmed_count {
                None => {
                    return Ok(PageBatchReply::ConfirmationRequired {
                        count: affected_count,
                        page_count,
                    });
                }
                Some(count) if count != affected_count => {
                    return Err("确认页数与当前页码范围不一致，文档未修改。".into());
                }
                Some(_) => Edit::DeletePages { pages },
            }
        }
    };
    workspace
        .session
        .as_mut()
        .ok_or("请先打开 PDF")?
        .apply(edit)
        .map_err(|error| error.to_string())?;
    Ok(PageBatchReply::Applied {
        document: Box::new(workspace.checkpoint()?),
        affected_count,
    })
}

fn apply_single_edit(workspace: &mut Workspace, edit: Edit) -> Reply<DocumentView> {
    workspace.ensure_editor_idle()?;
    if matches!(
        edit,
        Edit::RotatePages { .. }
            | Edit::AddInk { .. }
            | Edit::DeletePages { .. }
            | Edit::DeletePage { .. }
            | Edit::DeleteNote { .. }
            | Edit::DeleteShape { .. }
            | Edit::DeleteAnnotation { .. }
    ) {
        return Err(
            "批量旋转、手绘和删除必须使用各自的专用入口；页面与批注删除须原生确认。".into(),
        );
    }
    workspace
        .session
        .as_mut()
        .ok_or("请先打开 PDF")?
        .apply(edit)
        .map_err(|error| error.to_string())?;
    workspace.checkpoint()
}

fn apply_confirmed_delete(
    workspace: &mut Workspace,
    range: &str,
    count: usize,
    snapshot: &[u8],
    document_epoch: u64,
) -> Reply<PageBatchReply> {
    if workspace.document_epoch != document_epoch {
        return Err("确认期间已切换文档，请重新选择页面；文档未修改。".into());
    }
    let current = workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .bytes()
        .map_err(|error| error.to_string())?;
    if current != snapshot {
        return Err("确认期间文档已改变，请重新选择页面；文档未修改。".into());
    }
    apply_page_batch(workspace, range, PageBatchOperation::Delete, Some(count))
}

struct PreparedShapeDelete {
    document_epoch: u64,
    page: usize,
    expected: pdfmaster_core::ShapeAnnotation,
    snapshot: Vec<u8>,
}

fn prepare_shape_delete(
    workspace: &Workspace,
    page: usize,
    expected: pdfmaster_core::ShapeAnnotation,
) -> Reply<PreparedShapeDelete> {
    workspace.ensure_editor_idle()?;
    let snapshot = workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .bytes()
        .map_err(|e| e.to_string())?;
    let mut candidate = Session::open(&snapshot).map_err(|e| e.to_string())?;
    candidate
        .apply(Edit::DeleteShape {
            page,
            expected: expected.clone(),
        })
        .map_err(|e| e.to_string())?;
    Ok(PreparedShapeDelete {
        document_epoch: workspace.document_epoch,
        page,
        expected,
        snapshot,
    })
}

fn apply_confirmed_shape_delete(
    workspace: &mut Workspace,
    selected: &PreparedShapeDelete,
) -> Reply<DocumentView> {
    workspace.ensure_editor_idle()?;
    if workspace.document_epoch != selected.document_epoch {
        return Err("确认期间已切换文档，请重新选择图形；文档未修改。".into());
    }
    let session = workspace.session.as_mut().ok_or("请先打开 PDF")?;
    if session.bytes().map_err(|e| e.to_string())? != selected.snapshot {
        return Err("确认期间文档已改变，请重新选择图形；文档未修改。".into());
    }
    session
        .apply(Edit::DeleteShape {
            page: selected.page,
            expected: selected.expected.clone(),
        })
        .map_err(|e| e.to_string())?;
    workspace.checkpoint()
}

#[tauri::command(rename_all = "snake_case")]
async fn delete_shape_document(
    app: tauri::AppHandle,
    page: usize,
    expected: pdfmaster_core::ShapeAnnotation,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    let selected = {
        let workspace = state.lock().map_err(|_| "文档状态不可用")?;
        prepare_shape_delete(&workspace, page, expected)?
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().message(format!("确认删除第 {} 页的选中图形批注？\n关联弹窗会一并移除。可通过撤销恢复；此操作不是安全擦除，保存仍需导出副本。", page + 1))
        .title("删除图形批注")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom("删除图形".into(), "取消".into()));
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let dialog = dialog.parent(
        &app.get_webview_window("main")
            .ok_or("确认窗口不存在；文档未修改。")?,
    );
    dialog.show(move |confirmed| {
        let _ = sender.send(confirmed);
    });
    if !receiver
        .await
        .map_err(|_| "系统确认窗口不可用；文档未修改。")?
    {
        return Ok(None);
    }
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    apply_confirmed_shape_delete(&mut workspace, &selected).map(Some)
}

struct PreparedAnnotationDelete {
    document_epoch: u64,
    expected: pdfmaster_core::AnnotationRow,
    snapshot: Vec<u8>,
}

fn prepare_annotation_delete(
    workspace: &Workspace,
    expected: pdfmaster_core::AnnotationRow,
) -> Reply<PreparedAnnotationDelete> {
    workspace.ensure_editor_idle()?;
    if expected.contents.len() > 65_536
        || expected.subtype.len() > 128
        || expected
            .modified
            .as_ref()
            .is_some_and(|date| date.len() > 64)
    {
        return Err("所选批注超出预算；文档未修改。".into());
    }
    let snapshot = workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .bytes()
        .map_err(|error| error.to_string())?;
    let mut candidate = Session::open(&snapshot).map_err(|error| error.to_string())?;
    candidate
        .apply(Edit::DeleteAnnotation {
            expected: Box::new(expected.clone()),
        })
        .map_err(|error| error.to_string())?;
    Ok(PreparedAnnotationDelete {
        document_epoch: workspace.document_epoch,
        expected,
        snapshot,
    })
}

fn apply_confirmed_annotation_delete(
    workspace: &mut Workspace,
    selected: &PreparedAnnotationDelete,
) -> Reply<DocumentView> {
    workspace.ensure_editor_idle()?;
    if workspace.document_epoch != selected.document_epoch {
        return Err("确认期间已切换文档，请重新选择批注；文档未修改。".into());
    }
    let session = workspace.session.as_mut().ok_or("请先打开 PDF")?;
    if session.bytes().map_err(|error| error.to_string())? != selected.snapshot {
        return Err("确认期间文档已改变，请重新选择批注；文档未修改。".into());
    }
    session
        .apply(Edit::DeleteAnnotation {
            expected: Box::new(selected.expected.clone()),
        })
        .map_err(|error| error.to_string())?;
    workspace.checkpoint()
}

#[tauri::command(rename_all = "snake_case")]
async fn delete_annotation_document(
    app: tauri::AppHandle,
    expected: pdfmaster_core::AnnotationRow,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    let selected = {
        let workspace = state.lock().map_err(|_| "文档状态不可用")?;
        prepare_annotation_delete(&workspace, expected)?
    };
    let preview: String = selected.expected.contents.chars().take(120).collect();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dialog=app.dialog().message(format!("确认删除第 {} 页的选中批注（{}）？\n{}\n关联弹窗会一并移除。可通过撤销恢复；此操作不是安全擦除，保存仍需导出副本。",selected.expected.page_index+1,selected.expected.subtype,preview))
        .title("删除选中批注？")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom("删除".into(),"取消".into()));
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let dialog = dialog.parent(
        &app.get_webview_window("main")
            .ok_or("确认窗口不存在；文档未修改。")?,
    );
    dialog.show(move |confirmed| {
        let _ = sender.send(confirmed);
    });
    if !receiver
        .await
        .map_err(|_| "系统确认窗口不可用；文档未修改。")?
    {
        return Ok(None);
    }
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    apply_confirmed_annotation_delete(&mut workspace, &selected).map(Some)
}

struct PreparedNoteDelete {
    document_epoch: u64,
    page: usize,
    annotation_index: usize,
    contents: String,
    snapshot: Vec<u8>,
}

fn prepare_note_delete(
    workspace: &Workspace,
    page: usize,
    annotation_index: usize,
    expected: &str,
) -> Reply<PreparedNoteDelete> {
    workspace.ensure_editor_idle()?;
    if expected.len() > 1024 * 1024 {
        return Err("便笺原始内容超出预算。".into());
    }
    let snapshot = workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .bytes()
        .map_err(|error| error.to_string())?;
    // Preflight the actual core operation on a disposable copy before prompting.
    let mut candidate = Session::open(&snapshot).map_err(|error| error.to_string())?;
    candidate
        .apply(Edit::DeleteNote {
            page,
            annotation_index,
            expected_contents: expected.to_owned(),
        })
        .map_err(|error| error.to_string())?;
    Ok(PreparedNoteDelete {
        document_epoch: workspace.document_epoch,
        page,
        annotation_index,
        contents: expected.to_owned(),
        snapshot,
    })
}

fn apply_confirmed_note_delete(
    workspace: &mut Workspace,
    selected: &PreparedNoteDelete,
) -> Reply<DocumentView> {
    workspace.ensure_editor_idle()?;
    if workspace.document_epoch != selected.document_epoch {
        return Err("确认期间已切换文档，请重新选择便笺；文档未修改。".into());
    }
    let session = workspace.session.as_mut().ok_or("请先打开 PDF")?;
    if session.bytes().map_err(|error| error.to_string())? != selected.snapshot {
        return Err("确认期间文档已改变，请重新选择便笺；文档未修改。".into());
    }
    session
        .apply(Edit::DeleteNote {
            page: selected.page,
            annotation_index: selected.annotation_index,
            expected_contents: selected.contents.clone(),
        })
        .map_err(|error| error.to_string())?;
    workspace.checkpoint()
}

#[tauri::command(rename_all = "snake_case")]
async fn delete_note_document(
    app: tauri::AppHandle,
    page: usize,
    annotation_index: usize,
    expected_contents: String,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    let selected = {
        let workspace = state.lock().map_err(|_| "文档状态不可用")?;
        prepare_note_delete(&workspace, page, annotation_index, &expected_contents)?
    };
    let preview: String = selected.contents.chars().take(120).collect();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().message(format!("确认删除第 {} 页的选中便笺？\n{}\n关联弹窗会一并移除。可通过撤销恢复；此操作不是安全擦除，保存仍需导出副本。", page + 1, preview))
        .title("删除便笺")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom("删除便笺".into(), "取消".into()));
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let dialog = dialog.parent(
        &app.get_webview_window("main")
            .ok_or("确认窗口不存在；文档未修改。")?,
    );
    dialog.show(move |confirmed| {
        let _ = sender.send(confirmed);
    });
    if !receiver
        .await
        .map_err(|_| "系统确认窗口不可用；文档未修改。")?
    {
        return Ok(None);
    }
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    apply_confirmed_note_delete(&mut workspace, &selected).map(Some)
}

#[tauri::command]
async fn open_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    state.lock().map_err(|_| "文档状态不可用")?.clean()?;
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("PDF 文档", &["pdf"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|_| "不能访问所选文件")?;
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|e| e.to_string())?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let session = Session::open(&bytes).map_err(|e| e.to_string())?;
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into();
    w.replace_document(session, name).map(Some)
}

#[tauri::command]
async fn new_document(state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    let session = Session::blank(595., 842., 1).map_err(|e| e.to_string())?;
    w.replace_document(session, "未命名.pdf".into()).map(Some)
}

#[tauri::command]
async fn document_info(state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let w = state.lock().map_err(|_| "文档状态不可用")?;
    if w.recovery_fault && w.session.is_none() {
        return Err("本地草稿无法安全恢复。原草稿已保留，请勿覆盖。".into());
    }
    if w.session.is_none() {
        Ok(None)
    } else {
        w.view().map(Some)
    }
}

#[tauri::command]
async fn render_page(page: usize, scale: f32, state: State<'_, Store>) -> Reply<String> {
    let w = state.lock().map_err(|_| "文档状态不可用")?;
    let png = w
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .render_png(page, scale)
        .map_err(|e| e.to_string())?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

#[tauri::command]
async fn notes_document(page: usize, state: State<'_, Store>) -> Reply<pdfmaster_core::NoteList> {
    let workspace = state.lock().map_err(|_| "文档状态不可用")?;
    workspace_notes(&workspace, page)
}

#[tauri::command]
async fn shapes_document(page: usize, state: State<'_, Store>) -> Reply<pdfmaster_core::ShapeList> {
    let workspace = state.lock().map_err(|_| "文档状态不可用")?;
    workspace_shapes(&workspace, page)
}

#[tauri::command(rename_all = "snake_case")]
async fn begin_ink_document(
    request: String,
    page: usize,
    color: pdfmaster_core::Color,
    line_width: f32,
    state: State<'_, Store>,
) -> Reply<InkEditorView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_ink(&request, page, color, line_width)
}
#[tauri::command(rename_all = "snake_case")]
async fn commit_ink_document(
    id: u32,
    strokes: Vec<Vec<pdfmaster_core::Point>>,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .commit_ink(id, strokes)
}
#[tauri::command(rename_all = "snake_case")]
async fn cancel_ink_document(id: u32, state: State<'_, Store>) -> Reply<DocumentView> {
    state.lock().map_err(|_| "文档状态不可用")?.cancel_ink(id)
}
#[tauri::command(rename_all = "snake_case")]
async fn cancel_ink_request_document(
    request: String,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_ink_request(&request)
}

#[tauri::command(rename_all = "snake_case")]
async fn begin_note_editor(
    page: usize,
    annotation_index: usize,
    expected_contents: String,
    state: State<'_, Store>,
) -> Reply<NoteEditor> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_note_editor(page, annotation_index, &expected_contents)
}

#[tauri::command]
async fn cancel_note_editor(id: u32, state: State<'_, Store>) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_note_editor(id)
}

#[tauri::command]
async fn commit_note_editor(
    id: u32,
    contents: String,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .commit_note_editor(id, contents)
}

#[tauri::command(rename_all = "snake_case")]
async fn search_document(
    query: String,
    case_sensitive: bool,
    state: State<'_, Store>,
) -> Reply<search::SearchResponse> {
    let workspace = state.lock().map_err(|_| "文档状态不可用")?;
    let session = workspace.session.as_ref().ok_or("请先打开 PDF")?;
    search::search_session(session, &query, case_sensitive)
}

#[tauri::command]
async fn outline_document(state: State<'_, Store>) -> Reply<Vec<pdfmaster_core::OutlineItem>> {
    let workspace = state.lock().map_err(|_| "文档状态不可用")?;
    let session = workspace.session.as_ref().ok_or("请先打开 PDF")?;
    session.outline().map_err(|error| error.to_string())
}

#[tauri::command]
async fn annotations_document(
    state: State<'_, Store>,
) -> Reply<Vec<pdfmaster_core::AnnotationRow>> {
    let workspace = state.lock().map_err(|_| "文档状态不可用")?;
    workspace_annotations(&workspace)
}

#[tauri::command]
async fn edit_document(edit: Edit, state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    apply_single_edit(&mut w, edit).map(Some)
}

#[tauri::command(rename_all = "snake_case")]
async fn batch_pages(
    app: tauri::AppHandle,
    range: String,
    operation: PageBatchOperation,
    state: State<'_, Store>,
) -> Reply<PageBatchReply> {
    let (count, page_count, snapshot, document_epoch) = {
        let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
        match apply_page_batch(&mut workspace, &range, operation, None)? {
            PageBatchReply::ConfirmationRequired { count, page_count } => {
                let snapshot = workspace
                    .session
                    .as_ref()
                    .ok_or("请先打开 PDF")?
                    .bytes()
                    .map_err(|error| error.to_string())?;
                (count, page_count, snapshot, workspace.document_epoch)
            }
            reply => return Ok(reply),
        }
    };
    // Never hold the workspace mutex across a platform dialog. Bind the exact
    // immutable range and document bytes; IPC cannot supply a confirmation flag.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dialog = app
        .dialog()
        .message(format!(
            "确认删除选中的 {count} 页？文档共有 {page_count} 页；此操作可通过撤销恢复。"
        ))
        .title("批量删除页面")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            "删除选中页面".into(),
            "取消".into(),
        ));
    // On macOS an unparented RFD dialog uses CFUserNotification instead of an
    // application sheet. Attach desktop alerts to the actual candidate window.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let dialog = dialog.parent(
        &app.get_webview_window("main")
            .ok_or("确认窗口不存在；文档未修改。")?,
    );
    dialog.show(move |confirmed| {
        let _ = sender.send(confirmed);
    });
    if !receiver
        .await
        .map_err(|_| "系统确认窗口不可用；文档未修改。")?
    {
        return Ok(PageBatchReply::Cancelled);
    }
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    apply_confirmed_delete(&mut workspace, &range, count, &snapshot, document_epoch)
}
#[tauri::command]
async fn undo_document(state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.ensure_editor_idle()?;
    w.session
        .as_mut()
        .ok_or("请先打开 PDF")?
        .undo()
        .map_err(|e| e.to_string())?;
    w.checkpoint().map(Some)
}
#[tauri::command]
async fn redo_document(state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.ensure_editor_idle()?;
    w.session
        .as_mut()
        .ok_or("请先打开 PDF")?
        .redo()
        .map_err(|e| e.to_string())?;
    w.checkpoint().map(Some)
}

#[tauri::command]
async fn export_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    let (id, name, bytes) = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        w.ensure_editor_idle()?;
        w.begin_export()?
    };
    let reservation = ExportReservation {
        store: state.inner(),
        id,
    };
    let copy_name = format!(
        "{}-副本.pdf",
        Path::new(&name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
    );
    if !publish_pdf_copy(&app, &copy_name, &bytes).await? {
        return Ok(None);
    }
    let view = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        w.finish_reserved_export(reservation.id, &bytes)?
    };
    drop(reservation);
    Ok(Some(view))
}

#[tauri::command(rename_all = "snake_case")]
async fn extract_pages_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
    range: String,
) -> Reply<Option<DocumentView>> {
    let (id, name, bytes) = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        w.begin_extraction(&range)?
    };
    let reservation = ExportReservation {
        store: state.inner(),
        id,
    };
    if !publish_pdf_copy(&app, &name, &bytes).await? {
        return Ok(None);
    }
    let view = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_extraction(id)?;
    drop(reservation);
    Ok(Some(view))
}

#[tauri::command(rename_all = "snake_case")]
async fn insert_pdf_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
    index: usize,
) -> Reply<Option<DocumentView>> {
    let id = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_insertion(index)?;
    let reservation = ExportReservation {
        store: state.inner(),
        id,
    };
    let Some(file) = app
        .dialog()
        .file()
        .add_filter("PDF 文档", &["pdf"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|_| "不能访问所选 PDF")?;
    // Native picker URL only, never an IPC-supplied path. Readback enforces
    // regular-file and bounded size checks before the core parses the bytes.
    let bytes = export_staging::readback_pdf(&path)
        .map_err(|_| "无法读取所选 PDF，文件须为普通文件且不超过 32MiB。")?;
    let view = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_insertion(id, index, &bytes)?;
    drop(reservation);
    Ok(Some(view))
}

#[tauri::command(rename_all = "snake_case")]
async fn insert_pdfs_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
    index: usize,
) -> Reply<Option<DocumentView>> {
    let id = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_insertion(index)?;
    let reservation = ExportReservation {
        store: state.inner(),
        id,
    };
    let Some(selected) = app
        .dialog()
        .file()
        .set_title("批量插入 PDF（按文件面板返回顺序）")
        .add_filter("PDF 文档", &["pdf"])
        .blocking_pick_files()
    else {
        return Ok(None);
    };
    if selected.is_empty() || selected.len() > pdfmaster_core::MAX_INSERT_FILES {
        return Err("请选择 1–32 份 PDF；整批未修改当前文档。".into());
    }
    let paths = selected
        .into_iter()
        .map(|file| file.into_path().map_err(|_| "不能访问所选 PDF"))
        .collect::<Result<Vec<_>, _>>()?;
    let files = read_insertion_files(&paths)?;
    let bytes: Vec<_> = files.iter().map(Vec::as_slice).collect();
    let view = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_insertions(id, index, &bytes)?;
    drop(reservation);
    Ok(Some(view))
}

async fn publish_pdf_copy(app: &tauri::AppHandle, copy_name: &str, bytes: &[u8]) -> Reply<bool> {
    // Validate before either platform publishes; use the same picker/provider
    // and exact bounded readback for full exports and derived extraction.
    Session::open(bytes).map_err(|e| e.to_string())?;
    #[cfg(target_os = "ios")]
    if !ios_export::export(app, copy_name, bytes).await? {
        return Ok(false);
    }
    #[cfg(not(target_os = "ios"))]
    {
        let Some(target) = app
            .dialog()
            .file()
            .set_file_name(copy_name)
            .add_filter("PDF 文档", &["pdf"])
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let target = target.into_path().map_err(|_| "不能访问导出位置")?;
        // Parse before publishing; persist no-clobber, read back exact bytes, then update dirty state.
        storage::export_new(&target, bytes).map_err(|e| e.to_string())?;
        if export_staging::readback_pdf(&target)? != bytes {
            return Err("导出回读不一致，文档仍保留未保存状态。".into());
        }
    }
    Ok(true)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app=tauri::Builder::default().plugin(tauri_plugin_dialog::init()).manage(Store::default())
        .setup(|app| {
            let dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
            let journal = recovery::Journal::new(dir.join("draft-v1.json"));
            let journal_lock = recovery::lock_directory(&dir)?;
            let store = app.state::<Store>();
            let mut w = store.lock().map_err(|_| "文档状态不可用")?;
            match journal.load() {
                Ok(Some(draft)) => match Session::open(&draft.bytes) {
                    Ok(session) => { w.session = Some(session); w.name = draft.name; w.restored = true; },
                    Err(_) => { w.recovery_fault = true; }
                },
                Ok(None) => {},
                Err(_) => { w.recovery_fault = true; }
            }
            w.journal = Some(journal);
            w._journal_lock = Some(journal_lock);
            drop(w);
            #[cfg(target_os = "macos")]
            macos_termination::install(app.handle())?;
            #[cfg(desktop)] {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
                // Native predefined Quit invokes NSApplication terminate: directly,
                // bypassing RunEvent::ExitRequested on macOS. Route through app.exit.
                let quit = MenuItem::with_id(app, "guarded-quit", "退出 PDF Master", true, Some("CmdOrCtrl+Q"))?;
                let application = Submenu::with_items(app, "PDF Master", true, &[&PredefinedMenuItem::about(app, None, None)?, &PredefinedMenuItem::separator(app)?, &quit])?;
                let edit = Submenu::with_items(app, "编辑", true, &[&PredefinedMenuItem::cut(app,None)?, &PredefinedMenuItem::copy(app,None)?, &PredefinedMenuItem::paste(app,None)?, &PredefinedMenuItem::select_all(app,None)?])?;
                let window = Submenu::with_items(app, "窗口", true, &[&PredefinedMenuItem::minimize(app,None)?, &PredefinedMenuItem::close_window(app,None)?])?;
                app.set_menu(Menu::with_items(app, &[&application, &edit, &window])?)?;
                app.on_menu_event(|app,event| { if event.id().as_ref() == "guarded-quit" { app.exit(0); } });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![open_document,new_document,document_info,render_page,notes_document,outline_document,annotations_document,shapes_document,begin_ink_document,commit_ink_document,cancel_ink_document,cancel_ink_request_document,begin_note_editor,cancel_note_editor,commit_note_editor,delete_note_document,delete_shape_document,delete_annotation_document,search_document,edit_document,batch_pages,undo_document,redo_document,export_document,extract_pages_document,insert_pdf_document,insert_pdfs_document])
        .on_window_event(|window,event|{
            if let tauri::WindowEvent::CloseRequested{api,..}=event {
                let state=window.state::<Store>();
                if state.lock().map_or(true,|w|w.has_unsaved_work()) {
                    api.prevent_close();
                    let dialog = window.dialog().message("存在未保存文档、便笺编辑或尚未完成的导出。请先完成或取消当前操作，再导出 PDF 后关闭。当前输入仍保留在窗口内。").title("保留未保存修改");
                    #[cfg(desktop)] let dialog = dialog.parent(window);
                    dialog.show(|_|{});
                }
            }
        })
        .build(tauri::generate_context!()).expect("unable to initialize PDF Master Rust");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && app
                .state::<Store>()
                .lock()
                .map_or(true, |w| w.has_unsaved_work())
        {
            api.prevent_exit();
            let dialog = app
                .dialog()
                .message(
                    "存在未保存文档、便笺编辑或尚未完成的导出。请先完成或取消当前操作，再导出 PDF 后退出。",
                )
                .title("保留未保存修改");
            #[cfg(desktop)]
            let dialog = if let Some(window) = app.get_webview_window("main") {
                dialog.parent(&window)
            } else {
                dialog
            };
            dialog.show(|_| {});
        }
    });
}
