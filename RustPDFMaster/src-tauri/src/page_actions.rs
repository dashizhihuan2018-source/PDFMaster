use super::*;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum PageAction {
    Extract,
    RotateRange,
    DeleteRange,
    Move,
}
#[derive(Default, Serialize)]
pub(super) struct PageActionReply {
    document: Option<DocumentView>,
    target_page: Option<usize>,
}
struct PromptSnapshot {
    token: u32,
    document: u32,
    epoch: u64,
    count: usize,
    bytes: Vec<u8>,
}
fn reserve(w: &mut Workspace) -> Reply<PromptSnapshot> {
    let (token, _, bytes) = w.begin_export()?;
    Ok(PromptSnapshot {
        token,
        document: w.active_document_id,
        epoch: w.document_epoch,
        count: w.session.as_ref().ok_or("请先打开 PDF")?.info().page_count,
        bytes,
    })
}
fn validate(w: &Workspace, s: &PromptSnapshot) -> Reply<()> {
    if w.export_reservation != Some(s.token)
        || w.active_document_id != s.document
        || w.document_epoch != s.epoch
        || w.session.as_ref().ok_or("请先打开 PDF")?.info().page_count != s.count
        || w.session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|e| e.to_string())?
            != s.bytes
    {
        return Err("输入期间文档已改变，请重新操作；文档未修改。".into());
    }
    Ok(())
}
fn parsed_pages(s: &PromptSnapshot, input: &str) -> Reply<Vec<usize>> {
    if input.len() > 2048 {
        return Err("页码输入过长。".into());
    }
    PageRange::parse(input, s.count).map_err(|e| format!("页码范围无效：{e}"))
}
fn move_target(s: &PromptSnapshot, input: &str) -> Reply<usize> {
    let value = input.trim();
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("请输入单个目标页码。".into());
    }
    let n = value.parse::<usize>().map_err(|_| "目标页码无效。")?;
    if n == 0 || n > s.count {
        return Err("目标页码超出文档范围。".into());
    }
    Ok(n - 1)
}
fn apply_input(
    w: &mut Workspace,
    s: &PromptSnapshot,
    action: PageAction,
    page: usize,
    input: &str,
    confirmed: Option<usize>,
) -> Reply<PageActionReply> {
    validate(w, s)?;
    // Parse completely before releasing ownership. The subsequent release and
    // existing core operation are under the caller's SAME Store lock.
    let target = if action == PageAction::Move {
        if page >= s.count {
            return Err("源页码无效。".into());
        }
        Some(move_target(s, input)?)
    } else {
        None
    };
    if action != PageAction::Move {
        let pages = parsed_pages(s, input)?;
        if action == PageAction::DeleteRange
            && (pages.len() >= s.count || confirmed != Some(pages.len()))
        {
            return Err("删除须确认选中页数，且不能删除全部页面。".into());
        }
    }
    if action == PageAction::Extract {
        return Err("提取须使用原生保存流程。".into());
    }
    w.release_export(s.token);
    let document = match action {
        PageAction::Move => apply_single_edit(
            w,
            Edit::MovePage {
                from: page,
                to: target.unwrap(),
            },
        )?,
        PageAction::RotateRange | PageAction::DeleteRange => {
            let reply = if action == PageAction::DeleteRange {
                apply_confirmed_delete(w, input, confirmed.unwrap(), &s.bytes, s.epoch)?
            } else {
                apply_page_batch(w, input, PageBatchOperation::Rotate90, None)?
            };
            match reply {
                PageBatchReply::Applied { document, .. } => *document,
                _ => return Err("页面操作未完成。".into()),
            }
        }
        PageAction::Extract => unreachable!(),
    };
    Ok(PageActionReply {
        document: Some(document),
        target_page: target,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn prompt_page_action(
    app: tauri::AppHandle,
    state: State<'_, Store>,
    operation: PageAction,
    page: usize,
) -> Reply<PageActionReply> {
    let snapshot = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        reserve(&mut w)?
    };
    let reservation = ExportReservation {
        store: state.inner(),
        id: snapshot.token,
    };
    let (title, label) = match operation {
        PageAction::Extract => ("提取页面", "页码范围，例如 1-3,5".to_string()),
        PageAction::RotateRange => ("旋转页面", "页码范围，例如 1-3,5".to_string()),
        PageAction::DeleteRange => ("删除页面", "页码范围，例如 1-3,5".to_string()),
        PageAction::Move => ("移动当前页", format!("目标页码 1-{}", snapshot.count)),
    };
    let Some(input) = page_input_windows::prompt(&app, title, &label).await? else {
        return Ok(PageActionReply::default());
    };
    if operation == PageAction::Extract {
        let (name, bytes) = {
            let w = state.lock().map_err(|_| "文档状态不可用")?;
            validate(&w, &snapshot)?;
            let pages = parsed_pages(&snapshot, &input)?;
            let bytes = w
                .session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .extract_pages(&pages)
                .map_err(|e| e.to_string())?;
            (
                format!(
                    "{}-提取页面.pdf",
                    Path::new(&w.name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                ),
                bytes,
            )
        };
        if !publish_pdf_copy(&app, &name, &bytes).await? {
            return Ok(PageActionReply::default());
        }
        let document = {
            let mut w = state.lock().map_err(|_| "文档状态不可用")?;
            validate(&w, &snapshot)?;
            w.finish_extraction(snapshot.token)?
        };
        drop(reservation);
        return Ok(PageActionReply {
            document: Some(document),
            target_page: None,
        });
    }
    let confirmed = if operation == PageAction::DeleteRange {
        let count = {
            let w = state.lock().map_err(|_| "文档状态不可用")?;
            validate(&w, &snapshot)?;
            let count = parsed_pages(&snapshot, &input)?.len();
            if count >= snapshot.count {
                return Err("不能删除文档的全部页面。".into());
            }
            count
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .message(format!(
                "确认删除选中的 {count} 页？文档共有 {} 页；此操作可通过撤销恢复。",
                snapshot.count
            ))
            .title("批量删除页面")
            .parent(&app.get_webview_window("main").ok_or("确认窗口不存在")?)
            .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
                "删除选中页面".into(),
                "取消".into(),
            ))
            .show(move |yes| {
                let _ = tx.send(yes);
            });
        if !rx.await.map_err(|_| "系统确认窗口不可用")? {
            return Ok(PageActionReply::default());
        }
        Some(count)
    } else {
        None
    };
    let reply = {
        let mut w = state.lock().map_err(|_| "文档状态不可用")?;
        apply_input(&mut w, &snapshot, operation, page, &input, confirmed)?
    };
    drop(reservation);
    Ok(reply)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_frontend_operations_match_native_parser_and_reject_unknown() {
        for (wire, operation) in [
            ("extract", PageAction::Extract),
            ("rotate_range", PageAction::RotateRange),
            ("delete_range", PageAction::DeleteRange),
            ("move", PageAction::Move),
        ] {
            assert_eq!(
                serde_json::from_value::<PageAction>(serde_json::json!(wire)).unwrap(),
                operation
            );
        }
        assert!(serde_json::from_value::<PageAction>(serde_json::json!("generic_panel")).is_err());
    }
    #[test]
    fn native_count_confirmation_is_required_and_delete_is_one_transaction() {
        let mut w = fixture();
        let s = reserve(&mut w).unwrap();
        assert!(apply_input(&mut w, &s, PageAction::DeleteRange, 0, "1,3", None).is_err());
        assert!(apply_input(&mut w, &s, PageAction::DeleteRange, 0, "1,3", Some(1)).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), s.bytes);
        assert!(!w.session.as_ref().unwrap().info().can_undo);
        apply_input(&mut w, &s, PageAction::DeleteRange, 0, "1,3", Some(2)).unwrap();
        let session = w.session.as_mut().unwrap();
        let after = session.bytes().unwrap();
        assert_eq!(session.info().page_count, 1);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), s.bytes);
        assert!(!session.info().can_undo);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
    }
    fn fixture() -> Workspace {
        Workspace {
            session: Some(Session::blank(100., 100., 3).unwrap()),
            ..Default::default()
        }
    }
    #[test]
    fn prompt_rotate_is_one_transaction_and_undo_redo_exact() {
        let mut w = fixture();
        let s = reserve(&mut w).unwrap();
        let before = s.bytes.clone();
        apply_input(&mut w, &s, PageAction::RotateRange, 0, "1,3", None).unwrap();
        let session = w.session.as_mut().unwrap();
        let after = session.bytes().unwrap();
        assert_ne!(before, after);
        assert_eq!(session.info().pages[1].rotation, 0);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), before);
        assert!(!session.info().can_undo);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
        assert_eq!(s.bytes, before);
    }
    #[test]
    fn prompt_move_is_one_transaction_and_returns_actual_destination() {
        let mut w = fixture();
        w.session
            .as_mut()
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let s = reserve(&mut w).unwrap();
        let before = s.bytes.clone();
        let reply = apply_input(&mut w, &s, PageAction::Move, 0, "3", None).unwrap();
        assert_eq!(reply.target_page, Some(2));
        let session = w.session.as_mut().unwrap();
        let after = session.bytes().unwrap();
        assert_eq!(session.info().pages[2].rotation, 90);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), before);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
    }
    #[test]
    fn invalid_or_stale_prompts_never_mutate_history() {
        for (action, input) in [
            (PageAction::Move, "0"),
            (PageAction::Move, "1,2"),
            (PageAction::Move, "4"),
            (PageAction::RotateRange, "1,x"),
            (PageAction::DeleteRange, "1-3"),
        ] {
            let mut w = fixture();
            let s = reserve(&mut w).unwrap();
            assert!(apply_input(&mut w, &s, action, 0, input, None).is_err());
            assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), s.bytes);
            assert!(!w.session.as_ref().unwrap().info().can_undo);
        }
        let mut w = fixture();
        let s = reserve(&mut w).unwrap();
        w.document_epoch += 1;
        assert!(apply_input(&mut w, &s, PageAction::RotateRange, 0, "1", None).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), s.bytes);
        assert!(!w.session.as_ref().unwrap().info().can_undo);
    }
    #[test]
    fn cancellation_releases_only_owned_reservation_without_document_replacement() {
        let store = Store::new(fixture());
        let s = reserve(&mut store.lock().unwrap()).unwrap();
        {
            let _guard = ExportReservation {
                store: &store,
                id: s.token,
            };
        }
        let w = store.lock().unwrap();
        assert!(w.export_reservation.is_none());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), s.bytes);
        assert!(!w.session.as_ref().unwrap().info().can_undo);
        let reply = PageActionReply::default();
        assert!(reply.document.is_none());
        assert!(reply.target_page.is_none());
    }
}
