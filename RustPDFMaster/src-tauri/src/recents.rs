//! App-profile metadata only. Canonical sources never cross IPC or enter exports.
use super::*;
use std::{
    collections::BTreeSet,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const STORE_ERROR: &str = "最近记录存储不可用；原记录保持不变。";
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    version: u8,
    next_id: u64,
    entries: Vec<Record>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            entries: vec![],
        }
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    id: u32,
    path: PathBuf,
    last_opened: Option<u64>,
    favorite: bool,
}
#[derive(Serialize)]
pub(super) struct Entry {
    pub id: u32,
    name: String,
    pub last_opened: Option<u64>,
    pub favorite: bool,
    exists: bool,
}
#[derive(Serialize)]
pub(super) struct View {
    pub entries: Vec<Entry>,
    pub warning: Option<String>,
}
pub(super) struct RecentStore {
    path: PathBuf,
    data: Snapshot,
    blocked: bool,
    warning: Option<String>,
}
fn reject_target(path: &Path) -> std::io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            Err(std::io::Error::other("nonregular metadata"))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
impl RecentStore {
    pub fn load(path: PathBuf) -> Self {
        let loaded = (|| -> std::io::Result<Snapshot> {
            reject_target(&path)?;
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Snapshot::default());
                }
                Err(e) => return Err(e),
            };
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other("nonregular metadata"));
            }
            let mut bytes = vec![];
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other("metadata budget"));
            }
            let data: Snapshot = serde_json::from_slice(&bytes)?;
            Self::validate(&data)?;
            Ok(data)
        })();
        match loaded {
            Ok(data) => Self {
                path,
                data,
                blocked: false,
                warning: None,
            },
            Err(_) => Self {
                path,
                data: Snapshot::default(),
                blocked: true,
                warning: Some(STORE_ERROR.into()),
            },
        }
    }
    fn validate(data: &Snapshot) -> std::io::Result<()> {
        let mut ids = BTreeSet::new();
        let mut paths = BTreeSet::new();
        if data.version != 1
            || data.next_id == 0
            || data.next_id > u64::from(u32::MAX) + 1
            || data.entries.len() > 10_000
        {
            return Err(std::io::Error::other("invalid metadata schema"));
        }
        for e in &data.entries {
            if e.id == 0
                || u64::from(e.id) >= data.next_id
                || !ids.insert(e.id)
                || !e.path.is_absolute()
                || e.path.as_os_str().len() > 8192
                || e.path.file_name().is_none()
                || !paths.insert(&e.path)
                || (!e.favorite && e.last_opened.is_none())
            {
                return Err(std::io::Error::other("invalid metadata entry"));
            }
        }
        if data.entries.iter().filter(|e| !e.favorite).count() > 50 {
            return Err(std::io::Error::other("invalid recent limit"));
        }
        Ok(())
    }
    pub fn view(&self) -> View {
        let mut entries: Vec<_> = self
            .data
            .entries
            .iter()
            .map(|e| Entry {
                id: e.id,
                name: e
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                last_opened: e.last_opened,
                favorite: e.favorite,
                exists: e.path.is_file(),
            })
            .collect();
        entries.sort_by_key(|e| (std::cmp::Reverse(e.last_opened), e.id));
        View {
            entries,
            warning: self.warning.clone(),
        }
    }
    pub fn snapshot(&self) -> Reply<Snapshot> {
        if self.blocked {
            Err(STORE_ERROR.into())
        } else {
            Ok(self.data.clone())
        }
    }
    pub fn source(&self, id: u32) -> Reply<PathBuf> {
        self.snapshot()?;
        self.data
            .entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.path.clone())
            .ok_or("记录已移除，请刷新列表。".into())
    }
    pub fn record(&mut self, source: &Path, at: u64) -> Reply<()> {
        let mut data = self.snapshot()?;
        let path = source
            .canonicalize()
            .map_err(|_| "文档已打开，但无法记录最近文件。")?;
        let at = data
            .entries
            .iter()
            .filter_map(|e| e.last_opened)
            .max()
            .map_or(at, |old| at.max(old.saturating_add(1)));
        if let Some(e) = data.entries.iter_mut().find(|e| e.path == path) {
            e.last_opened = Some(at);
        } else {
            let id = u32::try_from(data.next_id).map_err(|_| "最近记录编号已耗尽。")?;
            data.next_id += 1;
            data.entries.push(Record {
                id,
                path,
                last_opened: Some(at),
                favorite: false,
            });
        }
        Self::trim(&mut data);
        self.commit(data)
    }
    fn trim(data: &mut Snapshot) {
        let mut recent: Vec<_> = data
            .entries
            .iter()
            .filter(|e| !e.favorite && e.last_opened.is_some())
            .collect();
        recent.sort_by_key(|e| (std::cmp::Reverse(e.last_opened), e.id));
        let keep: BTreeSet<_> = recent.into_iter().take(50).map(|e| e.id).collect();
        data.entries.retain(|e| e.favorite || keep.contains(&e.id));
    }
    pub fn toggle(&mut self, id: u32) -> Reply<View> {
        let mut data = self.snapshot()?;
        let e = data
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("记录已移除，请刷新列表。")?;
        e.favorite = !e.favorite;
        Self::trim(&mut data);
        self.commit(data)?;
        Ok(self.view())
    }
    pub fn remove(&mut self, id: u32) -> Reply<View> {
        let mut data = self.snapshot()?;
        let e = data
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("记录已移除，请刷新列表。")?;
        e.last_opened = None;
        data.entries
            .retain(|e| e.favorite || e.last_opened.is_some());
        self.commit(data)?;
        Ok(self.view())
    }
    pub fn clear_confirmed(&mut self, snapshot: &Snapshot, confirmed: bool) -> Reply<Option<View>> {
        if !confirmed {
            return Ok(None);
        }
        if &self.snapshot()? != snapshot {
            return Err("最近记录已改变，未清除；请重新确认。".into());
        }
        let mut data = snapshot.clone();
        data.entries.retain(|e| e.favorite);
        for e in &mut data.entries {
            e.last_opened = None;
        }
        self.commit(data)?;
        Ok(Some(self.view()))
    }
    fn commit(&mut self, data: Snapshot) -> Reply<()> {
        let result = (|| -> std::io::Result<()> {
            Self::validate(&data)?;
            let bytes = serde_json::to_vec(&data)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other("metadata budget"));
            }
            reject_target(&self.path)?;
            let parent = self
                .path
                .parent()
                .ok_or_else(|| std::io::Error::other("metadata parent"))?;
            let mut staged = tempfile::NamedTempFile::new_in(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                staged
                    .as_file()
                    .set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            staged.write_all(&bytes)?;
            staged.flush()?;
            staged.as_file().sync_all()?;
            reject_target(&self.path)?;
            #[cfg(windows)]
            {
                let path = staged.into_temp_path();
                storage::move_file_write_through(&path, &self.path, true)?;
            }
            #[cfg(not(windows))]
            {
                staged.persist(&self.path).map_err(|e| e.error)?;
            }
            // No fallible step after publication: metadata and memory must agree.
            Ok(())
        })();
        if result.is_err() {
            self.warning = Some(STORE_ERROR.into());
            return Err(STORE_ERROR.into());
        }
        self.data = data;
        self.warning = None;
        Ok(())
    }
}
impl Workspace {
    fn recent_store(&mut self) -> Reply<&mut RecentStore> {
        self.recents.as_mut().ok_or(STORE_ERROR.into())
    }
    pub(super) fn record_source(&mut self, path: &Path) {
        if let Some(store) = self.recents.as_mut() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64);
            if store.record(path, now).is_err() && store.warning.is_none() {
                store.warning = Some(STORE_ERROR.into());
            }
        }
    }
    pub(super) fn source_document(&self, path: &Path) -> Option<u32> {
        self.source_documents
            .iter()
            .find_map(|(id, p)| (p == path).then_some(*id))
    }
    pub(super) fn open_source(&mut self, path: &Path) -> Reply<DocumentView> {
        self.ensure_editor_idle()?;
        let path = path
            .canonicalize()
            .map_err(|_| "无法访问此文件；文档保持不变。")?;
        if !path.is_file() {
            return Err("无法访问此文件；文档保持不变。".into());
        }
        if let Some(id) = self.source_document(&path) {
            self.activate_document(id)?;
        } else {
            let mut bytes = vec![];
            File::open(&path)
                .map_err(|_| "无法读取此文件。")?
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "无法读取此文件。")?;
            let session = Session::open(&bytes).map_err(|e| e.to_string())?;
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into();
            self.replace_document(session, name)?;
            self.source_documents
                .insert(self.active_document_id, path.clone());
        }
        self.record_source(&path);
        self.view()
    }
    pub(super) fn recent_warning(&self) -> Option<String> {
        self.recents.as_ref().and_then(|s| s.warning.clone())
    }
}
#[tauri::command]
pub(super) async fn recent_documents(state: State<'_, Store>) -> Reply<View> {
    Ok(state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .recent_store()?
        .view())
}
#[tauri::command]
pub(super) async fn open_recent_document(
    id: u32,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.ensure_editor_idle()?;
    let path = w.recent_store()?.source(id)?;
    w.open_source(&path).map(Some)
}
#[tauri::command]
pub(super) async fn toggle_recent_favorite(id: u32, state: State<'_, Store>) -> Reply<View> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.ensure_editor_idle()?;
    w.recent_store()?.toggle(id)
}
#[tauri::command]
pub(super) async fn remove_recent_document(id: u32, state: State<'_, Store>) -> Reply<View> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.ensure_editor_idle()?;
    w.recent_store()?.remove(id)
}
#[tauri::command]
pub(super) async fn clear_recent_documents(
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<Option<View>> {
    use tauri_plugin_dialog::{MessageDialogButtons, MessageDialogResult};
    let snapshot = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        w.ensure_editor_idle()?;
        w.recent_store()?.snapshot()?
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dialog = app
        .dialog()
        .message("不会删除原文件；收藏会保留。")
        .title("清除最近记录？")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "清除".into(),
            "取消".into(),
        ));
    #[cfg(desktop)]
    let dialog = dialog.parent(&app.get_webview_window("main").ok_or("确认窗口不存在。")?);
    dialog.show_with_result(move |result| {
        let _ = sender.send(result);
    });
    let result = receiver
        .await
        .map_err(|_| "系统确认窗口不可用；记录未清除。")?;
    let confirmed = matches!(&result, MessageDialogResult::Ok)
        || matches!(&result, MessageDialogResult::Custom(label) if label == "清除");
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    w.ensure_editor_idle()?;
    w.recent_store()?.clear_confirmed(&snapshot, confirmed)
}
