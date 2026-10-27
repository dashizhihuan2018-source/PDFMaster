use super::*;

pub(super) struct Parked {
    pub id: u32,
    pub session: Session,
    pub name: String,
    pub restored: bool,
}
#[derive(Clone, Serialize)]
pub(super) struct TabView {
    id: u32,
    name: String,
    dirty: bool,
}
pub(super) struct CloseSnapshot {
    id: u32,
    epoch: u64,
    name: String,
    bytes: Vec<u8>,
    dirty: bool,
}
#[derive(Serialize)]
pub(super) struct CloseReply {
    closed: bool,
    document: Option<DocumentView>,
}
impl Workspace {
    pub(super) fn prepare_close(&self, id: u32) -> Reply<CloseSnapshot> {
        self.ensure_editor_idle()?;
        if self.recovery_fault {
            return Err("草稿存储异常，请先导出副本，不可关闭文档。".into());
        }
        let (session, name, restored) = if let Some(session) = self
            .session
            .as_ref()
            .filter(|_| self.active_document_id == id)
        {
            (session, &self.name, self.restored)
        } else {
            let document = self
                .inactive_documents
                .iter()
                .find(|d| d.id == id)
                .ok_or("文档标签已关闭。")?;
            (&document.session, &document.name, document.restored)
        };
        Ok(CloseSnapshot {
            id,
            epoch: self.document_epoch,
            name: name.clone(),
            bytes: session.bytes().map_err(|e| e.to_string())?,
            dirty: restored || session.is_dirty(),
        })
    }
    fn validate_close(&self, snapshot: &CloseSnapshot) -> Reply<()> {
        let current = self.prepare_close(snapshot.id)?;
        if current.epoch != snapshot.epoch
            || current.bytes != snapshot.bytes
            || current.dirty != snapshot.dirty
        {
            return Err("文档会话已改变，未关闭标签，请重新确认。".into());
        }
        Ok(())
    }
    pub(super) fn close_confirmed(&mut self, snapshot: &CloseSnapshot) -> Reply<CloseReply> {
        self.validate_close(snapshot)?;
        let epoch = self
            .document_epoch
            .checked_add(1)
            .ok_or("文档会话编号已耗尽。")?;
        // Commit the remaining drafts before releasing any session. Failure
        // leaves all tabs and bytes in memory and preserves exit protection.
        self.persist_excluding(Some(snapshot.id))?;
        if snapshot.id == self.active_document_id {
            self.session = None;
            self.name.clear();
            self.restored = false;
            self.inactive_documents.sort_by_key(|d| d.id);
            if !self.inactive_documents.is_empty() {
                let index = self
                    .inactive_documents
                    .iter()
                    .position(|d| d.id > snapshot.id)
                    .unwrap_or(self.inactive_documents.len() - 1);
                let next = self.inactive_documents.remove(index);
                self.active_document_id = next.id;
                self.session = Some(next.session);
                self.name = next.name;
                self.restored = next.restored;
            }
        } else {
            self.inactive_documents.retain(|d| d.id != snapshot.id);
        }
        self.document_epoch = epoch;
        Ok(CloseReply {
            closed: true,
            document: self.session.as_ref().map(|_| self.view()).transpose()?,
        })
    }
    pub(super) fn activate_document(&mut self, id: u32) -> Reply<DocumentView> {
        self.ensure_editor_idle()?;
        if self.session.is_some() && self.active_document_id == id {
            return self.view();
        }
        let index = self
            .inactive_documents
            .iter()
            .position(|d| d.id == id)
            .ok_or("文档标签已关闭。")?;
        let epoch = self
            .document_epoch
            .checked_add(1)
            .ok_or("文档会话编号已耗尽。")?;
        let next = self.inactive_documents.remove(index);
        if let Some(session) = self.session.take() {
            self.inactive_documents.push(Parked {
                id: self.active_document_id,
                session,
                name: std::mem::take(&mut self.name),
                restored: self.restored,
            });
        }
        self.session = Some(next.session);
        self.name = next.name;
        self.restored = next.restored;
        self.active_document_id = next.id;
        self.document_epoch = epoch;
        self.view()
    }
    pub(super) fn recover_documents(&mut self, drafts: Vec<recovery::Draft>) -> Reply<()> {
        if self.session.is_some() || !self.inactive_documents.is_empty() || drafts.len() > 16 {
            return Err("恢复需要空工作区。".into());
        }
        // Parse every candidate before installing any. One invalid PDF must not
        // allow later checkpoints to erase an unreadable sibling draft.
        let mut documents = Vec::new();
        for (index, draft) in drafts.into_iter().enumerate() {
            documents.push(Parked {
                id: index as u32 + 1,
                session: Session::open(&draft.bytes).map_err(|e| e.to_string())?,
                name: draft.name,
                restored: true,
            });
        }
        if let Some(active) = documents.pop() {
            self.next_document_id = active.id;
            self.active_document_id = active.id;
            self.session = Some(active.session);
            self.name = active.name;
            self.restored = true;
            self.document_epoch = 1;
        }
        self.inactive_documents = documents;
        Ok(())
    }
    pub(super) fn tabs(&self) -> Vec<TabView> {
        let mut tabs: Vec<_> = self
            .inactive_documents
            .iter()
            .map(|d| TabView {
                id: d.id,
                name: d.name.clone(),
                dirty: d.restored || d.session.is_dirty() || self.recovery_fault,
            })
            .collect();
        if self.session.is_some() {
            tabs.push(TabView {
                id: self.active_document_id,
                name: self.name.clone(),
                dirty: self.dirty(),
            });
        }
        tabs.sort_by_key(|d| d.id);
        tabs
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn activate_document(
    id: u32,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .activate_document(id)
        .map(Some)
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn close_document(
    id: u32,
    app: tauri::AppHandle,
    state: State<'_, Store>,
) -> Reply<CloseReply> {
    use tauri_plugin_dialog::{MessageDialogButtons, MessageDialogResult};
    let snapshot = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .prepare_close(id)?;
    if snapshot.dirty {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let dialog = app
            .dialog()
            .message(format!(
                "是否保存“{}”的修改？\n保存会导出新的 PDF 副本，原文件不会覆盖。",
                snapshot.name
            ))
            .title("关闭文档")
            .buttons(MessageDialogButtons::YesNoCancelCustom(
                "保存副本并关闭".into(),
                "不保存".into(),
                "取消".into(),
            ));
        #[cfg(desktop)]
        let dialog = dialog.parent(&app.get_webview_window("main").ok_or("确认窗口不存在。")?);
        dialog.show_with_result(move |result| {
            let _ = sender.send(result);
        });
        let result = receiver
            .await
            .map_err(|_| "系统确认窗口不可用；文档未关闭。")?;
        let save = matches!(&result, MessageDialogResult::Yes)
            || matches!(&result,MessageDialogResult::Custom(label) if label=="保存副本并关闭");
        let discard = matches!(&result, MessageDialogResult::No)
            || matches!(&result,MessageDialogResult::Custom(label) if label=="不保存");
        if !save && !discard {
            let workspace = state.lock().map_err(|_| "文档状态不可用")?;
            return Ok(CloseReply {
                closed: false,
                document: workspace
                    .session
                    .as_ref()
                    .map(|_| workspace.view())
                    .transpose()?,
            });
        }
        if save {
            let reservation_id = {
                let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
                workspace.validate_close(&snapshot)?;
                let reservation_id = workspace
                    .next_export_id
                    .checked_add(1)
                    .ok_or("导出会话编号已耗尽。")?;
                workspace.next_export_id = reservation_id;
                workspace.export_reservation = Some(reservation_id);
                reservation_id
            };
            let reservation = ExportReservation {
                store: state.inner(),
                id: reservation_id,
            };
            let copy_name = format!(
                "{}-副本.pdf",
                Path::new(&snapshot.name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
            let published = publish_pdf_copy(&app, &copy_name, &snapshot.bytes).await?;
            drop(reservation);
            if !published {
                let workspace = state.lock().map_err(|_| "文档状态不可用")?;
                return Ok(CloseReply {
                    closed: false,
                    document: workspace
                        .session
                        .as_ref()
                        .map(|_| workspace.view())
                        .transpose()?,
                });
            }
        }
    }
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .close_confirmed(&snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace {
            journal: Some(recovery::Journal::new(dir.path().join("draft.json"))),
            ..Default::default()
        };
        (dir, workspace)
    }
    #[test]
    fn switching_preserves_independent_bytes_history_and_dirty_state() {
        let (_dir, mut w) = workspace();
        w.replace_document(Session::blank(100., 100., 2).unwrap(), "a.pdf".into())
            .unwrap();
        let first = w.active_document_id;
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let bytes = w.session.as_ref().unwrap().bytes().unwrap();
        w.replace_document(Session::blank(200., 100., 1).unwrap(), "b.pdf".into())
            .unwrap();
        let second = w.active_document_id;
        let epoch = w.document_epoch;
        w.activate_document(first).unwrap();
        assert!(w.document_epoch > epoch);
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), bytes);
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().info().pages[0].rotation, 0);
        w.activate_document(second).unwrap();
        assert_eq!(w.session.as_ref().unwrap().info().page_count, 1);
        assert!(!w.session.as_ref().unwrap().info().can_undo);
        w.activate_document(first).unwrap();
        assert!(w.session.as_ref().unwrap().info().can_redo);
    }
    #[test]
    fn all_dirty_tabs_recover_or_none_are_installed() {
        let (_dir, mut w) = workspace();
        let a = Session::blank(100., 100., 1).unwrap().bytes().unwrap();
        let b = Session::blank(200., 100., 2).unwrap().bytes().unwrap();
        w.recover_documents(vec![
            recovery::Draft {
                name: "a.pdf".into(),
                bytes: a.clone(),
            },
            recovery::Draft {
                name: "b.pdf".into(),
                bytes: b.clone(),
            },
        ])
        .unwrap();
        assert_eq!(w.tabs().len(), 2);
        assert!(w.has_unsaved_work());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), b);
        w.activate_document(1).unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), a);
        let (_dir, mut invalid) = workspace();
        assert!(
            invalid
                .recover_documents(vec![
                    recovery::Draft {
                        name: "a.pdf".into(),
                        bytes: a
                    },
                    recovery::Draft {
                        name: "bad.pdf".into(),
                        bytes: b"not PDF".to_vec()
                    }
                ])
                .is_err()
        );
        assert!(invalid.session.is_none());
        assert!(invalid.inactive_documents.is_empty());
    }
    #[test]
    fn close_is_snapshot_bound_and_preserves_other_dirty_tabs() {
        let (_dir, mut w) = workspace();
        w.replace_document(Session::blank(100., 100., 1).unwrap(), "a.pdf".into())
            .unwrap();
        let first = w.active_document_id;
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        w.replace_document(Session::blank(200., 100., 1).unwrap(), "b.pdf".into())
            .unwrap();
        let second = w.active_document_id;
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 180,
            },
        )
        .unwrap();
        let stale = w.prepare_close(first).unwrap();
        w.activate_document(first).unwrap();
        assert!(w.close_confirmed(&stale).is_err());
        let snapshot = w.prepare_close(first).unwrap();
        assert!(w.close_confirmed(&snapshot).unwrap().closed);
        assert_eq!(w.active_document_id, second);
        assert_eq!(w.tabs().len(), 1);
        let recovered = w.journal.as_ref().unwrap().load_many().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].name, "b.pdf");
        assert!(w.has_unsaved_work());
        let snapshot = w.prepare_close(second).unwrap();
        assert!(w.close_confirmed(&snapshot).unwrap().document.is_none());
        assert!(!w.has_unsaved_work());
        assert!(w.journal.as_ref().unwrap().load_many().unwrap().is_empty());
    }
    #[test]
    fn close_refuses_mutated_bytes_and_failed_journal_without_dropping_tabs() {
        let (dir, mut w) = workspace();
        w.replace_document(Session::blank(100., 100., 1).unwrap(), "a.pdf".into())
            .unwrap();
        let snapshot = w.prepare_close(w.active_document_id).unwrap();
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        assert!(w.close_confirmed(&snapshot).is_err());
        let snapshot = w.prepare_close(w.active_document_id).unwrap();
        std::fs::remove_file(dir.path().join("draft.json")).unwrap();
        std::fs::create_dir(dir.path().join("draft.json")).unwrap();
        assert!(w.close_confirmed(&snapshot).is_err());
        assert!(w.session.is_some());
        assert_eq!(w.tabs().len(), 1);
        assert!(w.has_unsaved_work());
    }
    #[test]
    fn switch_and_close_obey_export_and_epoch_guards() {
        let (_dir, mut w) = workspace();
        w.replace_document(Session::blank(100., 100., 1).unwrap(), "a.pdf".into())
            .unwrap();
        let first = w.active_document_id;
        w.replace_document(Session::blank(200., 100., 1).unwrap(), "b.pdf".into())
            .unwrap();
        let second = w.active_document_id;
        let snapshot = w.prepare_close(first).unwrap();
        w.export_reservation = Some(99);
        assert!(w.activate_document(first).is_err());
        assert!(w.prepare_close(first).is_err());
        assert!(w.close_confirmed(&snapshot).is_err());
        w.export_reservation = None;
        w.document_epoch = u64::MAX;
        assert!(w.activate_document(first).is_err());
        assert_eq!(w.active_document_id, second);
        assert_eq!(w.tabs().len(), 2);
        assert!(w.activate_document(u32::MAX).is_err());
        assert_eq!(w.active_document_id, second);
    }
    #[test]
    fn recovering_from_an_actual_multi_dirty_checkpoint_preserves_both_bytes() {
        let (_dir, mut w) = workspace();
        w.replace_document(Session::blank(100., 100., 1).unwrap(), "a.pdf".into())
            .unwrap();
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let first = w.session.as_ref().unwrap().bytes().unwrap();
        w.replace_document(Session::blank(200., 100., 1).unwrap(), "b.pdf".into())
            .unwrap();
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 180,
            },
        )
        .unwrap();
        let second = w.session.as_ref().unwrap().bytes().unwrap();
        let mut recovered = Workspace::default();
        recovered
            .recover_documents(w.journal.as_ref().unwrap().load_many().unwrap())
            .unwrap();
        assert_eq!(recovered.tabs().len(), 2);
        assert_eq!(recovered.session.as_ref().unwrap().bytes().unwrap(), second);
        recovered.activate_document(1).unwrap();
        assert_eq!(recovered.session.as_ref().unwrap().bytes().unwrap(), first);
        assert!(recovered.tabs().iter().all(|d| d.dirty));
    }

    #[test]
    fn opening_a_second_pdf_keeps_the_dirty_first_session_and_journals_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = Workspace {
            journal: Some(recovery::Journal::new(dir.path().join("draft.json"))),
            ..Default::default()
        };
        workspace
            .replace_document(Session::blank(100., 100., 1).unwrap(), "first.pdf".into())
            .unwrap();
        apply_single_edit(
            &mut workspace,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let first = workspace.session.as_ref().unwrap().bytes().unwrap();
        let reply = workspace
            .replace_document(Session::blank(200., 100., 2).unwrap(), "second.pdf".into())
            .unwrap();
        let value = serde_json::to_value(reply).unwrap();
        assert_eq!(value["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(value["tabs"][0]["name"], "first.pdf");
        assert_eq!(value["tabs"][0]["dirty"], true);
        assert!(workspace.has_unsaved_work());
        // A later clean active export must not remove the inactive dirty draft.
        let second = workspace.session.as_ref().unwrap().bytes().unwrap();
        workspace.finish_verified_export(&second).unwrap();
        assert!(workspace.has_unsaved_work());
        let recovered = workspace.journal.as_ref().unwrap().load().unwrap().unwrap();
        assert_eq!(recovered.name, "first.pdf");
        assert_eq!(recovered.bytes, first);
    }
}
