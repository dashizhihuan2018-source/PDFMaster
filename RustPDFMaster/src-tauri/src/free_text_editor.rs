use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FreeTextFont {
    Sans,
    CjkSc,
    Arabic,
    Devanagari,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub(crate) struct FreeTextAppearance {
    pub(crate) font: FreeTextFont,
    pub(crate) font_size: f32,
    pub(crate) color: pdfmaster_core::Color,
    pub(crate) alignment: pdfmaster_core::TextAlignment,
    pub(crate) line_height: f32,
}

impl FreeTextAppearance {
    fn to_core(&self) -> pdfmaster_core::FreeTextAppearanceStyle {
        use pdfmaster_core::text_appearance::BundledFont;
        pdfmaster_core::FreeTextAppearanceStyle {
            font: match self.font {
                FreeTextFont::Sans => BundledFont::Sans,
                FreeTextFont::CjkSc => BundledFont::CjkSc,
                FreeTextFont::Arabic => BundledFont::Arabic,
                FreeTextFont::Devanagari => BundledFont::Devanagari,
            },
            font_size: self.font_size,
            color: self.color,
            alignment: self.alignment,
            line_height: self.line_height,
        }
    }

    fn from_core(style: pdfmaster_core::FreeTextAppearanceStyle) -> Self {
        use pdfmaster_core::text_appearance::BundledFont;
        Self {
            font: match style.font {
                BundledFont::Sans => FreeTextFont::Sans,
                BundledFont::CjkSc => FreeTextFont::CjkSc,
                BundledFont::Arabic => FreeTextFont::Arabic,
                BundledFont::Devanagari => FreeTextFont::Devanagari,
            },
            font_size: style.font_size,
            color: style.color,
            alignment: style.alignment,
            line_height: style.line_height,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CanvasFreeTextTarget {
    Existing {
        original: pdfmaster_core::FreeTextAnnotation,
    },
    New {
        bounds: pdfmaster_core::Rect,
        style: FreeTextAppearance,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct FreeTextEditor {
    pub(crate) id: u32,
    pub(crate) request: String,
    pub(crate) page: usize,
    pub(crate) target: CanvasFreeTextTarget,
    pub(crate) appearance_style: Option<FreeTextAppearance>,
    #[serde(skip)]
    pub(super) document_id: u32,
    #[serde(skip)]
    pub(super) document_epoch: u64,
    #[serde(skip)]
    pub(super) snapshot: std::sync::Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub(crate) struct FinishedFreeText {
    id: u32,
    request: String,
    document_id: u32,
    document_epoch: u64,
    outcome: FinishedFreeTextOutcome,
    bytes: std::sync::Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq)]
enum FinishedFreeTextOutcome {
    Cancelled,
    Committed(FreeTextCommitPayload),
}

#[derive(Clone, Debug, PartialEq)]
struct FreeTextCommitPayload {
    contents: String,
    bounds: pdfmaster_core::Rect,
    style: Option<FreeTextAppearance>,
}

const MAX_SEEN_REQUESTS: usize = 4096;

impl Workspace {
    pub(super) fn free_text_editor_background(
        &self,
        id: u32,
        request: &str,
        scale: f32,
    ) -> Reply<Vec<u8>> {
        Self::validate_free_text_request(request)?;
        let editor = self
            .free_text_editor
            .as_ref()
            .filter(|editor| editor.id == id && editor.request == request)
            .ok_or("FreeText 会话已改变，未渲染编辑背景。")?;
        if self.note_editor.is_some()
            || self.shape_editor.is_some()
            || self.ink_editor.is_some()
            || self.export_reservation.is_some()
        {
            return Err("其他编辑或文件操作尚未结束，未渲染 FreeText 背景。".into());
        }
        if editor.document_id != self.active_document_id {
            return Err("FreeText 编辑期间文档身份已改变，未渲染背景。".into());
        }
        if editor.document_epoch != self.document_epoch {
            return Err("FreeText 编辑期间文档身份已改变，未渲染背景。".into());
        }
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        if session
            .bytes()
            .map_err(|error| error.to_string())?
            .as_slice()
            != editor.snapshot.as_ref()
        {
            return Err("FreeText 编辑期间文档已改变，未渲染背景。".into());
        }
        match &editor.target {
            CanvasFreeTextTarget::Existing { original } => session
                .render_free_text_background_png(original, scale)
                .map_err(|error| error.to_string()),
            CanvasFreeTextTarget::New { .. } => session
                .render_png(editor.page, scale)
                .map_err(|error| error.to_string()),
        }
    }

    pub(super) fn begin_canvas_free_text_editor(
        &mut self,
        request: &str,
        document_id: u32,
        page: usize,
        target: CanvasFreeTextTarget,
    ) -> Reply<FreeTextEditor> {
        Self::validate_free_text_request(request)?;
        if document_id != self.active_document_id {
            return Err("FreeText 请求不属于当前文档，请同步状态。".into());
        }
        if !self.seen_free_text_requests.contains(request)
            && self.seen_free_text_requests.len() >= MAX_SEEN_REQUESTS
        {
            return Err("FreeText 恢复请求预算已用尽，请先保存文档并正常重启。".into());
        }
        let appearance_style = self.validate_free_text_target(page, &target)?;
        if let Some(active) = &self.free_text_editor
            && active.request == request
            && active.document_id == document_id
            && active.page == page
            && active.target == target
        {
            let current = self.session.as_ref().ok_or("请先打开 PDF")?;
            if active.document_epoch != self.document_epoch
                || current
                    .bytes()
                    .map_err(|error| error.to_string())?
                    .as_slice()
                    != active.snapshot.as_ref()
            {
                return Err("FreeText 编辑期间文档已改变，未重放开始请求。".into());
            }
            return Ok(active.clone());
        }
        if self.seen_free_text_requests.contains(request) {
            return Err("该 FreeText 请求已结束或取消，未重新开始。".into());
        }
        self.ensure_editor_idle()?;
        let snapshot: std::sync::Arc<[u8]> = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|error| error.to_string())?
            .into();
        let id = self
            .next_editor_id
            .checked_add(1)
            .ok_or("FreeText 会话编号已耗尽，请安全保存后重启。")?;
        let editor = FreeTextEditor {
            id,
            request: request.to_owned(),
            page,
            appearance_style,
            target,
            document_id,
            document_epoch: self.document_epoch,
            snapshot,
        };
        self.seen_free_text_requests.insert(request.to_owned());
        self.next_editor_id = id;
        self.free_text_editor = Some(editor.clone());
        Ok(editor)
    }

    pub(super) fn cancel_canvas_free_text_request(
        &mut self,
        request: &str,
        document_id: u32,
    ) -> Reply<DocumentView> {
        Self::validate_free_text_request(request)?;
        if document_id != self.active_document_id {
            return Err("FreeText 请求不属于当前文档，请同步状态。".into());
        }
        if !self.seen_free_text_requests.contains(request)
            && self.seen_free_text_requests.len() >= MAX_SEEN_REQUESTS
        {
            return Err("FreeText 恢复请求预算已用尽，请先保存文档并正常重启。".into());
        }
        self.seen_free_text_requests.insert(request.to_owned());
        if let Some(active) = &self.free_text_editor {
            if active.request != request || active.document_id != document_id {
                return Err("该恢复请求不属于当前 FreeText，未取消新编辑。".into());
            }
            return self.cancel_free_text_editor(active.id, request);
        }
        self.view()
    }

    pub(super) fn cancel_free_text_editor(
        &mut self,
        id: u32,
        request: &str,
    ) -> Reply<DocumentView> {
        Self::validate_free_text_request(request)?;
        if self.free_text_editor.is_none() {
            let finished = self
                .finished_free_text
                .as_ref()
                .filter(|finished| finished.id == id && finished.request == request)
                .ok_or("FreeText 会话已改变，未取消新内容。")?;
            self.validate_finished_free_text(finished)?;
            return self.view();
        }
        let active = self
            .free_text_editor
            .as_ref()
            .filter(|editor| editor.id == id && editor.request == request)
            .ok_or("FreeText 会话已改变，未取消新内容。")?
            .clone();
        if active.document_id != self.active_document_id
            || active.document_epoch != self.document_epoch
            || self
                .session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .bytes()
                .map_err(|error| error.to_string())?
                .as_slice()
                != active.snapshot.as_ref()
        {
            return Err("FreeText 编辑期间文档已改变，未取消内容。".into());
        }
        self.free_text_editor = None;
        self.finished_free_text = Some(FinishedFreeText {
            id,
            request: request.to_owned(),
            document_id: active.document_id,
            document_epoch: active.document_epoch,
            outcome: FinishedFreeTextOutcome::Cancelled,
            bytes: active.snapshot,
        });
        self.view()
    }

    pub(super) fn commit_free_text_editor(
        &mut self,
        id: u32,
        request: &str,
        contents: String,
        bounds: pdfmaster_core::Rect,
        style: Option<FreeTextAppearance>,
    ) -> Reply<DocumentView> {
        Self::validate_free_text_request(request)?;
        if contents.len() > 65_536 {
            return Err("FreeText 正文不得超过 64KiB。".into());
        }
        let payload = FreeTextCommitPayload {
            contents: contents.clone(),
            bounds,
            style: style.clone(),
        };
        if self.free_text_editor.is_none() {
            let finished = self
                .finished_free_text
                .as_ref()
                .filter(|finished| finished.id == id && finished.request == request)
                .ok_or("FreeText 会话已改变，请同步状态。")?;
            self.validate_finished_free_text(finished)?;
            if matches!(&finished.outcome, FinishedFreeTextOutcome::Committed(saved) if saved == &payload)
            {
                return self.view();
            }
            return Err("FreeText 已结束，提交内容与终态不一致。".into());
        }
        let editor = self
            .free_text_editor
            .as_ref()
            .filter(|editor| editor.id == id && editor.request == request)
            .ok_or("FreeText 会话已改变，请同步状态。")?
            .clone();
        if editor.document_id != self.active_document_id
            || editor.document_epoch != self.document_epoch
        {
            return Err("FreeText 编辑期间文档身份已改变，未提交内容。".into());
        }
        if self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|error| error.to_string())?
            .as_slice()
            != editor.snapshot.as_ref()
        {
            return Err("FreeText 编辑期间文档已改变，未提交内容。".into());
        }
        let page_info = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .info()
            .pages
            .get(editor.page)
            .cloned()
            .ok_or("FreeText 页码无效。")?;
        Self::validate_free_text_geometry(page_info.width, page_info.height, bounds)?;
        if let Some(explicit) = &style {
            Self::validate_free_text_style(explicit)?;
        }
        let session = self.session.as_mut().ok_or("请先打开 PDF")?;
        let commit = match (&editor.target, &style) {
            (CanvasFreeTextTarget::New { style: initial, .. }, None) => session
                .create_free_text(
                    editor.page,
                    &pdfmaster_core::FreeTextDraft {
                        contents: &contents,
                        bounds,
                        style: initial.to_core(),
                    },
                )
                .map_err(|error| error.to_string())?,
            (CanvasFreeTextTarget::New { .. }, Some(explicit)) => session
                .create_free_text(
                    editor.page,
                    &pdfmaster_core::FreeTextDraft {
                        contents: &contents,
                        bounds,
                        style: explicit.to_core(),
                    },
                )
                .map_err(|error| error.to_string())?,
            (CanvasFreeTextTarget::Existing { original }, None) => session
                .update_free_text(
                    original,
                    &pdfmaster_core::FreeTextUpdate {
                        contents: &contents,
                        bounds,
                        style: None,
                    },
                )
                .map_err(|error| error.to_string())?,
            (CanvasFreeTextTarget::Existing { original }, Some(explicit)) => session
                .update_free_text(
                    original,
                    &pdfmaster_core::FreeTextUpdate {
                        contents: &contents,
                        bounds,
                        style: Some(explicit.to_core()),
                    },
                )
                .map_err(|error| error.to_string())?,
        };
        let bytes: std::sync::Arc<[u8]> =
            session.bytes().map_err(|error| error.to_string())?.into();
        self.free_text_editor = None;
        self.finished_free_text = Some(FinishedFreeText {
            id,
            request: request.to_owned(),
            document_id: editor.document_id,
            document_epoch: editor.document_epoch,
            outcome: FinishedFreeTextOutcome::Committed(payload),
            bytes,
        });
        if commit == pdfmaster_core::FreeTextCommit::Unchanged {
            self.view()
        } else {
            self.checkpoint()
        }
    }

    fn validate_free_text_target(
        &self,
        page: usize,
        target: &CanvasFreeTextTarget,
    ) -> Reply<Option<FreeTextAppearance>> {
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        if page >= session.info().page_count {
            return Err("FreeText 页码无效。".into());
        }
        match target {
            CanvasFreeTextTarget::Existing { original } => {
                if original.contents.len() > 65_536
                    || original.style.font_resource.len() > 4_096
                    || original.annotation_index >= 1_024
                {
                    return Err("FreeText 原始批注超出预算。".into());
                }
                let current = session
                    .free_texts(page)
                    .map_err(|error| error.to_string())?
                    .annotations
                    .into_iter()
                    .any(|candidate| candidate == *original);
                if !current {
                    return Err("FreeText 已改变或不受支持，请刷新后重试。".into());
                }
                session
                    .free_text_appearance_style(original)
                    .map(|style| style.map(FreeTextAppearance::from_core))
                    .map_err(|error| error.to_string())
            }
            CanvasFreeTextTarget::New { bounds, style } => {
                Self::validate_free_text_geometry(
                    session.info().pages[page].width,
                    session.info().pages[page].height,
                    *bounds,
                )?;
                Self::validate_free_text_style(style)?;
                Ok(Some(style.clone()))
            }
        }
    }

    fn validate_free_text_geometry(
        page_width: f32,
        page_height: f32,
        bounds: pdfmaster_core::Rect,
    ) -> Reply<()> {
        let coordinates = [bounds.x, bounds.y, bounds.width, bounds.height];
        let right = bounds.x + bounds.width;
        let top = bounds.y + bounds.height;
        if coordinates.iter().any(|value| !value.is_finite())
            || !right.is_finite()
            || !top.is_finite()
            || bounds.x < 0.0
            || bounds.y < 0.0
            || bounds.width <= 0.0
            || bounds.height <= 0.0
            || bounds.width > 14_400.0
            || bounds.height > 14_400.0
            || right > page_width
            || top > page_height
        {
            return Err("FreeText 位置或页面几何无效。".into());
        }
        Ok(())
    }

    fn validate_free_text_style(style: &FreeTextAppearance) -> Reply<()> {
        if !style.font_size.is_finite()
            || style.font_size <= 0.0
            || style.font_size > 1_000.0
            || !style.line_height.is_finite()
            || !(1.0..=4.0).contains(&style.line_height)
            || [style.color.r, style.color.g, style.color.b]
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("FreeText 样式无效。".into());
        }
        Ok(())
    }

    fn validate_free_text_request(request: &str) -> Reply<()> {
        if !(8..=64).contains(&request.len())
            || !request
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("FreeText 请求身份无效。".into());
        }
        Ok(())
    }

    fn validate_finished_free_text(&self, finished: &FinishedFreeText) -> Reply<()> {
        if self.free_text_editor.is_some()
            || finished.document_id != self.active_document_id
            || finished.document_epoch != self.document_epoch
            || self
                .session
                .as_ref()
                .ok_or("请先打开 PDF")?
                .bytes()
                .map_err(|error| error.to_string())?
                .as_slice()
                != finished.bytes.as_ref()
        {
            return Err("FreeText 终态已过期，请同步状态。".into());
        }
        self.ensure_editor_idle()
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn free_texts_document(
    document_id: u32,
    page: usize,
    state: State<'_, Store>,
) -> Reply<pdfmaster_core::FreeTextList> {
    let workspace = state.lock().map_err(|_| "文档状态不可用")?;
    if document_id != workspace.active_document_id {
        return Err("FreeText 请求不属于当前文档，请同步状态。".into());
    }
    workspace
        .session
        .as_ref()
        .ok_or("请先打开 PDF")?
        .free_texts(page)
        .map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn begin_canvas_free_text_editor(
    request: String,
    document_id: u32,
    page: usize,
    target: CanvasFreeTextTarget,
    state: State<'_, Store>,
) -> Reply<FreeTextEditor> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .begin_canvas_free_text_editor(&request, document_id, page, target)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn cancel_canvas_free_text_request(
    request: String,
    document_id: u32,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_canvas_free_text_request(&request, document_id)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn cancel_free_text_editor(
    id: u32,
    request: String,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .cancel_free_text_editor(id, &request)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn commit_free_text_editor(
    id: u32,
    request: String,
    contents: String,
    bounds: pdfmaster_core::Rect,
    style: Option<FreeTextAppearance>,
    state: State<'_, Store>,
) -> Reply<DocumentView> {
    state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .commit_free_text_editor(id, &request, contents, bounds, style)
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn render_free_text_editor_background(
    id: u32,
    request: String,
    scale: f32,
    state: State<'_, Store>,
) -> Reply<String> {
    let png = state
        .lock()
        .map_err(|_| "文档状态不可用")?
        .free_text_editor_background(id, &request, scale)?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}
