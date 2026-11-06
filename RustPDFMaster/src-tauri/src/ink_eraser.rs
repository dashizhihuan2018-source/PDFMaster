//! One-use, read-only preparation; whole annotation deletion stays in core.
use super::*;

#[derive(Default)]
pub(super) struct EraserState {
    next_id: u32,
    pub(super) prepared: Option<Prepared>,
}

pub(super) struct Prepared {
    capability: EraserCapability,
    epoch: u64,
    bytes: Vec<u8>,
    rows: Vec<pdfmaster_core::AnnotationRow>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct EraserCapability {
    pub(super) id: u32,
    document_id: u32,
    page: usize,
}

impl Workspace {
    pub(super) fn prepare_ink_eraser(
        &mut self,
        document_id: u32,
        page: usize,
    ) -> Reply<EraserCapability> {
        self.ensure_editor_idle()?;
        if document_id == 0 || document_id != self.active_document_id {
            return Err("橡皮文档已改变，请重新点击。".into());
        }
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        if page >= session.info().page_count {
            return Err("橡皮页面不可用。".into());
        }
        let bytes = session.bytes().map_err(|e| e.to_string())?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("文档超出橡皮会话预算；文档未修改。".into());
        }
        let rows = session
            .annotation_hit_rows(page)
            .map_err(|e| e.to_string())?;
        let id = self
            .ink_eraser
            .next_id
            .checked_add(1)
            .ok_or("橡皮会话编号已耗尽。")?;
        let capability = EraserCapability {
            id,
            document_id,
            page,
        };
        self.ink_eraser.prepared = Some(Prepared {
            capability: capability.clone(),
            epoch: self.document_epoch,
            bytes,
            rows,
        });
        self.ink_eraser.next_id = id;
        Ok(capability)
    }

    pub(super) fn erase_ink(
        &mut self,
        id: u32,
        x: f32,
        y: f32,
        tolerance_x: f32,
        tolerance_y: f32,
    ) -> Reply<Option<DocumentView>> {
        // Wrong/old IDs must never take a newer click's preparation.
        if self
            .ink_eraser
            .prepared
            .as_ref()
            .is_none_or(|p| p.capability.id != id)
        {
            return Err("橡皮点击已失效，请重新点击。".into());
        }
        let prepared = self.ink_eraser.prepared.take().ok_or("橡皮点击已失效。")?;
        self.ensure_editor_idle()?;
        if prepared.epoch != self.document_epoch
            || prepared.capability.document_id != self.active_document_id
        {
            return Err("橡皮文档已改变；文档未修改。".into());
        }
        let session = self.session.as_mut().ok_or("请先打开 PDF")?;
        if session.bytes().map_err(|e| e.to_string())? != prepared.bytes {
            return Err("橡皮点击期间文档已改变；文档未修改。".into());
        }
        let info = session.info();
        let page = info
            .pages
            .get(prepared.capability.page)
            .ok_or("橡皮页面不可用。")?;
        if ![x, y, tolerance_x, tolerance_y]
            .iter()
            .all(|v| v.is_finite())
            || x < 0.
            || y < 0.
            || x > page.width
            || y > page.height
            || !(0. ..=200.).contains(&tolerance_x)
            || !(0. ..=200.).contains(&tolerance_y)
        {
            return Err("橡皮点击或容差无效；文档未修改。".into());
        }
        let hit = prepared.rows.iter().rev().find(|row| {
            row.subtype != "Link"
                && row.bounds.is_some_and(|r| {
                    x >= r.x - tolerance_x
                        && x <= r.x + r.width + tolerance_x
                        && y >= r.y - tolerance_y
                        && y <= r.y + r.height + tolerance_y
                })
        });
        let Some(hit) = hit.filter(|row| row.subtype == "Ink") else {
            return Ok(None);
        };
        // Hit projections deliberately omit Contents. Fetch the exact full row,
        // keeping original page/array/object identity for core's transaction.
        let expected = session
            .annotations()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|row| {
                row.page_index == hit.page_index
                    && row.page_id == hit.page_id
                    && row.annotation_index == hit.annotation_index
                    && row.object_id == hit.object_id
                    && row.subtype == "Ink"
            })
            .ok_or("手绘批注身份已改变；文档未修改。")?;
        session
            .apply(Edit::DeleteAnnotation {
                expected: Box::new(expected),
            })
            .map_err(|e| e.to_string())?;
        self.checkpoint().map(Some)
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn prepare_ink_eraser(
    document_id: u32,
    page: usize,
    state: State<'_, Store>,
) -> Reply<EraserCapability> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .prepare_ink_eraser(document_id, page)
}

#[tauri::command(rename_all = "snake_case")]
pub(super) async fn erase_ink_document(
    id: u32,
    x: f32,
    y: f32,
    tolerance_x: f32,
    tolerance_y: f32,
    state: State<'_, Store>,
) -> Reply<Option<DocumentView>> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .erase_ink(id, x, y, tolerance_x, tolerance_y)
}
