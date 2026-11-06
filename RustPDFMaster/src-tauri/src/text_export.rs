//! Native-only, source-safe text derivatives; never acknowledges a PDF save.
use super::{Reply, Store, Workspace};
use pdfmaster_core::{Session, TextExportFormat};
use serde::Serialize;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::State;
#[cfg(windows)]
use tauri::{AppHandle, Manager};

const LIMIT: usize = 32 * 1024 * 1024;
const FAILURE: &str = "文本导出未完成；源文档和已有输出保持不变。";
const STALE: &str = "文本导出期间文档已改变；未发布输出。";

#[derive(Debug, Serialize)]
pub(super) struct TextExportReceipt {
    pub(super) format: TextExportFormat,
    pub(super) file_name: String,
    pub(super) total_bytes: u64,
    pub(super) warning: Option<String>,
}

pub(super) struct Job {
    pub(super) id: u32,
    document: u32,
    epoch: u64,
    bytes: Vec<u8>,
    sources: Vec<PathBuf>,
}
pub(super) fn begin(workspace: &mut Workspace) -> Reply<Job> {
    let (id, _, bytes) = workspace.begin_export()?;
    Ok(Job {
        id,
        document: workspace.active_document_id,
        epoch: workspace.document_epoch,
        bytes,
        sources: workspace.source_documents.values().cloned().collect(),
    })
}
pub(super) fn validate_current(workspace: &Workspace, job: &Job) -> Reply<()> {
    if workspace.export_reservation != Some(job.id)
        || workspace.active_document_id != job.document
        || workspace.document_epoch != job.epoch
        || workspace
            .session
            .as_ref()
            .ok_or(STALE)?
            .bytes()
            .map_err(|_| STALE.to_string())?
            != job.bytes
    {
        return Err(STALE.into());
    }
    Ok(())
}

pub(super) struct Reservation<C: FnOnce()>(Option<C>);
impl<C: FnOnce()> Drop for Reservation<C> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.0.take() {
            cleanup();
        }
    }
}
type Admission<C> = (Job, PathBuf, Reservation<C>);
pub(super) fn with_picker<C: FnOnce()>(
    job: Job,
    cleanup: C,
    picker: impl FnOnce() -> Reply<Option<PathBuf>>,
) -> Reply<Option<Admission<C>>> {
    let guard = Reservation(Some(cleanup));
    Ok(picker()?.map(|path| (job, path, guard)))
}

fn regular(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.is_file() && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_file() && !metadata.file_type().is_symlink()
    }
}
fn identity(file: &File) -> Reply<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata().map_err(|_| FAILURE)?;
        Ok((m.dev(), m.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: the live File owns this handle; Win32 initializes info on success.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(FAILURE.into());
        }
        // SAFETY: successful call initialized every member.
        let info = unsafe { info.assume_init() };
        Ok((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
}
fn parent_handle(path: &Path) -> Reply<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000);
    }
    options.open(path).map_err(|_| FAILURE.into())
}
fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
fn extension(format: TextExportFormat) -> &'static str {
    match format {
        TextExportFormat::PlainText | TextExportFormat::SummaryText => "txt",
        TextExportFormat::SummaryMarkdown => "md",
        TextExportFormat::SummaryHtml => "html",
    }
}
#[cfg(windows)]
fn suggested(format: TextExportFormat) -> &'static str {
    match format {
        TextExportFormat::PlainText => "文档.txt",
        TextExportFormat::SummaryText => "批注摘要.txt",
        TextExportFormat::SummaryMarkdown => "批注摘要.md",
        TextExportFormat::SummaryHtml => "批注摘要.html",
    }
}
fn file_bytes(file: &mut File) -> Reply<Vec<u8>> {
    let metadata = file.metadata().map_err(|_| FAILURE)?;
    if !regular(&metadata) || metadata.len() > LIMIT as u64 {
        return Err(FAILURE.into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| FAILURE)?;
    if bytes.len() > LIMIT {
        return Err(FAILURE.into());
    }
    Ok(bytes)
}
struct Existing {
    identity: (u64, u64),
    bytes: Vec<u8>,
    _handle: File,
}
pub(super) struct Target {
    path: PathBuf,
    parent: PathBuf,
    parent_identity: (u64, u64),
    _parent_handle: File,
    existing: Option<Existing>,
    sources: Vec<PathBuf>,
}
impl Target {
    pub(super) fn capture(
        path: &Path,
        format: TextExportFormat,
        sources: &[PathBuf],
    ) -> Reply<Self> {
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(extension(format)))
        {
            return Err("导出文件扩展名与所选格式不一致。".into());
        }
        let name = path.file_name().ok_or(FAILURE)?;
        let name_string = name.to_str().ok_or(FAILURE)?;
        if name_string.is_empty()
            || name_string.chars().any(|c| {
                c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            })
        {
            return Err(FAILURE.into());
        }
        let parent = path
            .parent()
            .ok_or(FAILURE)?
            .canonicalize()
            .map_err(|_| FAILURE)?;
        let path = parent.join(name);
        let parent_file = parent_handle(&parent)?;
        let existing = Self::existing(&path)?;
        let target = Self {
            path,
            parent,
            parent_identity: identity(&parent_file)?,
            _parent_handle: parent_file,
            existing,
            sources: sources.to_vec(),
        };
        target.protect_sources()?;
        Ok(target)
    }
    fn existing(path: &Path) -> Reply<Option<Existing>> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(FAILURE.into()),
            Ok(metadata) => {
                if !regular(&metadata) {
                    return Err(FAILURE.into());
                }
                let mut file = File::open(path).map_err(|_| FAILURE)?;
                let id = identity(&file)?;
                let bytes = file_bytes(&mut file)?;
                // Confirm the opened file is still the named regular target.
                if !regular(&fs::symlink_metadata(path).map_err(|_| FAILURE)?)
                    || identity(&File::open(path).map_err(|_| FAILURE)?)? != id
                {
                    return Err(FAILURE.into());
                }
                Ok(Some(Existing {
                    identity: id,
                    bytes,
                    _handle: file,
                }))
            }
        }
    }
    fn protect_sources(&self) -> Reply<()> {
        for source in &self.sources {
            let source_path = source
                .canonicalize()
                .map_err(|_| "无法验证源文档保护；未发布输出。")?;
            if same_path(&source_path, &self.path) {
                return Err("不能将文本输出写入源文档。".into());
            }
            if let Some(existing) = &self.existing {
                let file = File::open(&source_path).map_err(|_| FAILURE)?;
                if identity(&file)? == existing.identity {
                    return Err("不能将文本输出写入源文档的链接。".into());
                }
            }
        }
        Ok(())
    }
    fn revalidate(&self) -> Reply<()> {
        if self.parent.canonicalize().map_err(|_| FAILURE)? != self.parent
            || identity(&parent_handle(&self.parent)?)? != self.parent_identity
        {
            return Err(FAILURE.into());
        }
        let current = Self::existing(&self.path)?;
        match (&self.existing, current) {
            (None, None) => {}
            (Some(before), Some(after))
                if before.identity == after.identity && before.bytes == after.bytes => {}
            _ => return Err("导出目标已改变；未覆盖任何文件。".into()),
        }
        self.protect_sources()
    }
    pub(super) fn publish(
        self,
        format: TextExportFormat,
        text: &str,
        validate: impl FnOnce() -> Reply<()>,
    ) -> Reply<TextExportReceipt> {
        if text.len() > LIMIT {
            return Err("文本导出超过32MiB预算。".into());
        }
        let mut staged = tempfile::Builder::new()
            .prefix(".pdfmaster-text-")
            .tempfile_in(&self.parent)
            .map_err(|_| FAILURE)?;
        staged
            .write_all(text.as_bytes())
            .and_then(|()| staged.flush())
            .and_then(|()| staged.as_file().sync_all())
            .map_err(|_| FAILURE)?;
        let mut readback = staged.reopen().map_err(|_| FAILURE)?;
        if file_bytes(&mut readback)? != text.as_bytes() {
            return Err(FAILURE.into());
        }
        validate()?;
        self.revalidate()?;
        #[cfg(windows)]
        {
            let replace = self.existing.is_some();
            // Keep the captured identity alive through the final validation,
            // then close our destination handle: MoveFileExW cannot replace an
            // open destination even when that handle shares delete access.
            // An external open handle still fails safely without truncation.
            drop(self.existing);
            crate::storage::move_file_write_through(staged.path(), &self.path, replace)
                .map_err(|_| FAILURE)?;
        }
        #[cfg(not(windows))]
        {
            if self.existing.is_some() {
                staged.persist(&self.path).map_err(|_| FAILURE)?;
            } else {
                staged.persist_noclobber(&self.path).map_err(|_| FAILURE)?;
            }
        }
        // Publication succeeded. A later directory-sync limitation is a warning,
        // never a false error after the user's output has already changed.
        #[cfg(unix)]
        let warning = self
            ._parent_handle
            .sync_all()
            .err()
            .map(|_| "文本已导出；目录持久性尚未确认。".into());
        #[cfg(not(unix))]
        let warning = None;
        Ok(TextExportReceipt {
            format,
            file_name: self
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(FAILURE)?
                .to_owned(),
            total_bytes: text.len() as u64,
            warning,
        })
    }
}

pub(super) fn run_job(
    job: &Job,
    path: &Path,
    format: TextExportFormat,
    validate: impl FnOnce() -> Reply<()>,
) -> Reply<TextExportReceipt> {
    let target = Target::capture(path, format, &job.sources)?;
    let text = Session::open(&job.bytes)
        .and_then(|session| session.export_text(format))
        .map_err(|_| "无法生成文本导出；源文档保持不变。".to_string())?;
    target.publish(format, &text, validate)
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn export_text_document(
    format: TextExportFormat,
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<Option<TextExportReceipt>> {
    let job = {
        let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
        begin(&mut workspace)?
    };
    #[cfg(not(windows))]
    {
        let _ = (app, format);
        let _guard = Reservation(Some(|| {
            if let Ok(mut w) = state.lock() {
                w.release_export(job.id);
            }
        }));
        Err("文本导出的原生保存仅支持 Windows。".into())
    }
    #[cfg(windows)]
    {
        use tauri_plugin_dialog::DialogExt;
        let cleanup_app: AppHandle = app.clone();
        let id = job.id;
        let Some((job, path, guard)) = with_picker(
            job,
            move || {
                if let Ok(mut w) = cleanup_app.state::<Store>().lock() {
                    w.release_export(id);
                }
            },
            || {
                let title = if format == TextExportFormat::PlainText {
                    "导出纯文本"
                } else {
                    "导出批注摘要"
                };
                app.dialog()
                    .file()
                    .set_title(title)
                    .set_file_name(suggested(format))
                    .add_filter(extension(format), &[extension(format)])
                    .blocking_save_file()
                    .map(|file| {
                        file.into_path()
                            .map_err(|_| "不能访问文本导出位置。".to_string())
                    })
                    .transpose()
            },
        )?
        else {
            return Ok(None);
        };
        tauri::async_runtime::spawn_blocking(move || {
            // The guard lives inside the worker, including on dropped receivers
            // and unwind; no detached computation releases another job's token.
            let _guard = guard;
            run_job(&job, &path, format, || {
                let store = app.state::<Store>();
                let workspace = store.lock().map_err(|_| "文档状态不可用")?;
                validate_current(&workspace, &job)
            })
            .map(Some)
        })
        .await
        .map_err(|_| "文本导出工作线程失败；源文档保持不变。".to_string())?
    }
}
