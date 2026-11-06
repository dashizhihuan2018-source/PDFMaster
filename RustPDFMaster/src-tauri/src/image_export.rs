use super::{Reply, Store, Workspace};
use crate::image_export_storage::{ImageExportControl, PreparedImageExport};
use pdfmaster_core::{ImageExportFormat, ImageExportRequest, Session};
use serde::Serialize;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use tauri::Manager;
use tauri::State;

const CHANGED_DOCUMENT: &str = "图片导出期间文档已改变；未创建公开目标。";
#[cfg(any(not(windows), test))]
const WINDOWS_ONLY: &str = "图片导出的原生目录选择仅支持 Windows。";

#[derive(Default)]
pub(super) struct ImageExportState {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) task: Option<ImageExportTask>,
}

pub(super) struct ImageExportTask {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) id: u32,
    phase: Phase,
    snapshot: Snapshot,
    request: ImageExportRequest,
    folder_name: String,
    control: ImageExportControl,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepared,
    Running,
}

#[derive(Clone)]
struct Snapshot {
    document: u32,
    epoch: u64,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Handle {
    pub(super) id: u32,
}

#[derive(Clone)]
pub(super) struct ImageExportJob {
    id: u32,
    snapshot: Snapshot,
    request: ImageExportRequest,
    folder_name: String,
    control: ImageExportControl,
}

impl ImageExportJob {
    #[cfg(test)]
    pub(super) fn id(&self) -> u32 {
        self.id
    }

    #[cfg(test)]
    pub(super) fn folder_name(&self) -> &str {
        &self.folder_name
    }

    #[cfg(test)]
    pub(super) fn is_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Receipt {
    pub(super) id: u32,
    pub(super) pages: usize,
    pub(super) total_bytes: usize,
    pub(super) folder_name: String,
    pub(super) warning: Option<String>,
}

pub(super) fn prepare(
    workspace: &mut Workspace,
    format: Option<ImageExportFormat>,
) -> Reply<Handle> {
    workspace.ensure_editor_idle()?;
    let session = workspace.session.as_ref().ok_or("请先打开 PDF")?;
    let info = session.info();
    let bytes = session.bytes().map_err(|error| error.to_string())?;
    let id = workspace
        .next_export_id
        .checked_add(1)
        .ok_or("图片导出编号已耗尽，请先安全保存并重启应用。")?;
    let folder_name = portable_folder_name(&workspace.name);
    let request = ImageExportRequest {
        format: format.unwrap_or(ImageExportFormat::Png),
        pages: (0..info.page_count).collect(),
        ..Default::default()
    };
    workspace.next_export_id = id;
    workspace.export_reservation = Some(id);
    workspace.image_export.task = Some(ImageExportTask {
        id,
        phase: Phase::Prepared,
        snapshot: Snapshot {
            document: workspace.active_document_id,
            epoch: workspace.document_epoch,
            bytes,
        },
        request,
        folder_name,
        control: ImageExportControl::new(),
    });
    Ok(Handle { id })
}

pub(super) fn claim(workspace: &mut Workspace, id: u32) -> Reply<ImageExportJob> {
    let task = workspace
        .image_export
        .task
        .as_mut()
        .filter(|task| {
            task.id == id
                && task.phase == Phase::Prepared
                && workspace.export_reservation == Some(id)
        })
        .ok_or("图片导出任务已失效或已经开始。")?;
    task.phase = Phase::Running;
    Ok(ImageExportJob {
        id,
        snapshot: task.snapshot.clone(),
        request: task.request.clone(),
        folder_name: task.folder_name.clone(),
        control: task.control.clone(),
    })
}

pub(super) fn cancel(workspace: &mut Workspace, id: u32) -> bool {
    let Some(task) = workspace
        .image_export
        .task
        .as_ref()
        .filter(|task| task.id == id && workspace.export_reservation == Some(id))
    else {
        return false;
    };
    let phase = task.phase;
    let cancelled = task.control.cancel();
    if phase == Phase::Prepared && cancelled {
        workspace.release_export(id);
    }
    cancelled
}

pub(super) fn released(workspace: &mut Workspace, id: u32) {
    if workspace
        .image_export
        .task
        .as_ref()
        .is_some_and(|task| task.id == id)
        && let Some(task) = workspace.image_export.task.take()
    {
        task.control.cancel();
    }
}

pub(super) fn validate_current(workspace: &Workspace, job: &ImageExportJob) -> Reply<()> {
    let owned = workspace.export_reservation == Some(job.id)
        && workspace
            .image_export
            .task
            .as_ref()
            .is_some_and(|task| task.id == job.id && task.phase == Phase::Running)
        && workspace.active_document_id == job.snapshot.document
        && workspace.document_epoch == job.snapshot.epoch;
    if !owned {
        return Err(CHANGED_DOCUMENT.into());
    }
    let bytes = workspace
        .session
        .as_ref()
        .ok_or(CHANGED_DOCUMENT)?
        .bytes()
        .map_err(|_| CHANGED_DOCUMENT.to_string())?;
    if bytes != job.snapshot.bytes {
        return Err(CHANGED_DOCUMENT.into());
    }
    Ok(())
}

pub(super) fn run_job(
    job: &ImageExportJob,
    parent: &Path,
    validate: impl FnOnce() -> Reply<()>,
) -> Reply<Receipt> {
    let session = Session::open(&job.snapshot.bytes)
        .map_err(|_| "图片导出快照无法打开；未创建公开目标。".to_string())?;
    let staged = PreparedImageExport::render_in(
        parent,
        job.folder_name.as_ref(),
        &session,
        &job.request,
        &job.control,
    )?;
    validate()?;
    let publication = staged.publish_new(&parent.join(&job.folder_name))?;
    Ok(Receipt {
        id: job.id,
        pages: publication.pages,
        total_bytes: publication.total_bytes,
        folder_name: job.folder_name.clone(),
        warning: publication.durability_warning,
    })
}

fn portable_folder_name(name: &str) -> String {
    let path = Path::new(name);
    let stem = if path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .is_some_and(|file_name| file_name.eq_ignore_ascii_case(".pdf"))
    {
        ""
    } else {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
    };
    let mut safe = String::with_capacity(stem.len().min(180));
    for ch in stem.chars() {
        let replacement =
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
        safe.push(if replacement { '-' } else { ch });
        if safe.len() >= 180 {
            break;
        }
    }
    let trimmed = safe.trim_matches(|ch: char| ch == ' ' || ch == '.');
    let reserved = matches!(
        trimmed.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let base = if trimmed.is_empty() || reserved {
        "document"
    } else {
        trimmed
    };
    format!("{base}-images")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CleanupAction {
    ReleaseBeforeWorker,
    CancelTransferredWorker,
}

pub(super) fn cleanup(workspace: &mut Workspace, id: u32, action: CleanupAction) {
    match action {
        CleanupAction::ReleaseBeforeWorker => {
            cancel(workspace, id);
            workspace.release_export(id);
        }
        CleanupAction::CancelTransferredWorker => {
            cancel(workspace, id);
        }
    }
}

pub(super) struct AdmissionGuard<C: FnMut(CleanupAction)> {
    cleanup: Option<C>,
    transferred: bool,
}

impl<C: FnMut(CleanupAction)> AdmissionGuard<C> {
    fn new(cleanup: C) -> Self {
        Self {
            cleanup: Some(cleanup),
            transferred: false,
        }
    }

    pub(super) fn transfer_to_worker(&mut self) {
        self.transferred = true;
    }

    pub(super) fn disarm(&mut self) {
        self.cleanup = None;
    }
}

impl<C: FnMut(CleanupAction)> Drop for AdmissionGuard<C> {
    fn drop(&mut self) {
        if let Some(mut cleanup) = self.cleanup.take() {
            let action = if self.transferred {
                CleanupAction::CancelTransferredWorker
            } else {
                CleanupAction::ReleaseBeforeWorker
            };
            cleanup(action);
        }
    }
}

pub(super) struct ImageExportAdmission<C: FnMut(CleanupAction)> {
    pub(super) job: ImageExportJob,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(super) parent: PathBuf,
    pub(super) guard: AdmissionGuard<C>,
}

pub(super) fn admit_with_picker<C, P>(
    id: u32,
    claim_task: impl FnOnce(u32) -> Reply<ImageExportJob>,
    cleanup: C,
    picker: P,
) -> Reply<Option<ImageExportAdmission<C>>>
where
    C: FnMut(CleanupAction),
    P: FnOnce() -> Reply<Option<PathBuf>>,
{
    let job = claim_task(id)?;
    let guard = AdmissionGuard::new(cleanup);
    let Some(parent) = picker()? else {
        return Ok(None);
    };
    Ok(Some(ImageExportAdmission { job, parent, guard }))
}

#[cfg(any(not(windows), test))]
pub(super) fn reject_unsupported_after_admission<C>(
    id: u32,
    claim_task: impl FnOnce(u32) -> Reply<ImageExportJob>,
    cleanup: C,
) -> Reply<()>
where
    C: FnMut(CleanupAction),
{
    let _ = admit_with_picker(id, claim_task, cleanup, || -> Reply<Option<PathBuf>> {
        Err(WINDOWS_ONLY.into())
    });
    Err(WINDOWS_ONLY.into())
}

#[cfg(windows)]
struct WorkerReservation {
    app: tauri::AppHandle,
    id: u32,
}

#[cfg(windows)]
impl Drop for WorkerReservation {
    fn drop(&mut self) {
        if let Ok(mut workspace) = self.app.state::<Store>().lock() {
            workspace.release_export(self.id);
        }
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(super) fn prepare_image_export(
    format: Option<ImageExportFormat>,
    state: State<'_, Store>,
) -> Reply<Handle> {
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    prepare(&mut workspace, format)
}

#[tauri::command(rename_all = "snake_case")]
pub(super) fn cancel_image_export(id: u32, state: State<'_, Store>) -> Reply<bool> {
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    Ok(cancel(&mut workspace, id))
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn export_images_document(
    id: u32,
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<Option<Receipt>> {
    #[cfg(not(windows))]
    {
        let _ = app;
        let store = state.inner();
        return reject_unsupported_after_admission(
            id,
            |id| {
                let mut workspace = store.lock().map_err(|_| "文档状态不可用")?;
                claim(&mut workspace, id)
            },
            |action| {
                if let Ok(mut workspace) = store.lock() {
                    cleanup(&mut workspace, id, action);
                }
            },
        )
        .map(|()| None);
    }
    #[cfg(windows)]
    {
        use tauri_plugin_dialog::DialogExt;

        let cleanup_app = app.clone();
        let Some(admission) = admit_with_picker(
            id,
            |id| {
                let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
                claim(&mut workspace, id)
            },
            move |action| {
                if let Ok(mut workspace) = cleanup_app.state::<Store>().lock() {
                    cleanup(&mut workspace, id, action);
                }
            },
            || {
                app.dialog()
                    .file()
                    .blocking_pick_folder()
                    .map(|parent| {
                        parent
                            .into_path()
                            .map_err(|_| "不能访问图片导出位置。".to_string())
                    })
                    .transpose()
            },
        )?
        else {
            return Ok(None);
        };
        let ImageExportAdmission {
            job,
            parent,
            mut guard,
        } = admission;
        let worker_app = app.clone();
        let validation_app = app.clone();
        let worker = tauri::async_runtime::spawn_blocking(move || {
            let _reservation = WorkerReservation {
                app: worker_app,
                id,
            };
            run_job(&job, &parent, || {
                let state = validation_app.state::<Store>();
                let workspace = state.lock().map_err(|_| "文档状态不可用")?;
                validate_current(&workspace, &job)
            })
        });
        guard.transfer_to_worker();
        let result = worker
            .await
            .map_err(|error| format!("图片导出工作线程失败：{error}"))?;
        guard.disarm();
        result.map(Some)
    }
}
