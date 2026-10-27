use super::*;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
pub(super) struct ImportState {
    next: u32,
    pending: Option<Pending>,
}
struct Pending {
    id: u32,
    paths: Option<Vec<PathBuf>>,
    running: Option<Running>,
}
struct Running {
    reservation: u32,
    cancelled: Arc<AtomicBool>,
}
impl Workspace {
    fn start_image_picker(&mut self) -> Reply<u32> {
        self.ensure_editor_idle()?;
        if self
            .image_import
            .pending
            .as_ref()
            .is_some_and(|p| p.paths.is_none())
        {
            return Err("请先完成或取消现有图片文件选择面板。".into());
        }
        if self.recovery_fault && self.session.is_none() {
            return Err("请先安全处理未恢复草稿。".into());
        }
        if self.inactive_documents.len() + usize::from(self.session.is_some()) >= 16 {
            return Err("最多16份PDF，请先安全关闭部分标签。".into());
        }
        let id = self
            .image_import
            .next
            .checked_add(1)
            .ok_or("图片导入编号已耗尽。")?;
        self.image_import.next = id;
        self.image_import.pending = Some(Pending {
            id,
            paths: None,
            running: None,
        });
        Ok(id)
    }
    fn accept_image_selection(&mut self, id: u32, paths: Vec<PathBuf>) -> Reply<Vec<String>> {
        let p = self
            .image_import
            .pending
            .as_mut()
            .filter(|p| p.id == id && p.paths.is_none())
            .ok_or("图片选择会话已失效。")?;
        if paths.is_empty() || paths.len() > pdfmaster_core::MAX_INSERT_FILES {
            return Err("请选择1–32份PNG/JPEG/TIFF。".into());
        }
        let mut unique = vec![];
        for path in paths {
            if !path.is_absolute()
                || path.as_os_str().len() > 8192
                || !path.extension().is_some_and(|e| {
                    ["png", "jpg", "jpeg", "tif", "tiff"]
                        .iter()
                        .any(|s| e.eq_ignore_ascii_case(s))
                })
            {
                return Err("仅支持普通本地PNG/JPEG/TIFF；未创建文档。".into());
            }
            if !unique.contains(&path) {
                unique.push(path);
            }
        }
        let names = unique
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        p.paths = Some(unique);
        Ok(names)
    }
    fn cancel_image_tool(&mut self, id: u32) -> bool {
        if self
            .image_import
            .pending
            .as_ref()
            .is_none_or(|p| p.id != id)
        {
            return false;
        }
        let reservation = self
            .image_import
            .pending
            .as_ref()
            .and_then(|pending| pending.running.as_ref())
            .map(|running| {
                running.cancelled.store(true, Ordering::Release);
                running.reservation
            });
        self.image_import.pending = None;
        if let Some(reservation) = reservation {
            self.release_export(reservation);
        }
        true
    }
    fn begin_image_conversion(
        &mut self,
        id: u32,
        order: &[usize],
        layout: pdfmaster_core::ImageImportLayout,
    ) -> Reply<(u32, Vec<PathBuf>, String, Arc<AtomicBool>)> {
        self.ensure_editor_idle()?;
        layout.validate().map_err(|e| e.to_string())?;
        if self.recovery_fault && self.session.is_none() {
            return Err("请先安全处理未恢复草稿。".into());
        }
        if self.inactive_documents.len() + usize::from(self.session.is_some()) >= 16 {
            return Err("最多16份PDF，请先安全关闭部分标签。".into());
        }
        let p = self
            .image_import
            .pending
            .as_ref()
            .filter(|p| p.id == id && p.running.is_none())
            .ok_or("图片导入会话已失效。")?;
        let paths = p.paths.as_ref().ok_or("图片选择尚未结束。")?;
        let mut seen = std::collections::BTreeSet::new();
        if order.is_empty()
            || order.len() > paths.len()
            || order.iter().any(|i| *i >= paths.len() || !seen.insert(*i))
        {
            return Err("图片顺序无效；未创建文档。".into());
        }
        let selected: Vec<_> = order.iter().map(|i| paths[*i].clone()).collect();
        let name = if selected.len() == 1 {
            format!(
                "{}.pdf",
                selected[0]
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        } else {
            "图片合并.pdf".into()
        };
        let r = self
            .next_export_id
            .checked_add(1)
            .ok_or("图片任务编号已耗尽。")?;
        self.next_export_id = r;
        self.export_reservation = Some(r);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.image_import
            .pending
            .as_mut()
            .ok_or("图片会话失效。")?
            .running = Some(Running {
            reservation: r,
            cancelled: cancelled.clone(),
        });
        Ok((r, selected, name, cancelled))
    }
    fn finish_image_conversion(
        &mut self,
        id: u32,
        r: u32,
        name: String,
        files: &[&[u8]],
    ) -> Reply<DocumentView> {
        if self.export_reservation != Some(r)
            || self.image_import.pending.as_ref().is_none_or(|p| {
                p.id != id
                    || p.running
                        .as_ref()
                        .is_none_or(|running| running.reservation != r)
            })
        {
            return Err("图片导入已取消或会话改变，未创建文档。".into());
        }
        let session = Session::from_imports(files).map_err(|e| e.to_string())?;
        self.release_export(r);
        let view = self.replace_document(session, name)?;
        self.image_import.pending = None;
        Ok(view)
    }
}
pub(super) fn released(w: &mut Workspace, r: u32) {
    let Some(p) = w.image_import.pending.as_mut() else {
        return;
    };
    if p.running
        .as_ref()
        .is_some_and(|running| running.reservation == r)
    {
        let running = p.running.take().expect("matching image worker");
        running.cancelled.store(true, Ordering::Release);
    }
}

#[derive(serde::Serialize)]
pub(super) struct Selection {
    id: u32,
    names: Vec<String>,
}

// A dismissed dialog, failed callback, or dropped command must not leave an
// outstanding native picker. Successful selection hands ownership to the UI.
struct PickerReservation<'a> {
    store: &'a Store,
    id: u32,
    armed: bool,
}
impl Drop for PickerReservation<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Ok(mut w) = self.store.lock()
        {
            w.cancel_image_tool(self.id);
        }
    }
}
#[tauri::command]
pub(super) async fn prepare_image_import(
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<Option<Selection>> {
    let id = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .start_image_picker()?;
    let mut guard = PickerReservation {
        store: state.inner(),
        id,
        armed: true,
    };
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let picker = app
        .dialog()
        .file()
        .set_title("选择图片创建新 PDF")
        .add_filter(
            "图片（PNG/JPEG/TIFF）",
            &["png", "jpg", "jpeg", "tif", "tiff"],
        )
        .set_can_create_directories(false);
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let picker = picker.set_parent(
        &app.get_webview_window("main")
            .ok_or("文件选择窗口不存在。")?,
    );
    picker.pick_files(move |files| {
        let _ = sender.send(files);
    });
    let Some(files) = receiver
        .await
        .map_err(|_| "图片文件选择不可用；未创建文档。")?
    else {
        return Ok(None);
    };
    let paths = files
        .into_iter()
        .map(|f| {
            f.into_path()
                .map_err(|_| "无法访问所选图片；未创建文档。".to_string())
        })
        .collect::<Reply<Vec<_>>>()?;
    let names = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .accept_image_selection(id, paths)?;
    guard.armed = false;
    Ok(Some(Selection { id, names }))
}
#[tauri::command(rename_all = "snake_case")]
pub(super) async fn convert_image_import(
    id: u32,
    order: Vec<usize>,
    layout: pdfmaster_core::ImageImportLayout,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    let (reservation, paths, name, cancel) = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_image_conversion(id, &order, layout)?;
    let guard = ExportReservation {
        store: state.inner(),
        id: reservation,
    };
    let worker_cancel = cancel.clone();
    let files = tauri::async_runtime::spawn_blocking(move || {
        file_drop::with_read_slot(|| {
            file_import::read_images_cancellable(&paths, layout, || {
                worker_cancel.load(Ordering::Acquire)
            })
        })
    })
    .await
    .map_err(|_| "图片转换任务失败；未创建文档。")??;
    let refs: Vec<_> = files.iter().map(Vec::as_slice).collect();
    let result = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_image_conversion(id, reservation, name, &refs);
    drop(guard);
    result
}
#[tauri::command]
pub(super) fn cancel_image_import(id: u32, state: State<'_, Store>) -> Reply<bool> {
    Ok(state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_image_tool(id))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_source_layout_reaches_real_file_conversion_and_new_tab_geometry() {
        let (_dir, paths) = files();
        let mut w = Workspace::default();
        let id = w.start_image_picker().unwrap();
        w.accept_image_selection(id, paths).unwrap();
        let layout =
            serde_json::from_str(r#"{"paper":"source","landscape":false,"margin":0}"#).unwrap();
        let (r, ordered, name, _) = w.begin_image_conversion(id, &[0, 1], layout).unwrap();
        let converted = file_import::read_images(&ordered, layout).unwrap();
        let refs: Vec<_> = converted.iter().map(Vec::as_slice).collect();
        let reply = w.finish_image_conversion(id, r, name, &refs).unwrap();
        assert!(reply.info.dirty);
        assert_eq!(reply.info.page_count, 2);
        // files() uses the clipboard renderer's 3x, untagged PNG fixture.
        assert_eq!(
            (reply.info.pages[0].width, reply.info.pages[0].height),
            (240., 120.)
        );
        assert_eq!(
            (reply.info.pages[1].width, reply.info.pages[1].height),
            (120., 240.)
        );
    }
    #[test]
    fn failed_tab_admission_retains_configuration_for_retry() {
        let (_dir, paths) = files();
        let mut w = Workspace::default();
        let id = w.start_image_picker().unwrap();
        w.accept_image_selection(id, paths).unwrap();
        let layout = pdfmaster_core::ImageImportLayout::default();
        let (r, _, name, _) = w.begin_image_conversion(id, &[0], layout).unwrap();
        let pdf = Session::blank(100., 100., 1).unwrap().bytes().unwrap();
        w.next_document_id = u32::MAX;
        assert!(w.finish_image_conversion(id, r, name, &[&pdf]).is_err());
        w.release_export(r);
        w.next_document_id = 0;
        assert!(
            w.begin_image_conversion(id, &[0], layout).is_ok(),
            "a rejected new tab must not silently discard the still-visible configuration"
        );
    }
    fn files() -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![dir.path().join("first.png"), dir.path().join("second.png")];
        for (p, (w, h)) in paths.iter().zip([(80., 40.), (40., 80.)]) {
            std::fs::write(
                p,
                Session::blank(w, h, 1)
                    .unwrap()
                    .page_clipboard(&[0])
                    .unwrap()
                    .png,
            )
            .unwrap();
        }
        (dir, paths)
    }
    #[test]
    fn ordered_image_configuration_creates_a_new_dirty_tab_without_mutating_hidden_original() {
        let (dir, paths) = files();
        let mut w = Workspace {
            journal: Some(recovery::Journal::new(dir.path().join("draft.json"))),
            ..Workspace::default()
        };
        w.replace_document(Session::blank(100., 100., 1).unwrap(), "old.pdf".into())
            .unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let id = w.start_image_picker().unwrap();
        let names = w
            .accept_image_selection(
                id,
                vec![paths[0].clone(), paths[1].clone(), paths[0].clone()],
            )
            .unwrap();
        assert_eq!(
            names,
            ["first.png", "second.png"],
            "picker de-duplicates exact source paths, without exposing them"
        );
        let layout = pdfmaster_core::ImageImportLayout {
            paper: pdfmaster_core::ImagePaper::Pixels,
            landscape: false,
            margin: 0.,
        };
        let (r, ordered, name, _) = w.begin_image_conversion(id, &[1, 0], layout).unwrap();
        assert_eq!(ordered, [paths[1].clone(), paths[0].clone()]);
        assert!(w.ensure_editor_idle().is_err());
        let converted = file_import::read_images(&ordered, layout).unwrap();
        let refs: Vec<_> = converted.iter().map(Vec::as_slice).collect();
        let reply = w.finish_image_conversion(id, r, name, &refs).unwrap();
        assert!(reply.info.dirty && !reply.info.can_undo);
        assert_eq!(reply.info.page_count, 2);
        assert!(reply.info.pages[0].height > reply.info.pages[0].width);
        assert_eq!(w.inactive_documents[0].session.bytes().unwrap(), before);
    }
    #[test]
    fn cancel_revokes_conversion_and_old_cancel_cannot_release_new_job() {
        let (_dir, paths) = files();
        let mut w = Workspace::default();
        let id = w.start_image_picker().unwrap();
        w.accept_image_selection(id, paths.clone()).unwrap();
        let layout = pdfmaster_core::ImageImportLayout::default();
        let (r, _, name, old_cancel) = w.begin_image_conversion(id, &[0], layout).unwrap();
        assert!(!old_cancel.load(std::sync::atomic::Ordering::Acquire));
        let pdf = Session::blank(100., 100., 1).unwrap().bytes().unwrap();
        assert!(w.cancel_image_tool(id));
        assert!(old_cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(w.finish_image_conversion(id, r, name, &[&pdf]).is_err());
        assert!(w.session.is_none());
        let next = w.start_image_picker().unwrap();
        w.accept_image_selection(next, paths).unwrap();
        let (new, _, _, new_cancel) = w.begin_image_conversion(next, &[1], layout).unwrap();
        assert!(!new_cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(!w.cancel_image_tool(id));
        w.release_export(r);
        assert_eq!(w.export_reservation, Some(new));
        assert!(
            !new_cancel.load(std::sync::atomic::Ordering::Acquire),
            "stale cancel/release must not invalidate the new worker token"
        );
        w.release_export(new);
        assert!(new_cancel.load(std::sync::atomic::Ordering::Acquire));
        assert!(w.ensure_editor_idle().is_ok());
    }
    #[test]
    fn invalid_orders_and_margins_do_not_reserve_or_publish_and_failed_job_can_retry() {
        let (_dir, paths) = files();
        let mut w = Workspace::default();
        let id = w.start_image_picker().unwrap();
        assert!(
            w.start_image_picker().is_err(),
            "only one actual picker may remain outstanding"
        );
        w.accept_image_selection(id, paths).unwrap();
        let layout = pdfmaster_core::ImageImportLayout::default();
        for order in [vec![], vec![0, 0], vec![2]] {
            assert!(w.begin_image_conversion(id, &order, layout).is_err());
        }
        assert!(
            w.begin_image_conversion(
                id,
                &[0],
                pdfmaster_core::ImageImportLayout {
                    margin: 999.,
                    ..layout
                }
            )
            .is_err()
        );
        assert!(w.export_reservation.is_none());
        let (r, _, name, _) = w.begin_image_conversion(id, &[0], layout).unwrap();
        assert!(w.finish_image_conversion(id, r, name, &[b"bad"]).is_err());
        w.release_export(r);
        assert!(
            w.begin_image_conversion(id, &[1], layout).is_ok(),
            "conversion failure releases running state but retains configuration"
        );
    }
}
