//! Direct page gestures and exact native-owner/decoded-image coordination.
use super::canvas_intent::Intent;
use super::*;
use pdfmaster_drawing::Point;
use pdfmaster_ui::canvas_notes::{Frame, click_page_point, is_click};
use pdfmaster_ui::canvas_shapes::{
    self as shapes, Draft, Handle, Hit, Kind, Owner, Selection, Style, Target,
};
use pdfmaster_ui::canvas_shapes_state::{Frozen, Phase, Session, Ticket};

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Projection {
    #[serde(flatten)]
    pub list: shapes::List,
    pub hit_rows: Vec<pdfmaster_ui::annotations::Row>,
}
#[derive(Clone, Debug)]
pub(super) struct PageShapes {
    document_id: u32,
    page: usize,
    source: String,
    image_ticket: Option<u64>,
    pub result: Result<Projection, String>,
}
#[derive(Clone, Debug)]
pub(super) struct Pointer {
    id: i32,
    start: Point,
    frame: Frame,
    source: String,
    image_ticket: Option<u64>,
    document_id: u32,
    page: shapes::Page,
    original: Option<(Selection, Handle)>,
    moved: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Preview {
    pub ticket: Ticket,
    pub attempt: u64,
    pub source: String,
    pub installed: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Completion {
    ticket: Ticket,
    document: Document,
    expected: Option<(usize, Draft)>,
    restore: Option<Selection>,
    attempt: u64,
    reading: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Settings {
    pub kind: Kind,
    pub style: Style,
    pub fill: shapes::Color,
    pub fill_enabled: bool,
    pub width: String,
    pub transparency: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            kind: Kind::Rectangle,
            style: Style::default(),
            fill: shapes::FILL_SWATCH,
            fill_enabled: false,
            width: "2".into(),
            transparency: "0".into(),
        }
    }
}
impl Settings {
    fn show_creation_numbers(&mut self) {
        self.width = self.style.line_width.to_string();
        self.transparency = ((1. - self.style.opacity) * 100.).to_string();
    }
}
#[derive(Clone, Debug, Default)]
pub(super) struct Ui {
    pub session: Session<Intent>,
    pub settings: Settings,
    pub list: Option<PageShapes>,
    pub selected: Option<Selection>,
    pub pointer: Option<Pointer>,
    pub frame: Option<Frame>,
    pub preview: Option<Preview>,
    pub completion: Option<Completion>,
    pub attempt: u64,
    pub projection_attempt: u64,
    pub mutation_attempt: u64,
    pub sync: u64,
    pub syncing: bool,
    pub base_count: Option<usize>,
    pub deleting: bool,
    pub error: Option<String>,
    pub local_error: Option<String>,
    pub cancelling: bool,
}
// Preserve *all* nested nullable fields at the JS boundary, locally only.
#[derive(serde::Serialize)]
#[serde(transparent)]
struct Wire(#[serde(with = "serde_wasm_bindgen::preserve")] wasm_bindgen::JsValue);
async fn call<T: serde::de::DeserializeOwned>(command: &str, args: Value) -> Result<T, String> {
    let value = js_sys::JSON::parse(&args.to_string()).map_err(|_| "形状参数无法序列化")?;
    bridge::call_bounded(command, &Wire(value)).await
}
fn request() -> Option<String> {
    let mut b = [0u8; 16];
    web_sys::window()?
        .crypto()
        .ok()?
        .get_random_values_with_u8_array(&mut b)
        .ok()?;
    Some(b.iter().map(|v| format!("{v:02x}")).collect())
}
fn image_frame(image: &web_sys::HtmlImageElement) -> Frame {
    let r = image.get_bounding_client_rect();
    Frame {
        left: r.left() as f32,
        top: r.top() as f32,
        width: r.width() as f32,
        height: r.height() as f32,
    }
}
pub(super) fn color(c: shapes::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.r * 255.).round() as u8,
        (c.g * 255.).round() as u8,
        (c.b * 255.).round() as u8
    )
}
fn parse_color(value: &str) -> Result<shapes::Color, String> {
    let v = value
        .strip_prefix('#')
        .filter(|v| v.len() == 6)
        .and_then(|v| u32::from_str_radix(v, 16).ok())
        .ok_or("颜色必须为 #RRGGBB")?;
    Ok(shapes::Color {
        r: ((v >> 16) & 255) as f32 / 255.,
        g: ((v >> 8) & 255) as f32 / 255.,
        b: (v & 255) as f32 / 255.,
    })
}
impl Model {
    pub(super) fn shape_page(self) -> Option<shapes::Page> {
        let index = self.page.get_untracked();
        self.document.with_untracked(|d| {
            d.as_ref()?.info.pages.get(index).map(|p| shapes::Page {
                index,
                width: p.width,
                height: p.height,
                rotation: p.rotation,
            })
        })
    }
    pub(super) fn has_shape_draft(self) -> bool {
        self.shape_ui
            .with(|u| u.session.active() || u.pointer.is_some() || u.completion.is_some())
            || self
                .document
                .with(|d| d.as_ref().is_some_and(|d| d.shape_editor.is_some()))
    }
    pub(super) fn shape_error(self, error: String) {
        self.shape_ui.update(|u| {
            u.error = Some(error.clone());
            u.session.error = Some(error.clone());
        });
        self.message.set(error);
    }
    fn shape_owns(self, t: &Ticket) -> bool {
        self.shape_ui.with_untracked(|u| u.session.owns(t))
            && self.route.get_untracked() == Route::Document
            && self.page.get_untracked() == t.page
            && self
                .document
                .with_untracked(|d| d.as_ref().is_some_and(|d| d.id == t.document_id))
    }
    pub(super) fn measure_shape_frame(self) {
        let frame = annotation_view::decoded_page_image(self).map(|i| image_frame(&i));
        self.shape_ui.update(|u| u.frame = frame);
    }
    pub(super) async fn refresh_canvas_shapes(self) -> Result<(), String> {
        let id = self
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.id))
            .ok_or("请先打开 PDF")?;
        let page = self.page.get_untracked();
        let source = self.image.get_untracked();
        let image_ticket = self.annotation_image_ticket.get_untracked();
        let mut attempt = 0;
        self.shape_ui.update(|u| {
            u.projection_attempt += 1;
            attempt = u.projection_attempt;
        });
        let result = call::<Projection>(
            "canvas_shapes_document",
            json!({"document_id":id,"page":page}),
        )
        .await
        .and_then(|p| {
            if p.hit_rows.len() != p.list.annotation_count
                || p.hit_rows
                    .iter()
                    .enumerate()
                    .any(|(i, r)| r.page_index != page || r.annotation_index != i)
                || p.list.annotations.len() + p.list.unsupported_count != p.list.annotation_count
            {
                return Err("形状命中投影不完整".into());
            }
            Ok(p)
        });
        // Superseded reads are inert, including their callers' error display.
        // No stale producer may replace a successor cache before its coordinator
        // checks completion ownership.
        if self
            .shape_ui
            .with_untracked(|u| u.projection_attempt != attempt)
            || self.document.with_untracked(|d| d.as_ref().map(|d| d.id)) != Some(id)
            || self.page.get_untracked() != page
            || self.image.get_untracked() != source
            || self.annotation_image_ticket.get_untracked() != image_ticket
        {
            return Ok(());
        }
        let status = result.as_ref().map(|_| ()).map_err(Clone::clone);
        self.shape_ui.update(|u| {
            u.list = Some(PageShapes {
                document_id: id,
                page,
                source,
                image_ticket,
                result,
            })
        });
        status
    }
    pub(super) fn select_shape_kind(self, kind: Kind) {
        if self.has_shape_draft() {
            self.shape_error("请先结束当前形状手势".into());
            return;
        }
        self.shape_ui.update(|u| {
            u.selected = None;
            u.settings.show_creation_numbers();
            u.settings.kind = kind;
            u.settings.style.fill = if kind.supports_fill() && u.settings.fill_enabled {
                Some(u.settings.fill)
            } else {
                None
            };
            u.error = None;
        });
    }
    fn passive_shape(self, selection: Option<Selection>) {
        self.shape_ui.update(|u| {
            if let Some(s) = &selection
                && let Ok(d) = s.draft()
            {
                u.settings.width = d.line_width.to_string();
                u.settings.transparency = ((1. - d.opacity) * 100.).to_string();
            } else {
                u.settings.show_creation_numbers();
            }
            u.selected = selection;
            u.error = None;
            u.local_error = None;
        });
    }
    pub(super) fn shape_style(self) -> Style {
        self.shape_ui.with(|u| {
            if let Some(d) = &u.session.candidate {
                return Style {
                    stroke: d.stroke,
                    fill: d.fill,
                    line_width: d.line_width,
                    opacity: d.opacity,
                };
            }
            if let Some(s) = &u.selected {
                return Style {
                    stroke: s.original.stroke,
                    fill: s.original.fill,
                    line_width: s.original.line_width,
                    opacity: s.opacity,
                };
            }
            u.settings.style
        })
    }
    pub(super) fn shape_style_disabled(self) -> bool {
        self.busy.get()
            || self.has_free_text_draft()
            || self.shape_ui.with(|u| {
                matches!(
                    u.session.phase,
                    Phase::Committing | Phase::Uncertain | Phase::Cancelling | Phase::Handoff
                ) || u.completion.is_some()
            })
    }
    pub(super) fn shape_number(self, field: &str, value: String, finish: bool) {
        self.shape_ui.update(|u| {
            if field == "width" {
                u.settings.width = value.clone();
            } else {
                u.settings.transparency = value.clone();
            }
        });
        if finish {
            self.shape_format(field, &value, true);
        }
    }
    pub(super) fn shape_format(self, field: &str, value: &str, finish: bool) {
        if self.shape_style_disabled() {
            return;
        }
        let mut style = self.shape_style();
        let parsed = (|| -> Result<(), String> {
            match field {
                "width" => {
                    style.line_width = value.parse().map_err(|_| "线宽须为 0.25–32")?;
                    if !style.line_width.is_finite() || !(0.25..=32.).contains(&style.line_width) {
                        return Err("线宽须为 0.25–32".into());
                    }
                }
                "transparency" => {
                    style.opacity =
                        shapes::transparency(value.parse().map_err(|_| "透明度须为 0–100")?)?;
                }
                "stroke" => style.stroke = parse_color(value)?,
                "fill" => style.fill = Some(parse_color(value)?),
                "enabled" => {
                    style.fill = if value == "true" {
                        Some(self.shape_ui.with_untracked(|u| u.settings.fill))
                    } else {
                        None
                    }
                }
                _ => return Err("未知形状样式".into()),
            }
            Ok(())
        })();
        if let Err(e) = parsed {
            self.shape_ui.update(|u| u.local_error = Some(e.clone()));
            self.shape_error(e);
            return;
        }
        self.shape_ui.update(|u| {
            if u.local_error.take().is_some() {
                u.error = None;
                u.session.error = None;
            }
            if field == "width" {
                u.settings.width = value.into();
            }
            if field == "transparency" {
                u.settings.transparency = value.into();
            }
            if field == "fill" {
                u.settings.fill = style.fill.unwrap_or(u.settings.fill);
            }
            if field == "enabled" {
                u.settings.fill_enabled = style.fill.is_some();
            }
        });
        let selected = self.shape_ui.with_untracked(|u| u.selected.clone());
        if !self.shape_ui.with_untracked(|u| u.session.active()) {
            if let Some(s) = selected {
                if s.flags & (64 | 128 | 512) != 0 {
                    self.shape_error("所选形状已锁定".into());
                    return;
                }
                let Ok(d) = s.draft() else {
                    return;
                };
                if d.stroke == style.stroke
                    && d.fill == style.fill
                    && d.line_width == style.line_width
                    && d.opacity == style.opacity
                {
                    return;
                }
                self.begin_shape(Target::Existing { original: s });
            } else {
                self.shape_ui.update(|u| {
                    u.settings.style = style;
                    u.settings.style.fill = if u.settings.kind.supports_fill() {
                        style.fill
                    } else {
                        None
                    };
                });
                return;
            }
        }
        self.shape_ui.update(|u| {
            if let Some(d) = &mut u.session.candidate {
                d.stroke = style.stroke;
                d.fill = style.fill;
                d.line_width = style.line_width;
                d.opacity = style.opacity;
            }
        });
        if finish {
            self.defer_shape_edit(None);
        }
    }
    pub(super) fn shape_pointer_down(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Shapes) {
            return false;
        }
        if event.button() != 0
            || !event.is_primary()
            || self.has_free_text_draft()
            || self.busy.get_untracked()
            || self
                .shape_ui
                .with_untracked(|u| u.session.active() || u.completion.is_some())
        {
            return true;
        }
        let Some(image) = annotation_view::decoded_page_image(self) else {
            self.shape_error("等待当前页面解码".into());
            return true;
        };
        let Some(page) = self.shape_page() else {
            return true;
        };
        let frame = image_frame(&image);
        let point = Point {
            x: event.client_x() as f32,
            y: event.client_y() as f32,
        };
        let id = self
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.id))
            .unwrap_or(0);
        let source = self.image.get_untracked();
        let image_ticket = self.annotation_image_ticket.get_untracked();
        let hit = self.shape_ui.with_untracked(|u| {
            let b = u.list.as_ref().ok_or("等待完整形状投影")?;
            if b.document_id != id
                || b.page != page.index
                || b.source != source
                || b.image_ticket != image_ticket
            {
                return Err("形状投影已过期".to_string());
            }
            let p = b.result.as_ref().map_err(Clone::clone)?;
            shapes::hit_test(
                &p.list,
                &p.hit_rows,
                u.selected.as_ref(),
                point,
                frame,
                page,
            )
        });
        let original = match hit {
            Ok(Some(Hit::Existing(s))) => {
                self.passive_shape(Some(s.clone()));
                Some((s, Handle::Move))
            }
            Ok(Some(Hit::Handle(h))) => self
                .shape_ui
                .with_untracked(|u| u.selected.clone().map(|s| (s, h))),
            Ok(None) => {
                self.passive_shape(None);
                None
            }
            Err(e) => {
                self.shape_error(e);
                return true;
            }
        };
        let Some(node) = surface.get_untracked() else {
            return true;
        };
        // Page focus scopes Escape/Delete without scrolling the gesture plane.
        if let Ok(focus) =
            js_sys::Reflect::get(node.as_ref(), &wasm_bindgen::JsValue::from_str("focus"))
            && let Ok(focus) = focus.dyn_into::<js_sys::Function>()
            && let Ok(options) = js_sys::JSON::parse("{\"preventScroll\":true}")
        {
            let _ = focus.call1(node.as_ref(), &options);
        }
        if node.set_pointer_capture(event.pointer_id()).is_err() {
            self.shape_error("无法捕获形状手势".into());
            return true;
        }
        self.shape_ui.update(|u| {
            u.pointer = Some(Pointer {
                id: event.pointer_id(),
                start: point,
                frame,
                source,
                image_ticket,
                document_id: id,
                page,
                original,
                moved: false,
            });
            u.frame = Some(frame);
            u.error = None;
        });
        event.prevent_default();
        true
    }
    pub(super) fn shape_pointer_move(self, event: &web_sys::PointerEvent) -> bool {
        let Some(p) = self
            .shape_ui
            .with_untracked(|u| u.pointer.clone())
            .filter(|p| p.id == event.pointer_id())
        else {
            return false;
        };
        let current = Point {
            x: event.client_x() as f32,
            y: event.client_y() as f32,
        };
        let coherent = self.page.get_untracked() == p.page.index
            && self.document.with_untracked(|d| d.as_ref().map(|d| d.id)) == Some(p.document_id)
            && self.annotation_image_ticket.get_untracked() == p.image_ticket
            && annotation_view::decoded_page_image(self)
                .is_none_or(|i| !image_frame(&i).changed_from(p.frame))
            && (self.image.get_untracked() == p.source
                || self.shape_ui.with_untracked(|u| {
                    u.preview
                        .as_ref()
                        .is_some_and(|v| v.installed && self.image.get_untracked() == v.source)
                }));
        if !coherent {
            self.cancel_canvas_shape();
            return true;
        }
        if !p.moved && is_click(p.start, current) {
            return true;
        }
        let mapped = |q| {
            click_page_point(q, p.frame, p.page.width, p.page.height, p.page.rotation)
                .map_err(String::from)
        };
        let candidate = (|| {
            let start = mapped(p.start)?;
            let end = mapped(current)?;
            if let Some((s, h)) = &p.original {
                let d = s.draft()?;
                let bounds = shapes::drag_bounds(d.bounds(p.page)?, start, end, *h, p.page)?;
                shapes::transform(&d, bounds, p.page)
            } else {
                let (kind, style) = self
                    .shape_ui
                    .with_untracked(|u| (u.settings.kind, u.settings.style));
                shapes::new_draft(kind, start, end, style, p.page)
            }
        })();
        match candidate {
            Ok(d) => {
                if !self.shape_ui.with_untracked(|u| u.session.active()) {
                    let target = if let Some((s, _)) = &p.original {
                        Target::Existing {
                            original: s.clone(),
                        }
                    } else {
                        let revision = self.shape_ui.with_untracked(|u| {
                            u.list
                                .as_ref()
                                .and_then(|p| p.result.as_ref().ok())
                                .map(|p| p.list.revision)
                        });
                        let Some(revision) = revision else {
                            return true;
                        };
                        Target::New {
                            draft: d.clone(),
                            revision,
                        }
                    };
                    self.begin_shape(target);
                }
                self.shape_ui.update(|u| {
                    if let Some(t) = u.session.ticket.clone() {
                        u.session.set_candidate(&t, d);
                    }
                    if let Some(p) = &mut u.pointer {
                        p.moved = true;
                    }
                    if u.local_error.take().is_some() {
                        u.error = None;
                        u.session.error = None;
                    }
                });
            }
            Err(e) => {
                self.shape_ui.update(|u| u.local_error = Some(e.clone()));
                self.shape_error(e);
            }
        }
        true
    }
    pub(super) fn shape_pointer_up(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        if !self.shape_ui.with_untracked(|u| {
            u.pointer
                .as_ref()
                .is_some_and(|p| p.id == event.pointer_id())
        }) {
            return false;
        }
        self.shape_pointer_move(event);
        let moved = self
            .shape_ui
            .with_untracked(|u| u.pointer.as_ref().is_some_and(|p| p.moved));
        self.shape_ui.update(|u| u.pointer = None);
        if let Some(node) = surface.get_untracked() {
            let _ = node.release_pointer_capture(event.pointer_id());
        }
        if moved {
            self.defer_shape_edit(None);
        }
        true
    }
    pub(super) fn shape_pointer_cancel(self, id: i32) -> bool {
        if !self
            .shape_ui
            .with_untracked(|u| u.pointer.as_ref().is_some_and(|p| p.id == id))
        {
            return false;
        }
        self.shape_ui.update(|u| u.pointer = None);
        if self.shape_ui.with_untracked(|u| u.session.active()) {
            self.cancel_canvas_shape();
        }
        true
    }
    fn begin_shape(self, target: Target) {
        let Some(page) = self.shape_page() else {
            return;
        };
        let Some(request) = request() else {
            self.shape_error("无法生成安全形状请求".into());
            return;
        };
        let id = self
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.id))
            .unwrap_or(0);
        let mut ticket = None;
        self.shape_ui.update(|u| {
            u.base_count = u
                .list
                .as_ref()
                .and_then(|p| p.result.as_ref().ok())
                .map(|p| p.list.annotation_count);
            u.sync += 1;
            u.syncing = false;
            ticket = u
                .session
                .begin(id, page.index, request.clone(), target.clone());
            u.error = None;
        });
        let Some(t) = ticket else {
            return;
        };
        spawn_local(async move {
            let result = call::<Owner>(
                "begin_canvas_shape_editor",
                json!({"request":request,"document_id":id,"page":page.index,"target":target}),
            )
            .await;
            if !self.shape_owns(&t)
                || !self
                    .shape_ui
                    .with_untracked(|u| u.session.phase == Phase::BeginPending)
            {
                return;
            }
            if annotation_view::decoded_page_image(self).is_none() {
                self.shape_error("形状手势源图片已切换；正在取消原始请求".into());
                self.cancel_canvas_shape();
                return;
            }
            match result {
                Ok(owner) => {
                    let mut accepted = false;
                    self.shape_ui
                        .update(|u| accepted = u.session.accept(&t, owner));
                    if accepted {
                        self.retry_shape_background();
                    } else {
                        self.shape_error("原生形状所有者与完整目标不匹配；请取消／同步".into());
                    }
                }
                Err(e) => self.shape_error(e),
            }
        });
    }
    pub(super) fn defer_shape_edit(self, intent: Option<Intent>) -> bool {
        if !self.has_shape_draft() {
            return false;
        }
        self.shape_ui.update(|u| u.session.queue(intent));
        self.submit_canvas_shape(false);
        true
    }
    pub(super) fn submit_canvas_shape(self, retry: bool) {
        let Some(page) = self.shape_page() else {
            return;
        };
        let mut frozen = None;
        self.shape_ui.update(|u| {
            if let Some(e) = u.local_error.clone() {
                u.session.reject_local(e);
                return;
            }
            if retry {
                frozen = u.session.retry();
            } else if u.session.finish_requested && u.session.phase == Phase::Editing {
                if u.base_count.is_none()
                    && matches!(
                        u.session.owner.as_ref().map(|o| &o.target),
                        Some(Target::New { .. })
                    )
                {
                    u.error = Some("等待恢复形状完整投影".into());
                    return;
                }
                if u.deleting {
                    if let Ok(f) = u.session.freeze_delete() {
                        frozen = f;
                    }
                } else {
                    match u.session.freeze(page) {
                        Ok(Some(p)) => frozen = Some(Frozen::Commit(p)),
                        Err(e) => u.error = Some(e),
                        _ => {}
                    }
                }
            }
        });
        if let Some(frozen) = frozen {
            self.perform_shape_action(frozen);
        }
    }
    fn perform_shape_action(self, frozen: Frozen) {
        let Some(t) = self.shape_ui.with_untracked(|u| u.session.ticket.clone()) else {
            return;
        };
        let mut attempt = 0;
        self.shape_ui.update(|u| {
            u.mutation_attempt += 1;
            attempt = u.mutation_attempt;
        });
        spawn_local(async move {
            let result: Result<Option<Document>, String> = match &frozen {
                Frozen::Commit(p) => call::<Document>(
                    "commit_shape_editor",
                    serde_json::to_value(p).unwrap_or(Value::Null),
                )
                .await
                .map(Some),
                Frozen::Delete { id, request } => {
                    call("delete_shape_editor", json!({"id":id,"request":request})).await
                }
            };
            if !self.shape_owns(&t)
                || !self.shape_ui.with_untracked(|u| {
                    u.session.phase == Phase::Committing
                        && u.mutation_attempt == attempt
                        && u.session.frozen.as_ref() == Some(&frozen)
                })
            {
                return;
            }
            match result {
                Ok(Some(doc)) => self.shape_terminal(t, doc, false),
                Ok(None) => {
                    self.shape_ui.update(|u| {
                        u.session.delete_cancelled(&t);
                        u.deleting = false;
                    });
                }
                Err(e) => {
                    self.shape_ui.update(|u| u.session.uncertain(&t, e.clone()));
                    self.shape_error(e.clone());
                    if !e.contains("超时")
                        && !e.contains("格式")
                        && let Ok(doc) = call::<Document>("document_info", json!({})).await
                        && self.shape_owns(&t)
                        && self.shape_ui.with_untracked(|u| {
                            u.mutation_attempt == attempt
                                && u.session.phase == Phase::Uncertain
                                && u.session.frozen.as_ref() == Some(&frozen)
                        })
                        && doc.id == t.document_id
                        && doc.free_text_editor.is_none()
                        && doc.shape_editor.as_ref().is_some_and(|o| {
                            self.shape_ui
                                .with_untracked(|u| u.session.matches_owner(doc.id, o))
                        })
                    {
                        self.shape_ui.update(|u| {
                            u.session.reject(&t, e);
                            u.deleting = false;
                        });
                    }
                }
            }
        });
    }
    pub(super) fn delete_canvas_shape(self) {
        if self.shape_style_disabled() {
            return;
        }
        if !self.shape_ui.with_untracked(|u| u.session.active()) {
            let Some(s) = self.shape_ui.with_untracked(|u| u.selected.clone()) else {
                return;
            };
            self.begin_shape(Target::Existing { original: s });
        }
        self.shape_ui.update(|u| u.deleting = true);
        self.defer_shape_edit(None);
    }
    pub(super) fn cancel_canvas_shape(self) {
        let Some(t) = self.shape_ui.with_untracked(|u| u.session.ticket.clone()) else {
            self.shape_ui.update(|u| u.pointer = None);
            return;
        };
        let phase = self.shape_ui.with_untracked(|u| u.session.phase);
        if matches!(phase, Phase::Committing | Phase::Uncertain) {
            self.sync_canvas_shape();
            return;
        }
        if phase == Phase::Handoff || self.shape_ui.with_untracked(|u| u.cancelling) {
            return;
        }
        let owner = self.shape_ui.with_untracked(|u| u.session.owner.clone());
        self.shape_ui.update(|u| {
            u.session.phase = Phase::Cancelling;
            u.cancelling = true;
            u.session.intent = None;
            u.pointer = None;
            u.preview = None;
            u.attempt += 1;
            u.sync += 1;
            u.syncing = false;
        });
        spawn_local(async move {
            let result = if let Some(o) = owner {
                call::<Document>(
                    "cancel_shape_editor",
                    json!({"id":o.id,"request":o.request}),
                )
                .await
            } else {
                call::<Document>(
                    "cancel_canvas_shape_request",
                    json!({"document_id":t.document_id,"request":t.request}),
                )
                .await
            };
            if !self.shape_owns(&t)
                || self.shape_ui.with_untracked(|u| u.session.phase) != Phase::Cancelling
            {
                return;
            }
            self.shape_ui.update(|u| u.cancelling = false);
            match result {
                Ok(doc) => self.shape_terminal(t, doc, true),
                Err(e) => self.shape_error(e),
            }
        });
    }
    fn shape_terminal(self, t: Ticket, doc: Document, cancelled: bool) {
        if !self.shape_owns(&t)
            || doc.id != t.document_id
            || doc.shape_editor.is_some()
            || doc.free_text_editor.is_some()
        {
            self.shape_ui.update(|u| {
                u.session
                    .uncertain(&t, "形状终态无法确认；仅精确重试".into())
            });
            self.shape_error("形状终态无法确认；保留恢复状态".into());
            return;
        }
        self.shape_ui.update(|u| {
            let expected = if cancelled {
                None
            } else {
                match &u.session.frozen {
                    Some(Frozen::Commit(p)) => Some((
                        match u.session.owner.as_ref().map(|o| &o.target) {
                            Some(Target::Existing { original }) => {
                                original.original.annotation_index
                            }
                            _ => u
                                .base_count
                                .expect("New submission requires bound append index"),
                        },
                        p.draft.clone(),
                    )),
                    _ => None,
                }
            };
            let restore = if cancelled { u.selected.clone() } else { None };
            u.session.phase = Phase::Handoff;
            u.sync += 1;
            u.syncing = false;
            u.pointer = None;
            u.preview = None;
            u.completion = Some(Completion {
                ticket: t,
                document: doc,
                expected,
                restore,
                attempt: 0,
                reading: false,
            });
            u.error = None;
        });
        self.retry_shape_background();
    }
    pub(super) fn retry_shape_background(self) {
        let Some(t) = self.shape_ui.with_untracked(|u| u.session.ticket.clone()) else {
            return;
        };
        let phase = self.shape_ui.with_untracked(|u| u.session.phase);
        if !matches!(phase, Phase::Preparing | Phase::Handoff) {
            return;
        }
        let Some(plan) = self.raster() else {
            return;
        };
        let owner = self.shape_ui.with_untracked(|u| u.session.owner.clone());
        let mut attempt = 0;
        self.shape_ui.update(|u| {
            u.attempt += 1;
            attempt = u.attempt;
            u.preview = None;
            u.error = None;
            u.session.error = None;
            if let Some(c) = &mut u.completion {
                c.attempt = attempt;
                c.reading = false;
            }
        });
        spawn_local(async move {
            // A recovered New owner has no frontend pre-list. Obtain its exact
            // append slot while still Preparing, before any submission is possible.
            if phase == Phase::Preparing
                && self.shape_ui.with_untracked(|u| u.base_count.is_none())
                && let Some(o) = &owner
                && let Target::New { revision, .. } = &o.target
            {
                let source = self.image.get_untracked();
                let image_ticket = self.annotation_image_ticket.get_untracked();
                let result = self.refresh_canvas_shapes().await;
                let proof = call::<Document>("document_info", json!({})).await;
                if !self.shape_owns(&t)
                    || self.shape_ui.with_untracked(|u| {
                        u.attempt != attempt || u.session.phase != Phase::Preparing
                    })
                    || self.image.get_untracked() != source
                    || self.annotation_image_ticket.get_untracked() != image_ticket
                {
                    return;
                }
                let count = result.and_then(|()| {
                    let doc = proof?;
                    if doc.id != t.document_id
                        || doc.free_text_editor.is_some()
                        || doc.shape_editor.as_ref() != Some(o)
                        || !self
                            .shape_ui
                            .with_untracked(|u| u.session.matches_owner(doc.id, o))
                    {
                        return Err("恢复形状所有者已变化".into());
                    }
                    self.shape_ui.with_untracked(|u| {
                        let p = u
                            .list
                            .as_ref()
                            .filter(|p| {
                                p.document_id == t.document_id
                                    && p.page == t.page
                                    && p.source == source
                                    && p.image_ticket == image_ticket
                            })
                            .and_then(|p| p.result.as_ref().ok())
                            .ok_or("恢复形状完整投影缺失")?;
                        if &p.list.revision != revision {
                            return Err("恢复形状投影版本不匹配".into());
                        }
                        Ok(p.list.annotation_count)
                    })
                });
                match count {
                    Ok(count) => self.shape_ui.update(|u| u.base_count = Some(count)),
                    Err(e) => {
                        self.shape_error(e);
                        return;
                    }
                }
            }
            let result = if phase == Phase::Handoff {
                call::<String>("render_page", json!({"page":t.page,"scale":plan.scale})).await
            } else if let Some(o) = owner {
                call::<String>(
                    "render_shape_editor_background",
                    json!({"id":o.id,"request":o.request,"scale":plan.scale}),
                )
                .await
            } else {
                Err("形状所有者不存在".into())
            };
            if !self.shape_owns(&t)
                || self
                    .shape_ui
                    .with_untracked(|u| u.attempt != attempt || u.session.phase != phase)
            {
                return;
            }
            match result {
                Ok(source) if source.starts_with("data:image/png;base64,") => {
                    self.shape_ui.update(|u| {
                        u.preview = Some(Preview {
                            ticket: t.clone(),
                            attempt,
                            source: source.clone(),
                            installed: false,
                        })
                    });
                    let _ = set_timeout_with_handle(
                        move || {
                            if self.shape_owns(&t)
                                && self.shape_ui.with_untracked(|u| {
                                    u.attempt == attempt
                                        && matches!(
                                            u.session.phase,
                                            Phase::Preparing | Phase::Handoff
                                        )
                                        && u.preview.as_ref().is_some_and(|p| p.source == source)
                                })
                            {
                                self.shape_error("形状页面解码超时；可重试当前背景".into());
                            }
                        },
                        std::time::Duration::from_secs(15),
                    );
                }
                Ok(_) => self.shape_error("形状背景不是 PNG".into()),
                Err(e) => self.shape_error(e),
            }
        });
    }
    pub(super) fn shape_stage_decoded(self, t: &Ticket, attempt: u64, source: &str) {
        if !self.shape_owns(t)
            || !self.shape_ui.with_untracked(|u| {
                u.attempt == attempt
                    && matches!(u.session.phase, Phase::Preparing | Phase::Handoff)
                    && u.preview.as_ref().is_some_and(|p| {
                        p.ticket == *t && p.attempt == attempt && p.source == source && !p.installed
                    })
            })
        {
            return;
        }
        self.shape_ui.update(|u| {
            if let Some(p) = &mut u.preview {
                p.installed = true;
            }
        });
        self.image_ready.set(false);
        self.image.set(source.into());
        leptos::leptos_dom::helpers::request_animation_frame(move || self.shape_image_loaded());
    }
    pub(super) fn shape_stage_failed(self, t: &Ticket, attempt: u64, source: &str) {
        if self.shape_owns(t)
            && self.shape_ui.with_untracked(|u| {
                u.attempt == attempt
                    && matches!(u.session.phase, Phase::Preparing | Phase::Handoff)
                    && u.preview.as_ref().is_some_and(|p| {
                        p.ticket == *t && p.attempt == attempt && p.source == source
                    })
            })
        {
            self.shape_error("形状背景解码失败；草稿保留，可重试背景".into());
        }
    }
    pub(super) fn shape_image_loaded(self) {
        self.measure_shape_frame();
        let Some(preview) = self
            .shape_ui
            .with_untracked(|u| u.preview.clone())
            .filter(|p| {
                p.installed
                    && p.source == self.image.get_untracked()
                    && p.attempt == self.shape_ui.with_untracked(|u| u.attempt)
            })
        else {
            return;
        };
        if !self.shape_owns(&preview.ticket) || annotation_view::decoded_page_image(self).is_none()
        {
            return;
        }
        self.image_ready.set(true);
        if self
            .shape_ui
            .with_untracked(|u| u.session.phase == Phase::Preparing)
        {
            self.shape_ui.update(|u| {
                u.session.ready(&preview.ticket);
                u.error = None;
            });
            self.submit_canvas_shape(false);
        } else if self
            .shape_ui
            .with_untracked(|u| u.session.phase == Phase::Handoff)
        {
            self.complete_canvas_shape();
        }
    }
    pub(super) fn shape_primary_event_current(self, attempt: u64) -> bool {
        self.shape_ui
            .with_untracked(|u| !u.session.active() || u.attempt == attempt)
    }
    pub(super) fn shape_image_failed(self, owner: Option<u64>, source: &str) {
        if self.annotation_image_ticket.get_untracked() != owner
            || self.image.with_untracked(|current| current != source)
        {
            return;
        }
        let Some(preview) = self.shape_ui.with_untracked(|u| {
            u.preview.clone().filter(|p| {
                p.installed
                    && p.source == source
                    && p.attempt == u.attempt
                    && matches!(u.session.phase, Phase::Preparing | Phase::Handoff)
            })
        }) else {
            return;
        };
        if !self.shape_owns(&preview.ticket) {
            return;
        }
        self.shape_stage_failed(&preview.ticket, preview.attempt, source);
        // Invalidate every read/load/timer from the failed decoder. Only a new
        // explicit background attempt may resume this read-only handoff.
        self.shape_ui.update(|u| {
            u.attempt += 1;
            u.projection_attempt += 1;
            u.preview = None;
            if let Some(c) = &mut u.completion {
                c.attempt = u.attempt;
                c.reading = false;
            }
        });
    }
    pub(super) fn complete_canvas_shape(self) {
        let Some(c) = self
            .shape_ui
            .with_untracked(|u| u.completion.clone())
            .filter(|c| !c.reading)
        else {
            return;
        };
        if !self.shape_owns(&c.ticket)
            || self.shape_ui.with_untracked(|u| u.attempt != c.attempt)
            || annotation_view::decoded_page_image(self).is_none()
        {
            return;
        }
        self.shape_ui.update(|u| {
            if let Some(c) = &mut u.completion {
                c.reading = true;
            }
            u.error = None;
        });
        let source = self.image.get_untracked();
        let image_ticket = self.annotation_image_ticket.get_untracked();
        spawn_local(async move {
            let result = self.refresh_canvas_shapes().await;
            if !self.shape_owns(&c.ticket)
                || self.image.get_untracked() != source
                || self.annotation_image_ticket.get_untracked() != image_ticket
                || self.shape_ui.with_untracked(|u| u.attempt != c.attempt)
            {
                return;
            }
            let selected = result.and_then(|()| {
                self.shape_ui.with_untracked(|u| {
                    let p = u
                        .list
                        .as_ref()
                        .and_then(|p| p.result.as_ref().ok())
                        .ok_or("形状完成投影缺失")?;
                    if let Some((index, d)) = &c.expected {
                        shapes::reconcile(
                            &p.list,
                            *index,
                            d,
                            self.shape_page().ok_or("形状页面不存在")?,
                        )
                        .map(Some)
                    } else {
                        Ok(c.restore.as_ref().and_then(|s| {
                            p.list
                                .annotations
                                .iter()
                                .find(|v| {
                                    v.page_id == s.page_id
                                        && v.object_id == s.object_id
                                        && v.original.annotation_index
                                            == s.original.annotation_index
                                })
                                .cloned()
                        }))
                    }
                })
            });
            match selected {
                Ok(selected) => {
                    let mut intent = None;
                    self.shape_ui.update(|u| {
                        intent = u.session.finish(&c.ticket, c.document.id, true);
                        u.completion = None;
                        u.preview = None;
                        u.deleting = false;
                        u.error = None;
                    });
                    self.replace_document_view(Some(c.document));
                    self.passive_shape(selected);
                    self.message
                        .set("形状编辑已完成；页面与完整投影已刷新".into());
                    if let Some(intent) = intent {
                        self.execute_canvas_intent(intent);
                    }
                }
                Err(e) => {
                    self.shape_ui.update(|u| {
                        if let Some(c) = &mut u.completion {
                            c.reading = false;
                        }
                    });
                    self.shape_error(format!("形状已结束；仅重试完成投影：{e}"));
                }
            }
        });
    }
    pub(super) fn recover_canvas_shape(self, doc: &Document) -> bool {
        let Some(owner) = doc.shape_editor.clone() else {
            return false;
        };
        if doc.free_text_editor.is_some() {
            self.shape_error("文字与形状原生所有者冲突；禁止编辑".into());
            return true;
        }
        if !self.shape_ui.with_untracked(|u| u.session.active()) {
            self.shape_ui.update(|u| {
                u.base_count = None;
                u.sync += 1;
                u.syncing = false;
                if let Some(t) = u.session.begin(
                    doc.id,
                    owner.page,
                    owner.request.clone(),
                    owner.target.clone(),
                ) {
                    u.session.accept(&t, owner.clone());
                }
                u.error = Some("恢复原生形状目标；未提交指针／样式输入没有持久化".into());
            });
        }
        self.page.set(owner.page);
        self.chrome.update(|c| {
            c.mode = Some(Mode::Annotation);
            c.active_tool = Some(ActiveTool::Shapes);
            c.panel = None;
        });
        self.overview_visible.set(false);
        true
    }
    pub(super) fn resume_canvas_shape(self) {
        if self
            .shape_ui
            .with_untracked(|u| u.session.phase == Phase::Preparing)
        {
            self.retry_shape_background();
        }
    }
    pub(super) fn sync_canvas_shape(self) {
        let Some(t) = self.shape_ui.with_untracked(|u| u.session.ticket.clone()) else {
            return;
        };
        if self.shape_ui.with_untracked(|u| u.syncing) {
            return;
        }
        let mut attempt = 0;
        self.shape_ui.update(|u| {
            u.sync += 1;
            attempt = u.sync;
            u.syncing = true;
        });
        spawn_local(async move {
            let result = call::<Document>("document_info", json!({})).await;
            if !self.shape_owns(&t) || self.shape_ui.with_untracked(|u| u.sync != attempt) {
                return;
            }
            self.shape_ui.update(|u| u.syncing = false);
            match result {
                Ok(doc)
                    if doc.id == t.document_id
                        && doc.free_text_editor.is_none()
                        && doc.shape_editor.as_ref().is_some_and(|o| {
                            self.shape_ui
                                .with_untracked(|u| u.session.matches_owner(doc.id, o))
                        }) =>
                {
                    self.message
                        .set("形状所有者一致；保留本地候选与精确重试载荷".into())
                }
                Ok(_) => self
                    .shape_error("原生形状所有者缺失／变化；不是完成证明，请精确重试或恢复".into()),
                Err(e) => self.shape_error(e),
            }
        });
    }
    pub(super) fn shape_key(self, event: web_sys::KeyboardEvent) {
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Shapes)
            || event.is_composing()
        {
            return;
        }
        if event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|e| {
                e.closest("input,textarea,select,button,summary,[contenteditable],[role=menu]")
                    .ok()
                    .flatten()
                    .is_some()
            })
        {
            return;
        }
        match event.key().as_str() {
            "Escape" => {
                event.prevent_default();
                self.cancel_canvas_shape();
            }
            "Delete" | "Backspace" => {
                event.prevent_default();
                self.delete_canvas_shape();
            }
            _ => {}
        }
    }
}
