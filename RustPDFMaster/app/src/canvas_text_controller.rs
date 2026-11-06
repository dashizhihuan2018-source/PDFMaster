//! Protected native transaction coordinator and decoded-image handoff.
use super::*;
use pdfmaster_drawing::Point;
use pdfmaster_ui::annotations::Row;
use pdfmaster_ui::canvas_text::{
    self as text, Appearance, Handle, Owner, Phase, Rect, Target, Ticket,
};
use wasm_bindgen::JsValue;

use super::canvas_intent::Intent;
#[derive(Clone, Debug)]
pub(super) struct PageTexts {
    document_id: u32,
    page: usize,
    source: String,
    ticket: Option<u64>,
    result: Result<text::List, String>,
    rows: Result<Vec<Row>, String>,
}
#[derive(Clone, Debug)]
pub(super) struct Pointer {
    pointer: i32,
    start: Point,
    frame: text::Frame,
    source: String,
    document_id: u32,
    page: usize,
    ticket: Option<u64>,
    moved: bool,
    drag: Option<(Handle, Rect, [f32; 6])>,
}
#[derive(Clone, Debug)]
pub(super) struct Preview {
    pub ticket: Ticket,
    pub attempt: u64,
    pub source: String,
    pub terminal: Option<Document>,
    pub installed: bool,
}

// serde_wasm_bindgen's default None is JS undefined, which can drop the field
// at the IPC boundary. D1's complete retry payload explicitly retains null.
#[derive(serde::Serialize)]
struct CommitWire<'a> {
    id: u32,
    request: &'a str,
    contents: &'a str,
    bounds: Rect,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    style: JsValue,
}

fn crypto_request() -> Option<String> {
    let mut bytes = [0_u8; 16];
    web_sys::window()?
        .crypto()
        .ok()?
        .get_random_values_with_u8_array(&mut bytes)
        .ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn frame(image: &web_sys::HtmlImageElement) -> text::Frame {
    let r = image.get_bounding_client_rect();
    text::Frame {
        left: r.left() as f32,
        top: r.top() as f32,
        width: r.width() as f32,
        height: r.height() as f32,
    }
}
pub(super) fn font_stack(font: text::Font) -> String {
    let selected = match font {
        text::Font::Sans => "PM Sans",
        text::Font::CjkSc => "PM CJK",
        text::Font::Arabic => "PM Arabic",
        text::Font::Devanagari => "PM Devanagari",
    };
    format!("'{selected}','PM Sans','PM CJK','PM Arabic','PM Devanagari'")
}
async fn load_fonts() -> Result<(), String> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("无法加载打包字体")?;
    let fonts = js_sys::Reflect::get(document.as_ref(), &JsValue::from_str("fonts"))
        .map_err(|_| "字体接口不可用")?;
    let load = js_sys::Reflect::get(&fonts, &JsValue::from_str("load"))
        .ok()
        .and_then(|v| v.dyn_into::<js_sys::Function>().ok())
        .ok_or("字体接口不可用")?;
    let promises = js_sys::Array::new();
    for name in ["PM Sans", "PM CJK", "PM Arabic", "PM Devanagari"] {
        let promise = load
            .call1(&fonts, &JsValue::from_str(&format!("18px '{name}'")))
            .map_err(|_| "打包字体加载失败")?;
        promises.push(&promise);
    }
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::all(&promises))
        .await
        .map_err(|_| "打包字体加载失败")?;
    Ok(())
}

impl Model {
    pub(super) fn has_free_text_draft(self) -> bool {
        self.free_text.with(|s| s.active())
            || self
                .document
                .with(|d| d.as_ref().is_some_and(|d| d.free_text_editor.is_some()))
    }
    pub(super) fn defer_free_text(self, intent: Option<Intent>) -> bool {
        if !self.has_free_text_draft() {
            return false;
        }
        self.free_text.update(|s| s.queue(intent));
        self.submit_free_text(false);
        true
    }
    pub(super) async fn refresh_free_texts(self) -> Result<(), String> {
        let Some(doc) = self.document.get_untracked() else {
            return Err("文档已切换".into());
        };
        let page = self.page.get_untracked();
        let source = self.image.get_untracked();
        let ticket = self.annotation_image_ticket.get_untracked();
        let result = bridge::call_bounded::<text::List>(
            "free_texts_document",
            &json!({"document_id":doc.id,"page":page}),
        )
        .await;
        // Generic projection distinguishes unsupported FreeText hits from blank space.
        let rows = bridge::call_bounded::<Vec<Row>>("annotations_document", &json!({})).await;
        if self.document.with_untracked(|d| d.as_ref().map(|d| d.id)) == Some(doc.id)
            && self.page.get_untracked() == page
            && self.image.get_untracked() == source
            && self.annotation_image_ticket.get_untracked() == ticket
        {
            let status = result
                .as_ref()
                .map(|_| ())
                .map_err(Clone::clone)
                .and_then(|()| rows.as_ref().map(|_| ()).map_err(Clone::clone));
            self.free_text_list.set(Some(PageTexts {
                document_id: doc.id,
                page,
                source,
                ticket,
                result,
                rows,
            }));
            status
        } else {
            Err("文档或页面已切换".into())
        }
    }
    pub(super) fn recover_free_text(self, doc: &Document) -> bool {
        let Some(owner) = doc.free_text_editor.clone() else {
            return false;
        };
        let same = self
            .free_text
            .with_untracked(|s| s.matches_owner(doc.id, &owner));
        if !same && !self.free_text.with_untracked(|s| s.active()) {
            self.free_text.update(|s| {
                if let Some(ticket) = s.begin(
                    doc.id,
                    owner.page,
                    owner.request.clone(),
                    owner.target.clone(),
                ) {
                    s.accept(&ticket, owner);
                }
            });
            self.message.set(
                "文字编辑尚未结束；当前恢复原始内容，未提交输入没有持久化，请核对后继续或取消。"
                    .into(),
            );
        }
        self.page
            .set(doc.free_text_editor.as_ref().map(|o| o.page).unwrap_or(0));
        self.overview_visible.set(false);
        self.chrome.update(|c| {
            c.mode = Some(Mode::Annotation);
            c.panel = None;
            c.active_tool = Some(ActiveTool::Text);
        });
        !same
    }
    pub(super) fn resume_free_text(self) {
        if self
            .free_text
            .with_untracked(|s| s.phase == Phase::Preparing)
            && self.free_text_preview.get_untracked().is_none()
        {
            self.prepare_free_text_background();
        }
    }
    pub(super) fn sync_free_text(self) {
        let Some((ticket, phase)) = self.free_text.with_untracked(|s| {
            matches!(s.phase, Phase::Editing | Phase::Uncertain)
                .then(|| Some((s.ticket.clone()?, s.phase)))
                .flatten()
        }) else {
            return;
        };
        if self.free_text_sync.get_untracked().is_some() {
            return;
        }
        self.free_text_sync.set(Some(ticket.clone()));
        spawn_local(async move {
            let result = bridge::call_bounded::<Document>("document_info", &json!({})).await;
            if self.free_text_sync.get_untracked().as_ref() == Some(&ticket) {
                self.free_text_sync.set(None);
            }
            if !self
                .free_text
                .with_untracked(|s| s.owns(&ticket) && s.phase == phase)
            {
                return;
            }
            match result{
                Ok(document) if document.free_text_editor.as_ref().is_some_and(|o|self.free_text.with_untracked(|s|s.matches_owner(document.id,o)))=>{
                    // A read-only refresh is not a commit or an image handoff.
                    // Keep textarea node/value/selection and protected D2a source.
                    self.document.set(Some(document));
                }
                Ok(_)=>self.free_text.update(|s|{s.phase=Phase::Uncertain;s.error=Some("只读状态同步冲突：文档或完整文字所有权不匹配；本地未提交正文保留，未认定提交成功。请取消／同步确认后继续。".into());}),
                Err(error)=>self.free_text_error(format!("只读状态同步失败，草稿已保留：{error}")),
            }
        });
    }
    pub(super) fn free_text_pointer_down(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Text) {
            return false;
        }
        if self.has_free_text_draft() {
            return true;
        }
        if self.blocked()
            || !self.image_ready.get_untracked()
            || !event.is_primary()
            || event.button() != 0
        {
            return true;
        }
        let Some(image) = annotation_view::decoded_page_image(self) else {
            self.message
                .set("页面图片尚未解码，未开始文字编辑。".into());
            return true;
        };
        let Some(doc) = self.document.get_untracked() else {
            return true;
        };
        let page = self.page.get_untracked();
        let Some(node) = surface.get() else {
            return true;
        };
        if node.set_pointer_capture(event.pointer_id()).is_err() {
            return true;
        }
        self.free_text_pointer.set(Some(Pointer {
            pointer: event.pointer_id(),
            start: Point {
                x: event.client_x() as f32,
                y: event.client_y() as f32,
            },
            frame: frame(&image),
            source: self.image.get_untracked(),
            document_id: doc.id,
            page,
            ticket: self.annotation_image_ticket.get_untracked(),
            moved: false,
            drag: None,
        }));
        event.prevent_default();
        true
    }
    pub(super) fn free_text_pointer_move(self, event: &web_sys::PointerEvent) -> bool {
        let Some(mut pointer) = self.free_text_pointer.get_untracked() else {
            return false;
        };
        if pointer.pointer != event.pointer_id() {
            return true;
        }
        let point = Point {
            x: event.client_x() as f32,
            y: event.client_y() as f32,
        };
        pointer.moved |= !pdfmaster_ui::canvas_notes::is_click(pointer.start, point);
        if let Some((handle, bounds, matrix)) = pointer.drag {
            if let Some(image) = annotation_view::decoded_page_image(self)
                && !frame(&image).changed_from(pointer.frame)
                && self.document.with_untracked(|d| d.as_ref().map(|d| d.id))
                    == Some(pointer.document_id)
                && self.page.get_untracked() == pointer.page
                && self.image.get_untracked() == pointer.source
            {
                if let Some(delta) = text::local_delta(
                    matrix,
                    Point {
                        x: point.x - pointer.start.x,
                        y: point.y - pointer.start.y,
                    },
                ) && let Some(page) = self
                    .document
                    .with_untracked(|d| d.as_ref()?.info.pages.get(pointer.page).cloned())
                    && let Some(rect) =
                        text::drag_rect(bounds, delta, handle, page.width, page.height)
                {
                    self.free_text.update(|s| {
                        if s.phase == Phase::Editing
                            && let Some(d) = &mut s.draft
                            && !d.geometry_locked()
                        {
                            d.bounds = rect;
                            if handle != Handle::Move {
                                d.manual_width = true;
                            }
                        }
                    });
                }
            } else {
                self.free_text_pointer.set(None);
                return true;
            }
        }
        self.free_text_pointer.set(Some(pointer));
        event.prevent_default();
        true
    }
    pub(super) fn free_text_pointer_up(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        let Some(pointer) = self.free_text_pointer.get_untracked() else {
            return false;
        };
        if pointer.pointer != event.pointer_id() {
            return true;
        }
        self.free_text_pointer.set(None);
        if let Some(node) = surface.get() {
            let _ = node.release_pointer_capture(event.pointer_id());
        }
        if let Some((handle, _, _)) = pointer.drag {
            // Preserve every manually resized rectangle. Only a move that
            // makes room for a previously unlaid-out body resumes autosizing.
            let adapt = handle == Handle::Move
                && self.free_text.with_untracked(|s| {
                    s.error.as_ref().is_some_and(|e| e.starts_with("正文超出"))
                });
            self.measure_free_text_layout(adapt);
            return true;
        }
        let Some(image) = annotation_view::decoded_page_image(self) else {
            return true;
        };
        let current_frame = frame(&image);
        let end = Point {
            x: event.client_x() as f32,
            y: event.client_y() as f32,
        };
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Text)
            || pointer.moved
            || !event.is_primary()
            || event.button() != 0
            || !pdfmaster_ui::canvas_notes::is_click(pointer.start, end)
            || current_frame.changed_from(pointer.frame)
            || self.blocked()
            || self.image.get_untracked() != pointer.source
            || self.page.get_untracked() != pointer.page
            || self.annotation_image_ticket.get_untracked() != pointer.ticket
            || self.document.with_untracked(|d| d.as_ref().map(|d| d.id))
                != Some(pointer.document_id)
        {
            return true;
        }
        let Some(page) = self
            .document
            .with_untracked(|d| d.as_ref()?.info.pages.get(pointer.page).cloned())
        else {
            return true;
        };
        let Ok(point) = pdfmaster_ui::canvas_notes::click_page_point(
            end,
            current_frame,
            page.width,
            page.height,
            page.rotation,
        ) else {
            return true;
        };
        let Some(list) = self.free_text_list.get_untracked().filter(|l| {
            l.document_id == pointer.document_id
                && l.page == pointer.page
                && l.source == pointer.source
                && l.ticket == pointer.ticket
        }) else {
            self.message
                .set("文字批注列表尚未就绪，请等待后重试。".into());
            return true;
        };
        let (Ok(list), Ok(rows)) = (list.result, list.rows) else {
            self.message
                .set("文字批注读取失败，未把未知结果当作空白页；请重新渲染后重试。".into());
            return true;
        };
        let contains = |r: Rect| {
            pdfmaster_ui::canvas_notes::note_contains([r.x, r.y, r.width, r.height], point)
        };
        let hit = list
            .annotations
            .into_iter()
            .rev()
            .find(|a| contains(a.bounds));
        let target = if let Some(original) = hit {
            Target::Existing { original }
        } else {
            if rows.iter().any(|r| {
                r.page_index == pointer.page
                    && r.subtype == "FreeText"
                    && r.bounds.is_some_and(|b| {
                        contains(Rect {
                            x: b.x,
                            y: b.y,
                            width: b.width,
                            height: b.height,
                        })
                    })
            }) {
                self.message
                    .set("此处文字批注样式暂不支持原位编辑；原件已保留。".into());
                return true;
            }
            let height = (18. * 1.3_f32).min(page.height);
            let width = 120_f32.min(page.width);
            Target::New {
                bounds: Rect {
                    x: point.x.min(page.width - width),
                    y: (point.y - height).clamp(0., page.height - height),
                    width,
                    height,
                },
                style: Appearance::default(),
            }
        };
        let Some(request) = crypto_request() else {
            self.message.set("无法生成受保护文字请求。".into());
            return true;
        };
        let mut ticket = None;
        self.free_text.update(|s| {
            ticket = s.begin(
                pointer.document_id,
                pointer.page,
                request.clone(),
                target.clone(),
            )
        });
        let Some(ticket) = ticket else {
            return true;
        };
        spawn_local(async move {
            let result=bridge::call_bounded::<Owner>("begin_canvas_free_text_editor",&json!({"request":request,"document_id":pointer.document_id,"page":pointer.page,"target":target})).await;
            if !self
                .free_text
                .with_untracked(|s| s.owns(&ticket) && s.phase == Phase::BeginPending)
            {
                return;
            }
            match result {
                Ok(owner) => {
                    let mut accepted = false;
                    self.free_text
                        .update(|s| accepted = s.accept(&ticket, owner));
                    if accepted {
                        self.prepare_free_text_background();
                    } else {
                        self.free_text_error("文字编辑回执身份不符；请取消／同步后继续。".into());
                    }
                }
                Err(error) => {
                    self.free_text_error(error);
                }
            }
        });
        true
    }
    pub(super) fn free_text_pointer_cancel(self, pointer: i32) -> bool {
        let owned = self
            .free_text_pointer
            .with_untracked(|p| p.as_ref().is_some_and(|p| p.pointer == pointer));
        if owned {
            self.free_text_pointer.set(None);
        }
        owned
    }
    fn free_text_error(self, error: String) {
        self.message.set(error.clone());
        self.free_text.update(|s| s.error = Some(error));
    }
    fn next_free_text_attempt(self) -> Option<u64> {
        let attempt = self.free_text_attempt.get_untracked().checked_add(1)?;
        self.free_text_attempt.set(attempt);
        Some(attempt)
    }
    pub(super) fn prepare_free_text_background(self) {
        let Some((ticket, owner)) = self
            .free_text
            .with_untracked(|s| Some((s.ticket.clone()?, s.draft.as_ref()?.owner.clone())))
        else {
            return;
        };
        let Some(plan) = self.raster() else {
            return;
        };
        let Some(attempt) = self.next_free_text_attempt() else {
            return;
        };
        self.free_text.update(|s| s.error = None);
        spawn_local(async move {
            let result = bridge::call_bounded::<String>(
                "render_free_text_editor_background",
                &json!({"id":owner.id,"request":owner.request,"scale":plan.scale}),
            )
            .await;
            if !self
                .free_text
                .with_untracked(|s| s.owns(&ticket) && s.phase == Phase::Preparing)
                || self.free_text_attempt.get_untracked() != attempt
            {
                return;
            }
            match result {
                Ok(source) => {
                    let fonts = load_fonts().await;
                    if !self
                        .free_text
                        .with_untracked(|s| s.owns(&ticket) && s.phase == Phase::Preparing)
                        || self.free_text_attempt.get_untracked() != attempt
                    {
                        return;
                    }
                    if let Err(error) = fonts {
                        self.free_text_error(error);
                        return;
                    }
                    self.free_text_fonts.set(true);
                    self.stage_free_text(ticket, attempt, source, None);
                }
                Err(error) => {
                    self.free_text_error(format!("编辑背景未就绪：{error}；原页与编辑所有权保留。"))
                }
            }
        });
    }
    fn stage_free_text(
        self,
        ticket: Ticket,
        attempt: u64,
        source: String,
        terminal: Option<Document>,
    ) {
        self.free_text_preview.set(Some(Preview {
            ticket: ticket.clone(),
            attempt,
            source,
            terminal,
            installed: false,
        }));
        let _ = leptos::leptos_dom::helpers::set_timeout_with_handle(
            move || {
                if self.free_text_attempt.get_untracked() == attempt
                    && self.free_text.with_untracked(|s| {
                        s.owns(&ticket) && matches!(s.phase, Phase::Preparing | Phase::Handoff)
                    })
                    && self.free_text_preview.with_untracked(|p| {
                        p.as_ref().is_some_and(|p| {
                            p.ticket == ticket && p.attempt == attempt && !p.installed
                        })
                    })
                {
                    self.free_text_error(
                        "文字页面解码超时；草稿与原页面已保留，请重试背景或取消／同步。".into(),
                    );
                }
            },
            std::time::Duration::from_secs(15),
        );
    }
    pub(super) fn free_text_stage_decoded(self, ticket: &Ticket, attempt: u64, source: &str) {
        let valid = self.free_text.with_untracked(|s| {
            s.owns(ticket) && matches!(s.phase, Phase::Preparing | Phase::Handoff)
        }) && self.free_text_attempt.get_untracked() == attempt
            && self.free_text_preview.with_untracked(|p| {
                p.as_ref().is_some_and(|p| {
                    p.ticket == *ticket
                        && p.attempt == attempt
                        && p.source == source
                        && !p.installed
                })
            });
        if !valid {
            return;
        }
        self.free_text_preview.update(|p| {
            if let Some(p) = p {
                p.installed = true;
            }
        });
        self.image.set(source.into());
        self.image_ready.set(false);
        // The normal decoded-page guard also covers equal source cache hits.
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if annotation_view::decoded_page_image(self).is_some() {
                self.free_text_image_loaded(Some(&self.image.get_untracked()));
            }
        });
    }
    pub(super) fn free_text_stage_failed(self, ticket: &Ticket, attempt: u64, source: &str) {
        if self.free_text.with_untracked(|s| {
            s.owns(ticket) && matches!(s.phase, Phase::Preparing | Phase::Handoff)
        }) && self.free_text_attempt.get_untracked() == attempt
            && self.free_text_preview.with_untracked(|p| {
                p.as_ref().is_some_and(|p| {
                    p.ticket == *ticket
                        && p.attempt == attempt
                        && p.source == source
                        && !p.installed
                })
            })
        {
            self.free_text_error("文字页面解码失败；草稿与原页面保留。".into());
        }
    }
    pub(super) fn free_text_image_loaded(self, source: Option<&str>) {
        let Some(preview) = self.free_text_preview.get_untracked().filter(|p| {
            p.installed
                && p.attempt == self.free_text_attempt.get_untracked()
                && source == Some(p.source.as_str())
        }) else {
            return;
        };
        if !self.free_text.with_untracked(|s| s.owns(&preview.ticket))
            || annotation_view::decoded_page_image(self).is_none()
        {
            return;
        }
        // Equal-source handoffs do not produce another <img> load event.
        self.image_ready.set(true);
        if let Some(document) = preview.terminal {
            let mut intent = None;
            self.free_text.update(|s| {
                intent = s.finish(
                    &preview.ticket,
                    document.id,
                    document.free_text_editor.is_none(),
                )
            });
            if self.free_text.with_untracked(|s| s.active()) {
                return;
            }
            self.free_text_preview.set(None);
            self.free_text_frame.set(None);
            self.free_text_sync.set(None);
            self.replace_document_view(Some(document));
            // The decoded normal page no longer needs an overlay, but the
            // captured action still owns the UI until both projections settle.
            let ticket = preview.ticket;
            self.free_text_completion.set(Some(ticket.clone()));
            self.busy.set(true);
            let source = self.image.get_untracked();
            let image_ticket = self.annotation_image_ticket.get_untracked();
            self.message.set("文字编辑已完成；已刷新页面。".into());
            spawn_local(async move {
                let result = self.refresh_free_texts().await;
                // Never release a successor's busy state or dispatch an old
                // action into a new document, page, image or native owner.
                if self.free_text_completion.get_untracked().as_ref() != Some(&ticket) {
                    return;
                }
                self.free_text_completion.set(None);
                let current = self.route.get_untracked() == Route::Document
                    && self.page.get_untracked() == ticket.page
                    && self.image.get_untracked() == source
                    && self.annotation_image_ticket.get_untracked() == image_ticket
                    && self.document.with_untracked(|d| {
                        d.as_ref().is_some_and(|d| {
                            d.id == ticket.document_id && d.free_text_editor.is_none()
                        })
                    })
                    && self.free_text.with_untracked(|s| s.phase == Phase::Idle);
                if !current {
                    return;
                }
                self.busy.set(false);
                match result {
                    Ok(()) => {
                        if let Some(intent) = intent {
                            self.execute_canvas_intent(intent);
                        }
                    }
                    Err(error) => self
                        .message
                        .set(format!("文字已完成，未执行后续操作；列表同步失败：{error}")),
                }
            });
        } else {
            self.free_text.update(|s| {
                if s.phase == Phase::Preparing {
                    s.phase = Phase::Editing;
                    s.error = None;
                }
            });
            self.free_text_preview.set(None);
            self.measure_free_text_frame();
            self.focus_free_text();
            if self
                .free_text
                .with_untracked(|s| s.finish_requested && !s.composing)
            {
                self.submit_free_text(false);
            }
        }
    }
    pub(super) fn measure_free_text_frame(self) {
        if let Some(image) = annotation_view::decoded_page_image(self) {
            self.free_text_frame.set(Some(frame(&image)));
        }
    }
    pub(super) fn focus_free_text(self) {
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if let Some(node) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("canvas-text-body"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = node.focus();
            }
        });
    }
    pub(super) fn free_text_input(self, contents: String) {
        self.free_text.update(|s| {
            if s.phase == Phase::Editing
                && let Some(d) = &mut s.draft
                && !d.body_locked()
            {
                d.contents = contents;
            }
        });
        self.measure_free_text();
    }
    pub(super) fn measure_free_text(self) {
        self.measure_free_text_layout(true);
    }
    fn measure_free_text_layout(self, autosize: bool) {
        if !self.free_text_fonts.get_untracked() {
            return;
        }
        let Some(draft) = self.free_text.with_untracked(|s| {
            (s.phase == Phase::Editing)
                .then(|| s.draft.clone())
                .flatten()
        }) else {
            return;
        };
        let Some(page) = self
            .document
            .with_untracked(|d| d.as_ref()?.info.pages.get(draft.owner.page).cloned())
        else {
            return;
        };
        if let Err(error) = draft.validate(page.width, page.height) {
            self.free_text_error(error);
            return;
        }
        let Some(style) = draft.style.as_ref() else {
            return;
        };
        let autosize = autosize && !draft.geometry_locked();
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(node) = document.create_element("div") else {
            return;
        };
        let Some(body) = document.body() else {
            return;
        };
        let available_width = page.width - draft.bounds.x;
        let top = draft.bounds.y + draft.bounds.height;
        let base = format!(
            "position:fixed;left:-100000px;top:0;visibility:hidden;padding:0;border:0;box-sizing:content-box;font-family:{};font-size:{}px;line-height:{};white-space:pre;",
            font_stack(style.font),
            style.font_size,
            style.line_height
        );
        let _ = node.set_attribute("style", &(base.clone() + "width:max-content;"));
        node.set_text_content(Some(if draft.contents.is_empty() {
            " "
        } else {
            &draft.contents
        }));
        let _ = body.append_child(&node);
        let intrinsic = node.get_bounding_client_rect().width() as f32 + 1.;
        let width = if !autosize || draft.manual_width {
            draft.bounds.width
        } else {
            intrinsic.max(style.font_size * 1.5).min(available_width)
        };
        let _ = node.set_attribute(
            "style",
            &format!("{base}width:{width}px;white-space:pre-wrap;overflow-wrap:anywhere;"),
        );
        // A terminal newline needs a real final empty line in DOM measurement.
        if draft.contents.ends_with('\n') {
            node.set_text_content(Some(&(draft.contents.clone() + "\u{200b}")));
        }
        let height = (node.get_bounding_client_rect().height() as f32)
            .max(style.font_size * style.line_height);
        node.remove();
        let available_height = if autosize { top } else { draft.bounds.height };
        if height > available_height + 0.01 {
            self.free_text_error(
                "正文超出文字框或页面可用空间；正文完整保留，请移动、调整尺寸或删减。".into(),
            );
            return;
        }
        self.free_text.update(|s| {
            if s.phase == Phase::Editing
                && let Some(d) = &mut s.draft
            {
                if autosize {
                    d.bounds.width = width;
                    d.bounds.height = height;
                    d.bounds.y = top - height;
                }
                s.error = None;
            }
        });
    }
    pub(super) fn free_text_composition(self, composing: bool) {
        self.free_text.update(|s| s.composing = composing);
        if !composing {
            let ticket = self.free_text.with_untracked(|s| s.ticket.clone());
            // Some engines deliver the final input immediately after
            // compositionend. Admit that whole event chain before freezing.
            leptos::leptos_dom::helpers::set_timeout(
                move || {
                    if self.free_text.with_untracked(|s| {
                        s.ticket == ticket
                            && s.phase == Phase::Editing
                            && !s.composing
                            && s.finish_requested
                    }) {
                        self.submit_free_text(false);
                    }
                },
                std::time::Duration::from_millis(0),
            );
        }
    }
    pub(super) fn free_text_blur(self) {
        let ticket = self.free_text.with_untracked(|s| s.ticket.clone());
        leptos::leptos_dom::helpers::set_timeout(
            move || {
                if !self
                    .free_text
                    .with_untracked(|s| s.ticket == ticket && s.phase == Phase::Editing)
                {
                    return;
                }
                let within = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.active_element())
                    .and_then(|e| {
                        e.closest(
                            ".canvas-text-editor,.canvas-text-formatting,.canvas-text-recovery",
                        )
                        .ok()
                        .flatten()
                    })
                    .is_some();
                if !within {
                    self.defer_free_text(None);
                }
            },
            std::time::Duration::from_millis(0),
        );
    }
    pub(super) fn submit_free_text(self, retry: bool) {
        let Some(page) = self.document.with_untracked(|d| {
            d.as_ref()?
                .info
                .pages
                .get(self.page.get_untracked())
                .cloned()
        }) else {
            return;
        };
        // Revalidate the actual current rectangle, not an old error flag.
        if !retry {
            self.measure_free_text_layout(false);
        }
        // Layout overflow is a draft error: never send a previous valid body.
        if !retry
            && self
                .free_text
                .with_untracked(|s| s.error.as_ref().is_some_and(|e| e.starts_with("正文超出")))
        {
            self.free_text.update(|s| {
                if let Some(error) = s.error.clone() {
                    s.reject_local(error);
                }
            });
            return;
        }
        let mut result = Ok(None);
        self.free_text.update(|s| {
            result = if retry {
                Ok(s.retry())
            } else {
                s.freeze(page.width, page.height)
            };
        });
        let payload = match result {
            Ok(Some(p)) => p,
            Ok(None) => return,
            Err(error) => {
                self.free_text_error(error);
                return;
            }
        };
        let Some(ticket) = self.free_text.with_untracked(|s| s.ticket.clone()) else {
            return;
        };
        spawn_local(async move {
            let style = payload
                .style
                .as_ref()
                .map(|s| {
                    serde::Serialize::serialize(
                        s,
                        &serde_wasm_bindgen::Serializer::json_compatible(),
                    )
                })
                .transpose();
            let result = match style {
                Ok(style) => {
                    bridge::call_bounded::<Document>(
                        "commit_free_text_editor",
                        &CommitWire {
                            id: payload.id,
                            request: &payload.request,
                            contents: &payload.contents,
                            bounds: payload.bounds,
                            style: style.unwrap_or(JsValue::NULL),
                        },
                    )
                    .await
                }
                Err(error) => Err(format!("文字格式序列化失败：{error}")),
            };
            if !self
                .free_text
                .with_untracked(|s| s.owns(&ticket) && s.phase == Phase::Committing)
            {
                return;
            }
            match result {
                Ok(document)
                    if document.id == ticket.document_id && document.free_text_editor.is_none() =>
                {
                    self.finish_free_text_document(ticket, document)
                }
                Ok(_) => self.free_text.update(|s| {
                    s.uncertain(
                        &ticket,
                        "响应格式错误：文字提交文档身份／所有权不符；只能精确重试或取消／同步。"
                            .into(),
                    )
                }),
                Err(error) => self.free_text.update(|s| {
                    if error.starts_with("原生响应超时；") || error.starts_with("响应格式错误：")
                    {
                        s.uncertain(&ticket, error);
                    } else {
                        s.reject(&ticket, error);
                    }
                }),
            }
        });
    }
    fn finish_free_text_document(self, ticket: Ticket, document: Document) {
        self.free_text.update(|s| s.phase = Phase::Handoff);
        self.free_text_preview.set(Some(Preview {
            ticket: ticket.clone(),
            attempt: self.free_text_attempt.get_untracked(),
            source: String::new(),
            terminal: Some(document),
            installed: false,
        }));
        self.retry_free_text_preview();
    }
    pub(super) fn retry_free_text_preview(self) {
        if self
            .free_text
            .with_untracked(|s| s.phase == Phase::Preparing)
        {
            self.prepare_free_text_background();
            return;
        }
        let Some(preview) = self
            .free_text_preview
            .get_untracked()
            .filter(|p| p.terminal.is_some())
        else {
            return;
        };
        let Some(plan) = self.raster() else {
            return;
        };
        let Some(attempt) = self.next_free_text_attempt() else {
            return;
        };
        spawn_local(async move {
            match bridge::call_bounded::<String>(
                "render_page",
                &json!({"page":preview.ticket.page,"scale":plan.scale}),
            )
            .await
            {
                Ok(source) => {
                    if self
                        .free_text
                        .with_untracked(|s| s.owns(&preview.ticket) && s.phase == Phase::Handoff)
                        && self.free_text_attempt.get_untracked() == attempt
                    {
                        self.stage_free_text(preview.ticket, attempt, source, preview.terminal);
                    }
                }
                Err(error) => {
                    if self.free_text_attempt.get_untracked() == attempt
                        && self.free_text.with_untracked(|s| {
                            s.owns(&preview.ticket) && s.phase == Phase::Handoff
                        })
                    {
                        self.free_text_error(error)
                    }
                }
            }
        });
    }
    pub(super) fn cancel_free_text(self) {
        if self.free_text.with_untracked(|s| {
            s.composing || matches!(s.phase, Phase::Cancelling | Phase::Handoff)
        }) {
            return;
        }
        let Some((ticket, owner)) = self.free_text.with_untracked(|s| {
            Some((s.ticket.clone()?, s.draft.as_ref().map(|d| d.owner.clone())))
        }) else {
            return;
        };
        self.free_text.update(|s| {
            s.phase = Phase::Cancelling;
            s.intent = None;
            s.finish_requested = false;
        });
        self.free_text_pointer.set(None);
        spawn_local(async move {
            let (command, args) = if let Some(owner) = owner {
                (
                    "cancel_free_text_editor",
                    json!({"id":owner.id,"request":owner.request}),
                )
            } else {
                (
                    "cancel_canvas_free_text_request",
                    json!({"request":ticket.request,"document_id":ticket.document_id}),
                )
            };
            let result = bridge::call_bounded::<Document>(command, &args).await;
            if !self
                .free_text
                .with_untracked(|s| s.owns(&ticket) && s.phase == Phase::Cancelling)
            {
                return;
            }
            match result {
                Ok(document)
                    if document.id == ticket.document_id && document.free_text_editor.is_none() =>
                {
                    self.finish_free_text_document(ticket, document)
                }
                Ok(_) => {
                    self.free_text_error(
                        "取消同步未确认原文档所有权结束；请再次取消／同步。".into(),
                    );
                    self.free_text.update(|s| s.phase = Phase::Uncertain);
                }
                Err(error) => {
                    self.free_text_error(error);
                    self.free_text.update(|s| s.phase = Phase::Uncertain);
                }
            }
        });
    }
    pub(super) fn free_text_drag(self, event: &web_sys::PointerEvent, handle: Handle) {
        if !event.is_primary() || event.button() != 0 {
            return;
        }
        let Some(draft) = self.free_text.with_untracked(|s| {
            (s.phase == Phase::Editing && !s.composing)
                .then(|| s.draft.clone())
                .flatten()
        }) else {
            return;
        };
        if draft.geometry_locked() {
            return;
        }
        if draft.style.is_none() && handle != Handle::Move {
            self.free_text_error("调整尺寸前请明确选择打包字体。".into());
            return;
        }
        let Some(image) = annotation_view::decoded_page_image(self) else {
            return;
        };
        let Some(page) = self
            .document
            .with_untracked(|d| d.as_ref()?.info.pages.get(draft.owner.page).cloned())
        else {
            return;
        };
        let frame = frame(&image);
        let Ok(matrix) =
            text::text_matrix(draft.bounds, frame, page.width, page.height, page.rotation)
        else {
            return;
        };
        let Some(node) = event
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        if node.set_pointer_capture(event.pointer_id()).is_err() {
            return;
        }
        self.free_text_pointer.set(Some(Pointer {
            pointer: event.pointer_id(),
            start: Point {
                x: event.client_x() as f32,
                y: event.client_y() as f32,
            },
            frame,
            source: self.image.get_untracked(),
            document_id: self
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.id))
                .unwrap_or(0),
            page: draft.owner.page,
            ticket: self.annotation_image_ticket.get_untracked(),
            moved: false,
            drag: Some((handle, draft.bounds, matrix)),
        }));
        event.prevent_default();
        event.stop_propagation();
    }
    pub(super) fn free_text_format(self, field: &str, value: &str) {
        self.free_text.update(|s| {
            if s.phase != Phase::Editing {
                return;
            }
            let Some(d) = &mut s.draft else {
                return;
            };
            if d.geometry_locked() {
                return;
            }
            // An explicit font choice alone may replace an unknown imported style.
            if d.style.is_none() && field != "font" {
                s.error = Some("请先明确选择打包字体。".into());
                return;
            }
            let mut style = d.style.clone().unwrap_or_default();
            match field {
                "font" => {
                    style.font = match value {
                        "sans" => text::Font::Sans,
                        "cjk_sc" => text::Font::CjkSc,
                        "arabic" => text::Font::Arabic,
                        "devanagari" => text::Font::Devanagari,
                        _ => return,
                    }
                }
                "size" => {
                    let Ok(size) = value.parse::<f32>() else {
                        return;
                    };
                    if !size.is_finite() || !(1. ..=512.).contains(&size) {
                        return;
                    }
                    style.font_size = size;
                }
                "alignment" => {
                    style.alignment = match value {
                        "left" => text::Alignment::Left,
                        "center" => text::Alignment::Center,
                        "right" => text::Alignment::Right,
                        _ => return,
                    }
                }
                "color" => {
                    let Some(hex) = value.strip_prefix('#').filter(|h| h.len() == 6) else {
                        return;
                    };
                    let Ok(rgb) = u32::from_str_radix(hex, 16) else {
                        return;
                    };
                    style.color = text::Color {
                        r: ((rgb >> 16) & 255) as f32 / 255.,
                        g: ((rgb >> 8) & 255) as f32 / 255.,
                        b: (rgb & 255) as f32 / 255.,
                    };
                }
                _ => return,
            }
            d.style = Some(style);
            s.error = None;
        });
        self.measure_free_text();
    }
}
