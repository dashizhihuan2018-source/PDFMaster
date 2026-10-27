use super::*;

pub(super) struct Snapshot {
    token: u32,
    epoch: u64,
    bytes: Vec<u8>,
}

impl Workspace {
    fn prepare_overview(&mut self) -> Reply<u32> {
        self.ensure_editor_idle()?;
        let token = self
            .next_overview_id
            .checked_add(1)
            .ok_or("页面总览会话编号已耗尽。")?;
        let bytes = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|e| e.to_string())?;
        self.overview_snapshot = Some(Snapshot {
            token,
            epoch: self.document_epoch,
            bytes,
        });
        self.next_overview_id = token;
        Ok(token)
    }
    fn validate_overview(&self, token: u32) -> Reply<()> {
        self.ensure_editor_idle()?;
        let snapshot = self
            .overview_snapshot
            .as_ref()
            .filter(|s| s.token == token)
            .ok_or("页面总览已更新，请重新选择页面；文档未修改。")?;
        if snapshot.epoch != self.document_epoch
            || self
                .session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .bytes()
                .map_err(|e| e.to_string())?
                != snapshot.bytes
        {
            return Err("页面总览已更新，请重新选择页面；文档未修改。".into());
        }
        Ok(())
    }
    fn move_overview(
        &mut self,
        token: u32,
        pages: Vec<usize>,
        insertion: usize,
    ) -> Reply<DocumentView> {
        self.validate_overview(token)?;
        self.session
            .as_mut()
            .ok_or("请先打开 PDF")?
            .apply(Edit::MovePages { pages, insertion })
            .map_err(|e| e.to_string())?;
        self.checkpoint()
    }
}

#[tauri::command]
pub(super) async fn prepare_page_overview(state: State<'_, Store>) -> Reply<u32> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .prepare_overview()
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn move_overview_pages(
    token: u32,
    pages: Vec<usize>,
    insertion: usize,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .move_overview(token, pages, insertion)
        .map(Some)
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn render_overview_page(
    token: u32,
    page: usize,
    scale: f32,
    state: State<'_, Store>,
) -> Reply<String> {
    let w = state.lock().map_err(|_| "文档状态不可用")?;
    w.validate_overview(token)?;
    let png = w
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .render_png(page, scale)
        .map_err(|e| e.to_string())?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let directory = tempfile::tempdir().unwrap();
        let mut workspace = Workspace {
            journal: Some(recovery::Journal::new(directory.path().join("draft.json"))),
            ..Default::default()
        };
        let mut session = Session::blank(200., 120., 5).unwrap();
        session
            .apply(Edit::Rotate {
                page: 1,
                degrees: 90,
            })
            .unwrap();
        session
            .apply(Edit::Rotate {
                page: 3,
                degrees: 180,
            })
            .unwrap();
        session.mark_saved();
        workspace
            .replace_document(session, "synthetic.pdf".into())
            .unwrap();
        (directory, workspace)
    }

    #[test]
    fn generic_edit_cannot_bypass_the_snapshot_bound_move_entry() {
        let (_directory, mut w) = workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(
            apply_single_edit(
                &mut w,
                Edit::MovePages {
                    pages: vec![3, 1],
                    insertion: 0
                }
            )
            .is_err()
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    }

    #[test]
    fn overview_preparation_is_read_only_and_only_the_latest_token_is_valid() {
        let (_directory, mut w) = workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        let first = w.prepare_overview().unwrap();
        assert!(w.validate_overview(first).is_ok());
        let second = w.prepare_overview().unwrap();
        assert_ne!(first, second);
        assert!(w.validate_overview(first).is_err());
        assert!(w.validate_overview(second).is_ok());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
    }

    #[test]
    fn overview_move_is_one_checkpointed_transaction_and_token_is_consumed() {
        let (directory, mut w) = workspace();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let token = w.prepare_overview().unwrap();
        let view = w.move_overview(token, vec![3, 1], 0).unwrap();
        assert!(view.info.dirty);
        assert_eq!(
            view.info
                .pages
                .iter()
                .map(|p| p.rotation)
                .collect::<Vec<_>>(),
            [90, 180, 0, 0, 0]
        );
        let after = w.session.as_ref().unwrap().bytes().unwrap();
        let recovered = recovery::Journal::new(directory.path().join("draft.json"))
            .load_many()
            .unwrap();
        assert_eq!(recovered[0].bytes, after);
        assert!(w.move_overview(token, vec![0], 5).is_err());
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        w.session.as_mut().unwrap().redo().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), after);
    }

    #[test]
    fn invalid_and_noop_moves_do_not_consume_snapshot_or_history() {
        let (_directory, mut w) = workspace();
        let token = w.prepare_overview().unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let info = w.session.as_ref().unwrap().info();
        for (pages, insertion) in [
            (vec![], 0),
            (vec![1, 1], 0),
            (vec![1], 6),
            (vec![5], 0),
            (vec![1, 2], 2),
        ] {
            assert!(w.move_overview(token, pages, insertion).is_err());
            assert!(w.validate_overview(token).is_ok());
            assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
            assert_eq!(w.session.as_ref().unwrap().info(), info);
        }
    }

    #[test]
    fn old_snapshot_cannot_move_a_changed_document_or_a_different_tab() {
        let (_directory, mut w) = workspace();
        let token = w.prepare_overview().unwrap();
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        let after = w.session.as_ref().unwrap().bytes().unwrap();
        assert!(w.move_overview(token, vec![3, 1], 0).is_err());
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), after);
        let token = w.prepare_overview().unwrap();
        let first = w.active_document_id;
        w.replace_document(Session::open(&after).unwrap(), "same-bytes.pdf".into())
            .unwrap();
        assert!(w.validate_overview(token).is_err());
        w.activate_document(first).unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), after);
        assert!(w.validate_overview(token).is_err());
    }

    #[test]
    fn export_and_editor_guards_block_overview_and_exhaustion_never_wraps() {
        let (_directory, mut w) = workspace();
        let token = w.prepare_overview().unwrap();
        w.begin_export().unwrap();
        assert!(w.prepare_overview().is_err());
        assert!(w.move_overview(token, vec![1], 0).is_err());
        w.export_reservation = None;
        let editor = w
            .begin_ink(
                "overview-guard",
                0,
                pdfmaster_core::Color {
                    r: 1.,
                    g: 0.,
                    b: 0.,
                },
                2.,
            )
            .unwrap();
        assert!(w.prepare_overview().is_err());
        assert!(w.move_overview(token, vec![1], 0).is_err());
        w.cancel_ink(editor.id).unwrap();
        w.next_overview_id = u32::MAX;
        assert!(w.prepare_overview().is_err());
        assert_eq!(w.next_overview_id, u32::MAX);
    }

    #[test]
    fn checkpoint_invalidates_a_snapshot_even_when_undo_restores_identical_bytes() {
        let (_directory, mut w) = workspace();
        let token = w.prepare_overview().unwrap();
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        apply_single_edit(
            &mut w,
            Edit::Rotate {
                page: 0,
                degrees: 90,
            },
        )
        .unwrap();
        w.session.as_mut().unwrap().undo().unwrap();
        w.checkpoint().unwrap();
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(w.validate_overview(token).is_err());
    }

    #[test]
    fn journal_failure_retains_moved_bytes_and_exposes_a_sticky_warning() {
        let (directory, mut w) = workspace();
        let token = w.prepare_overview().unwrap();
        let bad_parent = directory.path().join("not-a-directory");
        std::fs::write(&bad_parent, b"synthetic storage failure").unwrap();
        w.journal = Some(recovery::Journal::new(bad_parent.join("draft.json")));
        let view = w.move_overview(token, vec![1, 3], 0).unwrap();
        assert!(view.warning.is_some());
        assert!(view.info.dirty);
        assert!(w.recovery_fault);
        assert!(w.has_unsaved_work());
        assert!(w.validate_overview(token).is_err());
        assert_eq!(
            view.info
                .pages
                .iter()
                .map(|p| p.rotation)
                .collect::<Vec<_>>(),
            [90, 180, 0, 0, 0]
        );
        w.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(
            w.session
                .as_ref()
                .unwrap()
                .info()
                .pages
                .iter()
                .map(|p| p.rotation)
                .collect::<Vec<_>>(),
            [0, 90, 0, 180, 0]
        );
    }
}
