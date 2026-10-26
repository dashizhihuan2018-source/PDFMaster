#[cfg(any(target_os = "ios", test))]
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
    name: String,
    journal: Option<recovery::Journal>,
    _journal_lock: Option<File>,
    restored: bool,
    recovery_fault: bool,
    recovery_warning: Option<String>,
}
#[derive(Clone, Serialize)]
struct DocumentView {
    name: String,
    info: DocumentInfo,
    restored: bool,
    warning: Option<String>,
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
        document: DocumentView,
        affected_count: usize,
    },
}
type Store = Mutex<Workspace>;
type Reply<T> = Result<T, String>;

fn workspace_notes(workspace: &Workspace, page: usize) -> Reply<pdfmaster_core::NoteList> {
    workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .notes(page)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
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
        let source = include_str!("lib.rs");
        assert!(
            source.contains(
                "#[tauri::command(rename_all = \"snake_case\")]\nasync fn search_document"
            )
        );
    }
    #[test]
    fn page_batch_command_wire_contract_is_explicitly_snake_case() {
        let source = include_str!("lib.rs");
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
        assert!(apply_confirmed_delete(&mut w, "1", 1, &snapshot).is_err());
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
            apply_confirmed_delete(&mut w, "3,1", 2, &before).unwrap(),
            PageBatchReply::Applied {
                affected_count: 2,
                ..
            }
        ));
        assert_eq!(w.session.as_ref().unwrap().info().page_count, 1);
        assert!(apply_confirmed_delete(&mut w, "1", 1, &before).is_err());
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
    fn view(&self) -> Reply<DocumentView> {
        let mut info = self.session.as_ref().ok_or("请先打开 PDF")?.info();
        info.dirty = self.dirty();
        Ok(DocumentView {
            name: self.name.clone(),
            info,
            restored: self.restored,
            warning: self.recovery_warning.clone(),
        })
    }
    fn clean(&self) -> Reply<()> {
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
        document: workspace.checkpoint()?,
        affected_count,
    })
}

fn apply_single_edit(workspace: &mut Workspace, edit: Edit) -> Reply<DocumentView> {
    if matches!(
        edit,
        Edit::RotatePages { .. } | Edit::DeletePages { .. } | Edit::DeletePage { .. }
    ) {
        return Err("批量旋转和页面删除必须使用带页数确认的批处理入口。".into());
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
) -> Reply<PageBatchReply> {
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
    w.clean()?;
    w.session = Some(session);
    w.name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into();
    w.checkpoint().map(Some)
}

#[tauri::command]
async fn new_document(state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.clean()?;
    w.session = Some(Session::blank(595., 842., 1).map_err(|e| e.to_string())?);
    w.name = "未命名.pdf".into();
    w.checkpoint().map(Some)
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
    let (count, page_count, snapshot) = {
        let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
        match apply_page_batch(&mut workspace, &range, operation, None)? {
            PageBatchReply::ConfirmationRequired { count, page_count } => {
                let snapshot = workspace
                    .session
                    .as_ref()
                    .ok_or("请先打开 PDF")?
                    .bytes()
                    .map_err(|error| error.to_string())?;
                (count, page_count, snapshot)
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
    apply_confirmed_delete(&mut workspace, &range, count, &snapshot)
}
#[tauri::command]
async fn undo_document(state: State<'_, Store>) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
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
    let (name, bytes) = {
        let w = state.lock().map_err(|_| "文档状态不可用")?;
        (
            w.name.clone(),
            w.session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .bytes()
                .map_err(|e| e.to_string())?,
        )
    };
    let copy_name = format!(
        "{}-副本.pdf",
        Path::new(&name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
    );
    // Validate the complete snapshot before either platform publishes it.
    Session::open(&bytes).map_err(|e| e.to_string())?;
    #[cfg(target_os = "ios")]
    if !ios_export::export(&app, &copy_name, &bytes).await? {
        return Ok(None);
    }
    #[cfg(not(target_os = "ios"))]
    {
        let Some(target) = app
            .dialog()
            .file()
            .set_file_name(&copy_name)
            .add_filter("PDF 文档", &["pdf"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let target = target.into_path().map_err(|_| "不能访问导出位置")?;
        // Parse before publishing; persist no-clobber, read back exact bytes, then update dirty state.
        storage::export_new(&target, &bytes).map_err(|e| e.to_string())?;
        if std::fs::read(&target).map_err(|e| e.to_string())? != bytes {
            return Err("导出回读不一致，文档仍保留未保存状态。".into());
        }
    }
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.finish_verified_export(&bytes).map(Some)
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
        .invoke_handler(tauri::generate_handler![open_document,new_document,document_info,render_page,notes_document,search_document,edit_document,batch_pages,undo_document,redo_document,export_document])
        .on_window_event(|window,event|{
            if let tauri::WindowEvent::CloseRequested{api,..}=event {
                let state=window.state::<Store>();
                if state.lock().map_or(true,|w|w.dirty()) {
                    api.prevent_close();
                    let dialog = window.dialog().message("文档尚未保存。请先导出副本，再关闭窗口。未保存的修改已保留在当前窗口。").title("保留未保存修改");
                    #[cfg(desktop)] let dialog = dialog.parent(window);
                    dialog.show(|_|{});
                }
            }
        })
        .build(tauri::generate_context!()).expect("unable to initialize PDF Master Rust");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && app.state::<Store>().lock().map_or(true, |w| w.dirty())
        {
            api.prevent_exit();
            let dialog = app
                .dialog()
                .message("文档尚未保存，请先导出副本再退出。")
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
