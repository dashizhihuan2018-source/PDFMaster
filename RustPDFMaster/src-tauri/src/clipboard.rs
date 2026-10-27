use super::*;

const PAGES_TYPE: &str = "cn.nankong.pdfmaster.pages.pdf";
const MAX_PDF: usize = 32 * 1024 * 1024;

#[cfg(target_os = "macos")]
#[path = "clipboard_macos.rs"]
mod platform;
#[cfg(target_os = "ios")]
#[path = "clipboard_ios.rs"]
mod platform;
#[cfg(windows)]
#[path = "clipboard_windows.rs"]
mod platform;
#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
mod platform {
    use super::*;
    pub(super) fn write(_: &tauri::AppHandle, _: &pdfmaster_core::PageClipboard) -> Reply<()> {
        Err("此平台的页面剪贴板尚未实现。".into())
    }
    pub(super) fn read(_: &tauri::AppHandle) -> Reply<Vec<u8>> {
        Err("此平台的页面剪贴板尚未实现。".into())
    }
}

impl Workspace {
    fn finish_page_copy(
        &mut self,
        id: u32,
        pages: Vec<usize>,
        cut: bool,
        published: Reply<()>,
    ) -> Reply<Option<DocumentView>> {
        published?;
        if self.export_reservation != Some(id) {
            return Err("页面复制会话已改变，文档未剪切。".into());
        }
        self.release_export(id);
        if cut {
            self.session
                .as_mut()
                .ok_or("请先打开 PDF")?
                .apply(Edit::DeletePages { pages })
                .map_err(|e| e.to_string())?;
            self.checkpoint().map(Some)
        } else {
            Ok(None)
        }
    }
    fn begin_clipboard_copy(
        &mut self,
        document: u32,
        pages: &[usize],
        token: Option<u32>,
        cut: bool,
    ) -> Reply<(u32, Vec<u8>)> {
        self.validate_clipboard_document(document, token)?;
        let count = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .info()
            .page_count;
        let unique: std::collections::BTreeSet<_> = pages.iter().copied().collect();
        if pages.is_empty()
            || pages.len() > 4096
            || unique.len() != pages.len()
            || unique.last().is_none_or(|p| *p >= count)
            || (cut && pages.len() == count)
        {
            return Err("请选择有效且不重复的页面；剪切不能移除所有页。".into());
        }
        let (id, _, bytes) = self.begin_export()?;
        Ok((id, bytes))
    }
    fn validate_clipboard_document(&self, document: u32, token: Option<u32>) -> Reply<()> {
        self.ensure_editor_idle()?;
        if document != self.active_document_id || self.session.is_none() {
            return Err("活动文档已改变，请重新选择页面。".into());
        }
        if let Some(token) = token {
            self.validate_overview(token)?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub(super) struct CopyReply {
    pages: usize,
    document: Option<DocumentView>,
}

async fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    operation: impl FnOnce(&tauri::AppHandle) -> Reply<T> + Send + 'static,
) -> Reply<T> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = send.send(operation(&handle));
    })
    .map_err(|_| "无法调度系统剪贴板。")?;
    receive
        .await
        .map_err(|_| "系统剪贴板操作未返回。".to_string())?
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn copy_pages_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
    document: u32,
    pages: Vec<usize>,
    token: Option<u32>,
    cut: bool,
) -> Reply<CopyReply> {
    let (id, bytes) = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_clipboard_copy(document, &pages, token, cut)?;
    let reservation = ExportReservation {
        store: state.inner(),
        id,
    };
    let selected = pages.clone();
    let payload = tauri::async_runtime::spawn_blocking(move || {
        Session::open(&bytes)
            .and_then(|s| s.page_clipboard(&selected))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "页面复制任务未完成")??;
    // The reservation excludes edits, tab switches and normal close/quit while
    // rendering and platform I/O run without holding the workspace mutex.
    let published = on_main(&app, move |app| platform::write(app, &payload)).await;
    let mut w = state.lock().map_err(|_| "文档状态不可用")?;
    let result = w.finish_page_copy(id, pages.clone(), cut, published)?;
    drop(w);
    drop(reservation);
    Ok(CopyReply {
        pages: pages.len(),
        document: result,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn paste_pages_document(
    app: tauri::AppHandle,
    state: State<'_, Store>,
    document: u32,
    index: usize,
    token: Option<u32>,
) -> Reply<Option<DocumentView>> {
    let id = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        w.validate_clipboard_document(document, token)?;
        w.begin_insertion(index)?
    };
    let reservation = ExportReservation {
        store: state.inner(),
        id,
    };
    // Only the current typed system clipboard is authoritative; never an old
    // application-private PDF or a browser-provided byte/path payload.
    let bytes = on_main(&app, platform::read).await?;
    let view = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .finish_insertion(id, index, &bytes)?;
    drop(reservation);
    Ok(Some(view))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn workspace() -> Workspace {
        let mut w = Workspace::default();
        w.replace_document(
            Session::blank(100., 80., 3).unwrap(),
            "synthetic.pdf".into(),
        )
        .unwrap();
        w
    }
    #[test]
    fn page_copy_reserves_the_exact_active_document_without_editing_or_clearing_dirty() {
        let mut w = workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let id = w.active_document_id;
        let (reservation, bytes) = w.begin_clipboard_copy(id, &[2, 0], None, false).unwrap();
        assert_eq!(bytes, before);
        assert!(w.ensure_editor_idle().is_err());
        assert!(w.has_unsaved_work());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        w.release_export(reservation);
        assert!(w.ensure_editor_idle().is_ok());
    }
    #[test]
    fn bad_page_copy_never_reserves_or_accepts_a_foreign_tab_or_cut_all_pages() {
        let mut w = workspace();
        let id = w.active_document_id;
        for (document, pages, cut) in [
            (id + 1, vec![0], false),
            (id, vec![], false),
            (id, vec![0, 0], false),
            (id, vec![3], false),
            (id, vec![0, 1, 2], true),
        ] {
            assert!(w.begin_clipboard_copy(document, &pages, None, cut).is_err());
            assert!(w.export_reservation.is_none());
        }
    }
    #[test]
    fn accepted_cut_is_one_undo_step_and_wrong_or_cancelled_reservation_cannot_delete() {
        let mut w = workspace();
        let original = w.session.as_ref().unwrap().bytes().unwrap();
        let document = w.active_document_id;
        let (id, _) = w.begin_clipboard_copy(document, &[1], None, true).unwrap();
        assert!(
            w.finish_page_copy(id, vec![1], true, Err("synthetic OS write failure".into()))
                .is_err()
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
        assert!(w.finish_page_copy(id + 1, vec![1], true, Ok(())).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
        let view = w
            .finish_page_copy(id, vec![1], true, Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!(view.info.page_count, 2);
        assert!(w.export_reservation.is_none());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
        let (cancelled, _) = w.begin_clipboard_copy(document, &[0], None, true).unwrap();
        w.release_export(cancelled);
        assert!(
            w.finish_page_copy(cancelled, vec![0], true, Ok(()))
                .is_err()
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), original);
    }
}
