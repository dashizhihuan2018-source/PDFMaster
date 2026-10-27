use super::*;
use std::path::PathBuf;
use std::time::Instant;
static READ_SLOT: Mutex<()> = Mutex::new(());
fn read_drop_files(paths: &[PathBuf]) -> Reply<Vec<Vec<u8>>> {
    with_read_slot(|| file_import::read(paths))
}
pub(super) fn with_read_slot<T>(read: impl FnOnce() -> Reply<T>) -> Reply<T> {
    // Keep the slot until the worker closure actually exits. Image import may
    // cooperate at safe checkpoints, but cancellation never releases it early.
    let _slot = READ_SLOT
        .try_lock()
        .map_err(|_| "上一批文件读取尚在结束，请稍后重试；本次未修改文档。")?;
    read()
}

fn logical_point(x: f64, y: f64, scale: f64, apple_points: bool) -> Option<(f64, f64)> {
    if !x.is_finite() || !y.is_finite() || !scale.is_finite() || scale <= 0. {
        return None;
    }
    // Locked Wry 0.57.1 WKWebView drag_drop.rs already supplies AppKit points.
    // Its Windows backend supplies physical client pixels. Reaudit on upgrade.
    let divisor = if apple_points { 1. } else { scale };
    Some((x / divisor, y / divisor))
}

#[derive(Default)]
pub(super) struct Broker {
    next: u32,
    pub(super) pending: Option<Pending>,
    pub(super) running: Option<(u32, u32)>,
    next_listener: u32,
    channel: Option<(u32, tauri::ipc::Channel<Notice>)>,
}
pub(super) struct Pending {
    id: u32,
    document: u32,
    epoch: u64,
    snapshot: Option<Vec<u8>>,
    created: Instant,
    paths: Vec<PathBuf>,
}
impl Workspace {
    fn begin_home_file_drop(&mut self, drop: u32) -> Reply<(u32, Vec<PathBuf>)> {
        self.ensure_editor_idle()?;
        if self.recovery_fault && self.session.is_none() {
            return Err("原草稿未能恢复，请先安全处理原草稿。".into());
        }
        if self.inactive_documents.len() + usize::from(self.session.is_some()) >= 16 {
            return Err("最多同时打开16份PDF，请先保存并关闭部分标签。".into());
        }
        let p = self
            .file_drop
            .pending
            .as_ref()
            .filter(|p| p.id == drop)
            .ok_or("拖入文件已失效，请重新拖入。")?;
        let snapshot = self
            .session
            .as_ref()
            .map(Session::bytes)
            .transpose()
            .map_err(|e| e.to_string())?;
        if p.document != self.active_document_id
            || p.epoch != self.document_epoch
            || p.snapshot != snapshot
            || p.created.elapsed().as_secs() >= 30
        {
            self.file_drop.pending = None;
            return Err("拖放期间会话已改变或超时，请重新拖入；未创建文档。".into());
        }
        let r = self
            .next_export_id
            .checked_add(1)
            .ok_or("文件操作编号已耗尽，请保存后重启。")?;
        self.next_export_id = r;
        self.export_reservation = Some(r);
        let p = self.file_drop.pending.take().ok_or("拖放已失效。")?;
        self.file_drop.running = Some((drop, r));
        Ok((r, p.paths))
    }
    fn finish_home_file_drop(
        &mut self,
        drop: u32,
        reservation: u32,
        name: String,
        files: &[&[u8]],
        single_pdf: bool,
    ) -> Reply<DocumentView> {
        if self.file_drop.running != Some((drop, reservation))
            || self.export_reservation != Some(reservation)
        {
            return Err("导入已取消或会话已改变，未创建文档。".into());
        }
        let session = if single_pdf && files.len() == 1 {
            Session::open(files[0])
        } else {
            Session::from_imports(files)
        }
        .map_err(|e| e.to_string())?;
        self.file_drop.running = None;
        self.release_export(reservation);
        self.replace_document(session, name)
    }
    fn offer_file_drop(&mut self, paths: Vec<PathBuf>) -> Reply<u32> {
        self.file_drop.pending = None;
        self.ensure_editor_idle()?;
        if paths.is_empty()
            || paths.len() > pdfmaster_core::MAX_INSERT_FILES
            || paths
                .iter()
                .any(|p| !p.is_absolute() || p.as_os_str().len() > 8192)
        {
            return Err("请拖入1–32份本地PDF或支持的图片；整批未修改文档。".into());
        }
        let snapshot = self
            .session
            .as_ref()
            .map(Session::bytes)
            .transpose()
            .map_err(|e| e.to_string())?;
        let id = self
            .file_drop
            .next
            .checked_add(1)
            .ok_or("拖放编号已耗尽，请安全保存后重启。")?;
        self.file_drop.next = id;
        self.file_drop.pending = Some(Pending {
            id,
            document: self.active_document_id,
            epoch: self.document_epoch,
            snapshot,
            created: Instant::now(),
            paths,
        });
        Ok(id)
    }
    fn begin_file_drop(
        &mut self,
        drop: u32,
        document: u32,
        index: usize,
    ) -> Reply<(u32, Vec<PathBuf>)> {
        self.ensure_editor_idle()?;
        let pending = self
            .file_drop
            .pending
            .as_ref()
            .filter(|p| p.id == drop)
            .ok_or("拖入文件已失效，请重新拖入；文档未修改。")?;
        if pending.document != document
            || document != self.active_document_id
            || pending.epoch != self.document_epoch
            || pending.created.elapsed().as_secs() >= 30
            || self
                .session
                .as_ref()
                .map(Session::bytes)
                .transpose()
                .map_err(|e| e.to_string())?
                != pending.snapshot
        {
            self.file_drop.pending = None;
            return Err("拖放期间文档已改变或等待超时，请重新拖入；文档未修改。".into());
        }
        let reservation = self.begin_insertion(index)?;
        let pending = self.file_drop.pending.take().ok_or("拖放已失效。")?;
        self.file_drop.running = Some((drop, reservation));
        Ok((reservation, pending.paths))
    }
    fn finish_file_drop(
        &mut self,
        drop: u32,
        reservation: u32,
        index: usize,
        files: &[&[u8]],
    ) -> Reply<DocumentView> {
        if self.file_drop.running != Some((drop, reservation))
            || self.export_reservation != Some(reservation)
        {
            return Err("导入已取消或会话已改变，未插入页面。".into());
        }
        self.file_drop.running = None;
        self.finish_insertions(reservation, index, files)
    }
    fn cancel_file_drop(&mut self, drop: u32) -> bool {
        if self
            .file_drop
            .pending
            .as_ref()
            .is_some_and(|p| p.id == drop)
        {
            self.file_drop.pending = None;
            return true;
        }
        if let Some((_, reservation)) = self.file_drop.running.filter(|(id, _)| *id == drop) {
            self.file_drop.running = None;
            self.release_export(reservation);
            return true;
        }
        false
    }
}

#[derive(Clone, Serialize)]
pub(super) struct Notice {
    kind: &'static str,
    id: Option<u32>,
    x: f64,
    y: f64,
    error: Option<String>,
}

pub(super) fn event(window: &tauri::Window, event: &tauri::DragDropEvent) {
    use tauri::DragDropEvent;
    if window.label() != "main" {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.);
    let position = match event {
        DragDropEvent::Enter { position, .. }
        | DragDropEvent::Over { position }
        | DragDropEvent::Drop { position, .. } => logical_point(
            position.x,
            position.y,
            scale,
            cfg!(any(target_os = "macos", target_os = "ios")),
        ),
        _ => Some((0., 0.)),
    };
    let Some((x, y)) = position else {
        return;
    };
    let state = window.state::<Store>();
    let Ok(mut w) = state.lock() else {
        return;
    };
    let mut notice = Notice {
        kind: "leave",
        id: None,
        x,
        y,
        error: None,
    };
    match event {
        DragDropEvent::Enter { .. } => {
            w.file_drop.pending = None;
            notice.kind = "over";
        }
        DragDropEvent::Over { .. } => notice.kind = "over",
        DragDropEvent::Drop { paths, .. } => {
            notice.kind = "drop";
            match w.offer_file_drop(paths.clone()) {
                Ok(id) => notice.id = Some(id),
                Err(error) => notice.error = Some(error),
            }
        }
        // Wry may send Leave immediately after Drop; do not revoke the notice.
        _ => {}
    }
    if let Some((_, channel)) = &w.file_drop.channel {
        let _ = channel.send(notice);
    }
}

#[tauri::command]
pub(super) async fn register_file_drop_channel(
    state: State<'_, Store>,
    channel: tauri::ipc::Channel<Notice>,
) -> Reply<u32> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    let id = w
        .file_drop
        .next_listener
        .checked_add(1)
        .ok_or("拖放监听编号已耗尽。")?;
    w.file_drop.next_listener = id;
    w.file_drop.channel = Some((id, channel));
    w.file_drop.pending = None;
    Ok(id)
}
#[tauri::command]
pub(super) async fn unregister_file_drop_channel(state: State<'_, Store>, id: u32) -> Reply<()> {
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    if w.file_drop
        .channel
        .as_ref()
        .is_some_and(|(current, _)| *current == id)
    {
        w.file_drop.channel = None;
        w.file_drop.pending = None;
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn import_file_drop(
    state: State<'_, Store>,
    id: u32,
    document: u32,
    index: usize,
) -> Reply<Option<DocumentView>> {
    let (reservation, paths) = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_file_drop(id, document, index)?;
    let guard = ExportReservation {
        store: state.inner(),
        id: reservation,
    };
    let files = tauri::async_runtime::spawn_blocking(move || read_drop_files(&paths))
        .await
        .map_err(|_| "无法读取拖入文件；整批未修改文档。")??;
    let refs: Vec<_> = files.iter().map(Vec::as_slice).collect();
    let result = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_file_drop(id, reservation, index, &refs)
        .map(Some);
    drop(guard);
    result
}

#[tauri::command]
pub(super) async fn cancel_file_drop(state: State<'_, Store>, id: u32) -> Reply<bool> {
    Ok(state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_file_drop(id))
}

#[tauri::command]
pub(super) async fn open_file_drop(
    state: State<'_, Store>,
    id: u32,
) -> Reply<Option<DocumentView>> {
    let (reservation, paths) = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_home_file_drop(id)?;
    let guard = ExportReservation {
        store: state.inner(),
        id: reservation,
    };
    let single_pdf = paths.len() == 1
        && paths[0]
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("pdf"));
    let name = if paths.len() == 1 {
        let stem = paths[0].file_stem().unwrap_or_default().to_string_lossy();
        format!("{}.pdf", stem.chars().take(512).collect::<String>())
    } else {
        "文件合并.pdf".into()
    };
    let files = tauri::async_runtime::spawn_blocking(move || read_drop_files(&paths))
        .await
        .map_err(|_| "无法读取拖入文件；未创建文档。")??;
    let refs: Vec<_> = files.iter().map(Vec::as_slice).collect();
    let result = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_home_file_drop(id, reservation, name, &refs, single_pdf)
        .map(Some);
    drop(guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn home_merge_creates_an_independent_unsaved_tab_and_never_appends_to_hidden_document() {
        let mut w = workspace();
        let old = w.session.as_ref().unwrap().bytes().unwrap();
        let (_dir, paths) = paths();
        let files = read_insertion_files(&paths).unwrap();
        let id = w.offer_file_drop(paths).unwrap();
        let (r, _) = w.begin_home_file_drop(id).unwrap();
        let refs: Vec<_> = files.iter().map(Vec::as_slice).collect();
        let view = w
            .finish_home_file_drop(id, r, "文件合并.pdf".into(), &refs, false)
            .unwrap();
        assert_eq!(view.info.page_count, 3);
        assert!(view.info.dirty && !view.info.can_undo && !view.info.can_redo);
        assert_eq!(w.inactive_documents.len(), 1);
        assert_eq!(w.inactive_documents[0].session.bytes().unwrap(), old);
        let id = w
            .offer_file_drop(vec![_dir.path().join("synthetic.pdf")])
            .unwrap();
        let (r, _) = w.begin_home_file_drop(id).unwrap();
        assert!(w.cancel_file_drop(id));
        assert!(
            w.finish_home_file_drop(id, r, "cancelled.pdf".into(), &refs, false)
                .is_err()
        );
        assert_eq!(w.inactive_documents.len(), 1);
    }
    #[test]
    fn home_first_import_has_no_session_and_late_invalid_batch_creates_no_tab() {
        let mut w = Workspace::default();
        let journal = tempfile::tempdir().unwrap();
        w.journal = Some(recovery::Journal::new(journal.path().join("draft.json")));
        let (_dir, paths) = paths();
        let files = read_insertion_files(&paths).unwrap();
        let id = w.offer_file_drop(paths.clone()).unwrap();
        let (r, _) = w.begin_home_file_drop(id).unwrap();
        assert!(
            w.finish_home_file_drop(id, r, "bad.pdf".into(), &[&files[0], b"bad"], false)
                .is_err()
        );
        w.release_export(r);
        assert!(w.session.is_none() && w.inactive_documents.is_empty());
        let id = w.offer_file_drop(paths).unwrap();
        let (r, _) = w.begin_home_file_drop(id).unwrap();
        let view = w
            .finish_home_file_drop(id, r, "first.pdf".into(), &[&files[0]], true)
            .unwrap();
        assert_eq!(view.info.page_count, 2);
        assert!(
            !view.info.dirty && !view.info.can_undo,
            "a single PDF drop follows the original open path, not a dirty merge"
        );
    }
    #[test]
    fn cancelled_slow_read_cannot_accumulate_unbounded_parallel_readers() {
        let _guard = READ_SLOT.lock().unwrap();
        let (_dir, paths) = paths();
        assert!(read_drop_files(&paths).is_err());
    }
    #[test]
    fn locked_wry_apple_points_are_not_scaled_twice_but_windows_physical_pixels_are() {
        assert_eq!(logical_point(620., 420., 2., true), Some((620., 420.)));
        assert_eq!(logical_point(620., 420., 2., false), Some((310., 210.)));
        assert_eq!(logical_point(f64::NAN, 420., 2., true), None);
        assert_eq!(logical_point(620., 420., 0., false), None);
    }
    fn workspace() -> Workspace {
        let mut w = Workspace::default();
        w.replace_document(
            Session::blank(300., 400., 3).unwrap(),
            "synthetic.pdf".into(),
        )
        .unwrap();
        w
    }
    fn paths() -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![dir.path().join("first.pdf"), dir.path().join("second.pdf")];
        std::fs::write(
            &paths[0],
            Session::blank(111., 222., 2).unwrap().bytes().unwrap(),
        )
        .unwrap();
        std::fs::write(
            &paths[1],
            Session::blank(333., 444., 1).unwrap().bytes().unwrap(),
        )
        .unwrap();
        (dir, paths)
    }
    #[test]
    fn native_drop_is_one_shot_ordered_and_inserts_as_one_exact_undo() {
        let mut w = workspace();
        let (_dir, paths) = paths();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let id = w.offer_file_drop(paths.clone()).unwrap();
        let (reservation, selected) = w.begin_file_drop(id, w.active_document_id, 1).unwrap();
        assert_eq!(selected, paths);
        assert!(w.begin_file_drop(id, w.active_document_id, 1).is_err());
        let files = read_insertion_files(&selected).unwrap();
        let refs: Vec<_> = files.iter().map(Vec::as_slice).collect();
        let view = w.finish_file_drop(id, reservation, 1, &refs).unwrap();
        assert_eq!(view.info.page_count, 6);
        assert_eq!(
            view.info.pages.iter().map(|p| p.width).collect::<Vec<_>>(),
            [300., 111., 111., 333., 300., 300.]
        );
        assert!(w.file_drop.running.is_none());
        assert!(w.export_reservation.is_none());
        let after = w.session.as_ref().unwrap().bytes().unwrap();
        let s = w.session.as_mut().unwrap();
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), before);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), after);
    }
    #[test]
    fn stale_expired_foreign_and_bad_destination_drops_do_not_reserve_or_edit() {
        let mut w = workspace();
        let (_dir, paths) = paths();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        for kind in 0..4 {
            let id = w.offer_file_drop(paths.clone()).unwrap();
            if kind == 0 {
                w.document_epoch += 1;
            }
            if kind == 1 {
                w.file_drop.pending.as_mut().unwrap().created =
                    Instant::now() - std::time::Duration::from_secs(31);
            }
            let doc = w.active_document_id + u32::from(kind == 2);
            assert!(
                w.begin_file_drop(id, doc, if kind == 3 { 4 } else { 1 })
                    .is_err()
            );
            assert!(w.export_reservation.is_none());
            assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        }
    }
    #[test]
    fn invalid_batch_is_atomic_and_cancelled_import_cannot_apply_late_bytes() {
        let mut w = workspace();
        let (_dir, paths) = paths();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let files = read_insertion_files(&paths).unwrap();
        let id = w.offer_file_drop(paths.clone()).unwrap();
        let (r, _) = w.begin_file_drop(id, w.active_document_id, 0).unwrap();
        assert!(
            w.finish_file_drop(id, r, 0, &[&files[0], b"invalid PDF"])
                .is_err()
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        let id = w.offer_file_drop(paths).unwrap();
        let (r, _) = w.begin_file_drop(id, w.active_document_id, 0).unwrap();
        assert!(!w.cancel_file_drop(id + 1));
        assert!(w.export_reservation.is_some());
        assert!(w.cancel_file_drop(id));
        assert!(!w.cancel_file_drop(id));
        assert!(w.export_reservation.is_none());
        assert!(w.finish_file_drop(id, r, 0, &[&files[0]]).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }
    #[test]
    fn new_offer_invalidates_old_and_paths_never_arrive_from_an_ipc_argument() {
        let mut w = workspace();
        let (_dir, paths) = paths();
        let id = w.offer_file_drop(paths.clone()).unwrap();
        assert!(
            w.offer_file_drop(vec![PathBuf::from("relative.pdf")])
                .is_err()
        );
        assert!(w.begin_file_drop(id, w.active_document_id, 0).is_err());
        assert!(w.offer_file_drop(vec![]).is_err());
        assert!(w.offer_file_drop(vec![paths[0].clone(); 33]).is_err());
        let id = w.offer_file_drop(paths).unwrap();
        assert!(w.cancel_file_drop(id));
        assert!(w.begin_file_drop(id, w.active_document_id, 0).is_err());
    }
}
