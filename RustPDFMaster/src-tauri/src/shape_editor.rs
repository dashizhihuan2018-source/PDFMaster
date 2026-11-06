use super::*;
use pdfmaster_core::{CanvasShapeDraft, CanvasShapeSelection};
use std::sync::Arc;

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct CanvasShapeProjection {
    #[serde(flatten)]
    list: pdfmaster_core::CanvasShapeList,
    hit_rows: Vec<pdfmaster_core::AnnotationRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CanvasShapeTarget {
    Existing {
        original: CanvasShapeSelection,
    },
    New {
        draft: CanvasShapeDraft,
        revision: [u8; 32],
    },
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn canvas_shapes_document(
    document_id: u32,
    page: usize,
    state: State<'_, Store>,
) -> Reply<CanvasShapeProjection> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .canvas_shape_projection_for_document(document_id, page)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn begin_canvas_shape_editor(
    request: String,
    document_id: u32,
    page: usize,
    target: CanvasShapeTarget,
    state: State<'_, Store>,
) -> Reply<ShapeEditor> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_canvas_shape_editor(&request, document_id, page, target)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn cancel_canvas_shape_request(
    request: String,
    document_id: u32,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_canvas_shape_request(&request, document_id)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn cancel_shape_editor(
    id: u32,
    request: String,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_shape_editor(id, &request)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn commit_shape_editor(
    id: u32,
    request: String,
    draft: CanvasShapeDraft,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .commit_shape_editor(id, &request, draft)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn render_shape_editor_background(
    id: u32,
    request: String,
    scale: f32,
    state: State<'_, Store>,
) -> Reply<String> {
    let png = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .shape_editor_background(id, &request, scale)?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn delete_shape_editor(
    app: tauri::AppHandle,
    id: u32,
    request: String,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    let prepared = {
        let workspace = state.lock().map_err(|_| "文档状态不可用")?;
        workspace.prepare_shape_editor_delete(id, &request)?
    };
    let selected = match prepared {
        ShapeDeletePreparation::Finished(view) => return Ok(Some(view)),
        ShapeDeletePreparation::Confirm(selected) => selected,
    };
    // No Store guard crosses the native dialog. Confirmation is bound to this
    // exact editor, document epoch and immutable source bytes, never a JS bool.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dialog = app
        .dialog()
        .message("确认删除所选形状？此操作可通过撤销恢复。")
        .title("删除形状")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            "删除形状".into(),
            "取消".into(),
        ));
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
        return Ok(None);
    }
    let mut workspace = state.lock().map_err(|_| "文档状态不可用")?;
    workspace
        .apply_confirmed_shape_editor_delete(&selected)
        .map(Some)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ShapeEditor {
    pub(crate) id: u32,
    pub(crate) request: String,
    pub(crate) page: usize,
    pub(crate) target: CanvasShapeTarget,
    #[serde(skip)]
    document_id: u32,
    #[serde(skip)]
    document_epoch: u64,
    #[serde(skip)]
    snapshot: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub(crate) struct FinishedShape {
    id: u32,
    request: String,
    document_id: u32,
    document_epoch: u64,
    bytes: Arc<[u8]>,
    outcome: ShapeOutcome,
}

// Only the most recent pre-begin cancellation retains source bytes. Older
// tombstones remain reserved but their replay fails closed, not by request alone.
#[derive(Clone, Debug)]
pub(crate) struct PrebeginShapeReceipt {
    request: String,
    document_id: u32,
    document_epoch: u64,
    bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
enum ShapeOutcome {
    Cancelled,
    Committed(CanvasShapeDraft),
    Deleted,
}

// Preserve the binding Confirm/Finished(DocumentView) API. This short-lived
// native preparation owns its result only across the confirmation handoff.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ShapeDeletePreparation {
    Confirm(PreparedShapeDelete),
    Finished(DocumentView),
}

#[derive(Clone)]
pub(crate) struct PreparedShapeDelete {
    owner: ShapeEditor,
}

impl Workspace {
    pub(super) fn prepare_shape_editor_delete(
        &self,
        id: u32,
        request: &str,
    ) -> Reply<ShapeDeletePreparation> {
        if self.shape_editor.is_none() {
            if self.finished_shape_for(id, request)?.outcome == ShapeOutcome::Deleted {
                return Ok(ShapeDeletePreparation::Finished(self.view()?));
            }
            return Err("Shapes 终态不是删除。".into());
        }
        let owner = self.active_shape(id, request)?;
        let CanvasShapeTarget::Existing { original } = &owner.target else {
            return Err("尚未创建的 Shapes 不能删除。".into());
        };
        let mut candidate = Session::open(&owner.snapshot).map_err(|e| e.to_string())?;
        candidate
            .delete_canvas_shape(original)
            .map_err(|e| e.to_string())?;
        Ok(ShapeDeletePreparation::Confirm(PreparedShapeDelete {
            owner,
        }))
    }

    pub(super) fn apply_confirmed_shape_editor_delete(
        &mut self,
        selected: &PreparedShapeDelete,
    ) -> Reply<DocumentView> {
        let expected = &selected.owner;
        if self.shape_editor.is_none() {
            let finished = self.finished_shape_for(expected.id, &expected.request)?;
            if finished.document_id == expected.document_id
                && finished.document_epoch == expected.document_epoch
                && finished.outcome == ShapeOutcome::Deleted
            {
                return self.view();
            }
            return Err("Shapes 删除确认已过期。".into());
        }
        let owner = self.active_shape(expected.id, &expected.request)?;
        if &owner != expected {
            return Err("Shapes 删除选择已改变。".into());
        }
        let CanvasShapeTarget::Existing { original } = &owner.target else {
            return Err("尚未创建的 Shapes 不能删除。".into());
        };
        self.session
            .as_mut()
            .ok_or("请先打开 PDF")?
            .delete_canvas_shape(original)
            .map_err(|e| e.to_string())?;
        self.shape_editor = None;
        self.checkpoint()?;
        self.finish_shape(&owner, ShapeOutcome::Deleted)?;
        self.view()
    }
    pub(super) fn shape_editor_background(
        &self,
        id: u32,
        request: &str,
        scale: f32,
    ) -> Reply<Vec<u8>> {
        let owner = self.active_shape(id, request)?;
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        match &owner.target {
            CanvasShapeTarget::Existing { original } => session
                .render_canvas_shape_background_png(original, scale)
                .map_err(|e| e.to_string()),
            CanvasShapeTarget::New { .. } => session
                .render_png(owner.page, scale)
                .map_err(|e| e.to_string()),
        }
    }
    pub(super) fn commit_shape_editor(
        &mut self,
        id: u32,
        request: &str,
        draft: CanvasShapeDraft,
    ) -> Reply<DocumentView> {
        if self.shape_editor.is_none() {
            let finished = self.finished_shape_for(id, request)?;
            return if matches!(&finished.outcome, ShapeOutcome::Committed(saved) if saved == &draft)
            {
                self.view()
            } else {
                Err("Shapes 提交与终态不一致。".into())
            };
        }
        let owner = self.active_shape(id, request)?;
        let session = self.session.as_mut().ok_or("请先打开 PDF")?;
        let changed = match &owner.target {
            CanvasShapeTarget::Existing { original } => {
                session
                    .update_canvas_shape(original, &draft)
                    .map_err(|e| e.to_string())?
                    == pdfmaster_core::CanvasShapeCommit::Updated
            }
            CanvasShapeTarget::New { draft: initial, .. } => {
                if std::mem::discriminant(&initial.geometry)
                    != std::mem::discriminant(&draft.geometry)
                {
                    return Err("Shapes 新建类型不得改变。".into());
                }
                session
                    .create_canvas_shape(owner.page, &draft)
                    .map_err(|e| e.to_string())?;
                true
            }
        };
        self.shape_editor = None;
        if changed {
            self.checkpoint()?;
        }
        self.finish_shape(&owner, ShapeOutcome::Committed(draft))?;
        self.view()
    }

    fn finish_shape(&mut self, owner: &ShapeEditor, outcome: ShapeOutcome) -> Reply<()> {
        self.finished_shape = Some(FinishedShape {
            id: owner.id,
            request: owner.request.clone(),
            document_id: owner.document_id,
            document_epoch: owner.document_epoch,
            bytes: self
                .session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .bytes()
                .map_err(|e| e.to_string())?
                .into(),
            outcome,
        });
        Ok(())
    }
    fn validate_shape_request(request: &str) -> Reply<()> {
        if !(8..=64).contains(&request.len())
            || !request
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("Shapes 请求身份无效。".into());
        }
        Ok(())
    }

    fn shape_other_owners_idle(&self) -> Reply<()> {
        if self.note_editor.is_some()
            || self.free_text_editor.is_some()
            || self.ink_editor.is_some()
            || self.export_reservation.is_some()
        {
            return Err("其他编辑或文件操作尚未结束，未操作 Shapes。".into());
        }
        Ok(())
    }

    fn shape_identity(&self, document_id: u32, epoch: u64, bytes: &[u8]) -> Reply<()> {
        self.shape_other_owners_idle()?;
        if document_id != self.active_document_id
            || epoch != self.document_epoch
            || self
                .session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .bytes()
                .map_err(|e| e.to_string())?
                .as_slice()
                != bytes
        {
            return Err("Shapes 文档身份或内容已改变，请同步状态。".into());
        }
        Ok(())
    }

    fn active_shape(&self, id: u32, request: &str) -> Reply<ShapeEditor> {
        Self::validate_shape_request(request)?;
        let owner = self
            .shape_editor
            .as_ref()
            .filter(|o| o.id == id && o.request == request)
            .ok_or("Shapes 会话已改变。")?;
        self.shape_identity(owner.document_id, owner.document_epoch, &owner.snapshot)?;
        Ok(owner.clone())
    }

    fn finished_shape_for(&self, id: u32, request: &str) -> Reply<&FinishedShape> {
        Self::validate_shape_request(request)?;
        let finished = self
            .finished_shape
            .as_ref()
            .filter(|f| f.id == id && f.request == request)
            .ok_or("Shapes 终态不存在或已过期。")?;
        if self.shape_editor.is_some() {
            return Err("新 Shapes 编辑尚未结束。".into());
        }
        self.shape_identity(
            finished.document_id,
            finished.document_epoch,
            &finished.bytes,
        )?;
        Ok(finished)
    }

    fn shape_request_budget(&self, request: &str) -> Reply<()> {
        if !self.seen_shape_requests.contains(request) && self.seen_shape_requests.len() >= 4096 {
            return Err("Shapes 请求预算已用尽，请安全保存后重启。".into());
        }
        Ok(())
    }

    pub(super) fn canvas_shapes_for_document(
        &self,
        document_id: u32,
        page: usize,
    ) -> Reply<pdfmaster_core::CanvasShapeList> {
        if document_id != self.active_document_id {
            return Err("Shapes 请求不属于当前文档。".into());
        }
        self.session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .canvas_shapes(page)
            .map_err(|e| e.to_string())
    }

    pub(super) fn canvas_shape_projection_for_document(
        &self,
        document_id: u32,
        page: usize,
    ) -> Reply<CanvasShapeProjection> {
        let list = self.canvas_shapes_for_document(document_id, page)?;
        let hit_rows = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .annotation_hit_rows(page)
            .map_err(|e| e.to_string())?;
        if hit_rows.len() != list.annotation_count
            || hit_rows
                .iter()
                .enumerate()
                .any(|(i, r)| r.page_index != page || r.annotation_index != i)
        {
            return Err("Shapes 命中投影不完整。".into());
        }
        Ok(CanvasShapeProjection { list, hit_rows })
    }

    pub(super) fn begin_canvas_shape_editor(
        &mut self,
        request: &str,
        document_id: u32,
        page: usize,
        target: CanvasShapeTarget,
    ) -> Reply<ShapeEditor> {
        Self::validate_shape_request(request)?;
        self.shape_other_owners_idle()?;
        let list = self.canvas_shapes_for_document(document_id, page)?;
        self.shape_request_budget(request)?;
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        match &target {
            CanvasShapeTarget::Existing { original } => {
                if original.page_index != page {
                    return Err("Shapes 选择页码不一致。".into());
                }
                session
                    .validate_canvas_shape(original)
                    .map_err(|e| e.to_string())?;
            }
            CanvasShapeTarget::New { draft, revision } => {
                if *revision != list.revision {
                    return Err("Shapes 页面修订已改变。".into());
                }
                let mut candidate = Session::open(&session.bytes().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                candidate
                    .create_canvas_shape(page, draft)
                    .map_err(|e| e.to_string())?;
            }
        }
        if let Some(active) = &self.shape_editor {
            if active.request == request
                && active.document_id == document_id
                && active.page == page
                && active.target == target
            {
                return self.active_shape(active.id, request);
            }
            return Err("其他 Shapes 会话尚未结束。".into());
        }
        if self.seen_shape_requests.contains(request) {
            return Err("Shapes 请求已结束或取消。".into());
        }
        let id = self
            .next_editor_id
            .checked_add(1)
            .ok_or("Shapes 会话编号已耗尽。")?;
        let owner = ShapeEditor {
            id,
            request: request.into(),
            page,
            target,
            document_id,
            document_epoch: self.document_epoch,
            snapshot: session.bytes().map_err(|e| e.to_string())?.into(),
        };
        self.next_editor_id = id;
        self.seen_shape_requests.insert(request.into());
        self.shape_editor = Some(owner.clone());
        Ok(owner)
    }

    pub(super) fn cancel_canvas_shape_request(
        &mut self,
        request: &str,
        document_id: u32,
    ) -> Reply<DocumentView> {
        Self::validate_shape_request(request)?;
        if document_id != self.active_document_id {
            return Err("Shapes 请求不属于当前文档。".into());
        }
        self.shape_other_owners_idle()?;
        if let Some(owner) = &self.shape_editor {
            if owner.request != request || owner.document_id != document_id {
                return Err("Shapes 请求不属于当前编辑。".into());
            }
            return self.cancel_shape_editor(owner.id, request);
        }
        if self.seen_shape_requests.contains(request) {
            if let Some(finished) = self
                .finished_shape
                .as_ref()
                .filter(|f| f.request == request)
            {
                self.finished_shape_for(finished.id, request)?;
            } else {
                let receipt = self
                    .prebegin_shape
                    .as_ref()
                    .filter(|r| r.request == request && r.document_id == document_id)
                    .ok_or("Shapes 请求终态不存在或已过期。")?;
                self.shape_identity(receipt.document_id, receipt.document_epoch, &receipt.bytes)?;
            }
            return self.view();
        }
        self.shape_request_budget(request)?;
        let view = self.view()?;
        let bytes: Arc<[u8]> = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|e| e.to_string())?
            .into();
        self.prebegin_shape = Some(PrebeginShapeReceipt {
            request: request.into(),
            document_id,
            document_epoch: self.document_epoch,
            bytes,
        });
        self.seen_shape_requests.insert(request.into());
        Ok(view)
    }

    pub(super) fn cancel_shape_editor(&mut self, id: u32, request: &str) -> Reply<DocumentView> {
        if self.shape_editor.is_none() {
            self.finished_shape_for(id, request)?;
            return self.view();
        }
        let owner = self.active_shape(id, request)?;
        self.shape_editor = None;
        self.finished_shape = Some(FinishedShape {
            id,
            request: request.into(),
            document_id: owner.document_id,
            document_epoch: owner.document_epoch,
            bytes: owner.snapshot,
            outcome: ShapeOutcome::Cancelled,
        });
        self.view()
    }
}
