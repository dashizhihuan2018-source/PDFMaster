#[path = "annotation_view.rs"]
mod annotation_view;
#[path = "bridge.rs"]
mod bridge;
#[path = "file_drop_view.rs"]
mod file_drop_view;
#[path = "home.rs"]
mod home;
#[path = "overview_view.rs"]
mod overview_view;
#[path = "thumbnail_view.rs"]
mod thumbnail_view;
#[path = "workbench_view.rs"]
mod workbench;
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::ink::{Frame, InkAction, InkGesture, InkImageHandoff};
use pdfmaster_ui::shell::{HomeSection, Route, document_reply_route, switch_route};
use pdfmaster_ui::workbench::{Action, Chrome, Mode, Panel, fit_page_zoom};
use wasm_bindgen::JsCast;
fn image_event_source(event: &web_sys::Event) -> Option<String> {
    event
        .target()?
        .dyn_into::<web_sys::HtmlImageElement>()
        .ok()
        .map(|image| image.src())
}

fn image_event_is_current(event: &web_sys::Event) -> bool {
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlImageElement>().ok())
    else {
        return false;
    };
    let Some(current) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("pdf-page-image"))
    else {
        return false;
    };
    target.is_same_node(Some(&current))
}

fn reveal_search_hit(index: usize) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(container) = document.get_element_by_id("search-results") else {
        return;
    };
    let Some(row) = document
        .get_element_by_id(&format!("search-hit-{index}"))
        .and_then(|row| row.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let height = container.client_height();
    if height <= 0 {
        return;
    }
    let top = f64::from(row.offset_top());
    container.set_scroll_top(pdfmaster_ui::reveal_scroll(
        f64::from(container.scroll_top()),
        top,
        top + f64::from(row.offset_height()),
        f64::from(height),
    ) as i32);
    // Mobile inspectors can clip the inner results scroller. Reveal the same
    // row inside that one ancestor too, without scrolling the app/body.
    if let Ok(Some(inspector)) = container.closest(".inspector") {
        let scroll = f64::from(inspector.scroll_top());
        let row_rect = row.get_bounding_client_rect();
        let top = scroll + row_rect.top()
            - inspector.get_bounding_client_rect().top()
            - f64::from(inspector.client_top());
        inspector.set_scroll_top(pdfmaster_ui::reveal_scroll(
            scroll,
            top,
            top + row_rect.height(),
            f64::from(inspector.client_height()),
        ) as i32);
    }
}
use pdfmaster_ui::{
    RasterPlan, adjacent_page, adjacent_search_hit, command_status_message, fit_width_zoom,
    line_edit, note_edit, page_batch_args, page_jump_index, page_jump_success_message,
    page_move_edit, raster_plan, search_conditions_message, search_response_is_current,
    search_status_message, shape_color_hex, shape_edit, step_zoom, update_note_edit,
    update_shape_edit, visible_status,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize)]
struct Page {
    width: f32,
    height: f32,
    rotation: i32,
}
#[derive(Clone, Debug, Deserialize)]
struct Info {
    page_count: usize,
    pages: Vec<Page>,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
}
#[derive(Clone, Debug, Deserialize)]
struct Document {
    #[serde(default)]
    id: u32,
    #[serde(default)]
    tabs: Vec<DocumentTab>,
    name: String,
    info: Info,
    restored: bool,
    #[serde(default)]
    warning: Option<String>,
    #[serde(default)]
    note_editor: Option<NoteDraft>,
    #[serde(default)]
    ink_editor: Option<InkDraft>,
}

#[derive(Clone, Debug, Deserialize)]
struct DocumentTab {
    id: u32,
    name: String,
    dirty: bool,
}
#[derive(Deserialize)]
struct CloseReply {
    closed: bool,
    document: Option<Document>,
}
#[derive(Clone)]
struct TabPresentation {
    page: usize,
    zoom: f32,
    chrome: Chrome,
    sidebar_mode: pdfmaster_ui::outline::Mode,
    annotation_sort: pdfmaster_ui::annotations::Sort,
    search_query: String,
    search_case_sensitive: bool,
    text: String,
    note_text: String,
    page_range: String,
    shape_bounds: String,
    shape_stroke: String,
    shape_fill: String,
    shape_line: String,
    line_points: String,
    ink_enabled: bool,
    overview_visible: bool,
    overview_scale: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct InkDraft {
    id: u32,
    page: usize,
    request: String,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Debug, Deserialize)]
struct TextNote {
    annotation_index: usize,
    bounds: NoteBounds,
    contents: String,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteList {
    notes: Vec<TextNote>,
    unsupported_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct ShapeList {
    shapes: Vec<Value>,
    unsupported_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteDraft {
    id: u32,
    page: usize,
    original: TextNote,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
struct SearchHit {
    page_index: usize,
    page_label: String,
    match_start: usize,
    match_end: usize,
    snippet: String,
    snippet_match_start: usize,
    snippet_match_end: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct SearchResponse {
    query: String,
    case_sensitive: bool,
    hits: Vec<SearchHit>,
    pages_scanned: usize,
    searchable_pages: usize,
    truncated: bool,
    limitation: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PageBatchReply {
    Cancelled,
    Applied {
        document: Box<Document>,
        affected_count: usize,
    },
}

#[cfg(test)]
mod batch_reply_contract {
    use super::*;

    #[test]
    fn native_batch_reply_keeps_stack_size_bounded() {
        assert!(
            std::mem::size_of::<PageBatchReply>() <= 64,
            "The host build must not inline the entire document in the applied variant"
        );
    }

    #[test]
    fn batch_reply_decodes_the_existing_native_wire_shape() {
        let reply: PageBatchReply = serde_json::from_value(json!({
            "kind":"applied", "affected_count":1,
            "document":{"name":"sample.pdf", "restored":false,
                "info":{"page_count":1,"pages":[{"width":595.,"height":842.,"rotation":0}],"dirty":true,"can_undo":true,"can_redo":false}}
        })).unwrap();
        let PageBatchReply::Applied {
            document,
            affected_count,
        } = reply
        else {
            panic!("expected applied reply")
        };
        assert_eq!(affected_count, 1);
        assert_eq!(document.name, "sample.pdf");
        assert!(document.info.dirty);
        assert!(matches!(
            serde_json::from_value::<PageBatchReply>(json!({"kind":"cancelled"})).unwrap(),
            PageBatchReply::Cancelled
        ));
    }
}

#[derive(Clone, Copy)]
struct Model {
    route: RwSignal<Route>,
    chrome: RwSignal<Chrome>,
    document: RwSignal<Option<Document>>,
    tab_presentations: RwSignal<std::collections::BTreeMap<u32, TabPresentation>>,
    thumbnail_cache: RwSignal<pdfmaster_ui::thumbnails::Cache>,
    thumbnail_window: RwSignal<pdfmaster_ui::thumbnails::Window>,
    overview_visible: RwSignal<bool>,
    overview_scale: RwSignal<f64>,
    overview_selection: RwSignal<pdfmaster_ui::overview::Selection>,
    overview_layout: RwSignal<pdfmaster_ui::overview::Layout>,
    overview_window: RwSignal<pdfmaster_ui::thumbnails::Window>,
    overview_cache: RwSignal<pdfmaster_ui::thumbnails::Cache>,
    overview_preparation: RwSignal<pdfmaster_ui::outline::ProjectionCache<u32>>,
    overview_drag: RwSignal<Option<overview_view::Pointer>>,
    overview_marker: RwSignal<Option<usize>>,
    file_drop_request: RwSignal<Option<u32>>,
    sidebar_mode: RwSignal<pdfmaster_ui::outline::Mode>,
    sidebar_width: RwSignal<pdfmaster_ui::sidebar_width::Width>,
    sidebar_viewport: RwSignal<f64>,
    outline_cache: RwSignal<pdfmaster_ui::outline::Cache>,
    outline_selection: RwSignal<Option<usize>>,
    annotation_cache:
        RwSignal<pdfmaster_ui::outline::ProjectionCache<pdfmaster_ui::annotations::Row>>,
    annotation_selection: RwSignal<Option<pdfmaster_ui::annotations::Identity>>,
    annotation_sort: RwSignal<pdfmaster_ui::annotations::Sort>,
    annotation_copying: RwSignal<bool>,
    annotation_navigation: RwSignal<pdfmaster_ui::annotation_navigation::Navigation>,
    annotation_image_ticket: RwSignal<Option<u64>>,
    page: RwSignal<usize>,
    zoom: RwSignal<f32>,
    image: RwSignal<String>,
    message: RwSignal<String>,
    busy: RwSignal<bool>,
    text: RwSignal<String>,
    note_text: RwSignal<String>,
    notes: RwSignal<Option<Result<NoteList, String>>>,
    note_draft: RwSignal<Option<NoteDraft>>,
    note_edit_text: RwSignal<String>,
    search_query: RwSignal<String>,
    search_case_sensitive: RwSignal<bool>,
    search_result: RwSignal<Option<SearchResponse>>,
    search_selection: RwSignal<Option<usize>>,
    search_flow: RwSignal<pdfmaster_ui::search_flow::Flow>,
    search_timer: RwSignal<Option<leptos::leptos_dom::helpers::TimeoutHandle>>,
    search_status: RwSignal<String>,
    page_jump: RwSignal<String>,
    page_range: RwSignal<String>,
    shape_bounds: RwSignal<String>,
    shape_stroke: RwSignal<String>,
    shape_fill: RwSignal<String>,
    shape_line: RwSignal<String>,
    line_points: RwSignal<String>,
    shapes: RwSignal<Option<Result<ShapeList, String>>>,
    selected_shape: RwSignal<Option<Value>>,
    ink_enabled: RwSignal<bool>,
    ink_gesture: RwSignal<Option<InkGesture>>,
    ink_handoff: RwSignal<bool>,
    ink_image: RwSignal<Option<InkImageHandoff>>,
    ink_image_revision: RwSignal<u32>,
    image_ready: RwSignal<bool>,
}

impl Model {
    fn remember_tab(self) {
        if let Some(id) = self.document.with_untracked(|d| d.as_ref().map(|d| d.id)) {
            let state = TabPresentation {
                page: self.page.get_untracked(),
                zoom: self.zoom.get_untracked(),
                chrome: self.chrome.get_untracked(),
                sidebar_mode: self.sidebar_mode.get_untracked(),
                annotation_sort: self.annotation_sort.get_untracked(),
                search_query: self.search_query.get_untracked(),
                search_case_sensitive: self.search_case_sensitive.get_untracked(),
                text: self.text.get_untracked(),
                note_text: self.note_text.get_untracked(),
                page_range: self.page_range.get_untracked(),
                shape_bounds: self.shape_bounds.get_untracked(),
                shape_stroke: self.shape_stroke.get_untracked(),
                shape_fill: self.shape_fill.get_untracked(),
                shape_line: self.shape_line.get_untracked(),
                line_points: self.line_points.get_untracked(),
                ink_enabled: self.ink_enabled.get_untracked(),
                overview_visible: self.overview_visible.get_untracked(),
                overview_scale: self.overview_scale.get_untracked(),
            };
            self.tab_presentations.update(|tabs| {
                tabs.insert(id, state);
            });
        }
    }
    fn restore_tab(self, document: &Document) {
        if self
            .document
            .with_untracked(|old| old.as_ref().is_some_and(|old| old.id == document.id))
        {
            return;
        }
        self.image.set(String::new());
        self.image_ready.set(false);
        self.ink_image.set(None);
        let saved = self
            .tab_presentations
            .with_untracked(|tabs| tabs.get(&document.id).cloned());
        let state = saved.unwrap_or_else(|| TabPresentation {
            page: 0,
            zoom: 1.,
            chrome: Chrome {
                sidebar: !web_sys::window()
                    .and_then(|w| w.inner_width().ok())
                    .and_then(|v| v.as_f64())
                    .is_some_and(|width| width <= 720.),
                ..Chrome::default()
            },
            sidebar_mode: Default::default(),
            annotation_sort: Default::default(),
            search_query: String::new(),
            search_case_sensitive: false,
            text: String::new(),
            note_text: String::new(),
            page_range: String::new(),
            shape_bounds: "36,36,120,80".into(),
            shape_stroke: "#4F46E5".into(),
            shape_fill: String::new(),
            shape_line: "2".into(),
            line_points: "72,200,300,300".into(),
            ink_enabled: false,
            overview_visible: false,
            overview_scale: 1.,
        });
        self.page
            .set(state.page.min(document.info.page_count.saturating_sub(1)));
        self.zoom.set(state.zoom);
        self.chrome.set(state.chrome);
        self.sidebar_mode.set(state.sidebar_mode);
        self.annotation_sort.set(state.annotation_sort);
        self.search_query.set(state.search_query);
        self.search_case_sensitive.set(state.search_case_sensitive);
        self.text.set(state.text);
        self.note_text.set(state.note_text);
        self.page_range.set(state.page_range);
        self.shape_bounds.set(state.shape_bounds);
        self.shape_stroke.set(state.shape_stroke);
        self.shape_fill.set(state.shape_fill);
        self.shape_line.set(state.shape_line);
        self.line_points.set(state.line_points);
        self.ink_enabled.set(state.ink_enabled);
        self.overview_visible.set(state.overview_visible);
        self.overview_scale.set(state.overview_scale);
    }
    fn close_tab(self, id: u32) {
        if self.blocked() {
            return;
        }
        self.remember_tab();
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<CloseReply>("close_document", &json!({"id":id})).await {
                Ok(reply) if reply.closed => {
                    self.tab_presentations.update(|tabs| {
                        tabs.remove(&id);
                    });
                    if let Some(document) = reply.document {
                        self.restore_tab(&document);
                        self.page_jump
                            .set((self.page.get_untracked() + 1).to_string());
                        self.replace_document_view(Some(document));
                        self.message
                            .set("文档标签已关闭；其余文档保持不变。".into());
                        self.render().await;
                    } else {
                        self.replace_document_view(None);
                        self.image.set(String::new());
                        self.route.set(Route::Home(HomeSection::Home));
                        self.message.set("文档标签已关闭。".into());
                    }
                }
                Ok(_) => self.message.set("已取消关闭，文档保持不变。".into()),
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn replace_document_view(self, document: Option<Document>) {
        self.overview_preparation.update(|c| c.invalidate());
        self.overview_cache.update(|c| {
            c.invalidate();
        });
        self.overview_drag.set(None);
        self.overview_marker.set(None);
        self.overview_selection.update(|s| {
            s.reset(
                self.page.get_untracked(),
                document.as_ref().map(|d| d.info.page_count).unwrap_or(0),
            )
        });
        self.annotation_navigation.update(|nav| nav.cancel());
        self.annotation_cache.update(|cache| cache.invalidate());
        self.annotation_selection.set(None);
        self.outline_cache.update(|cache| cache.invalidate());
        self.outline_selection.set(None);
        self.stop_search_timer();
        self.search_flow.update(|flow| {
            flow.changed();
        });
        self.search_status.set("输入关键词搜索".into());
        self.search_selection.set(None);
        self.search_result.set(None);
        self.thumbnail_cache.update(|cache| {
            cache.invalidate();
        });
        self.document.set(document);
    }
    fn close_chrome_popups(self) {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            // There are exactly two local menus. Bound the DOM work even if a
            // future markup change introduces an unexpected open element.
            for _ in 0..2 {
                let Ok(Some(menu)) = document.query_selector(".workbench-toolbar details[open]")
                else {
                    break;
                };
                let _ = menu.remove_attribute("open");
            }
        }
    }
    fn select_mode(self, mode: Mode) {
        let blocked = self.blocked();
        if blocked {
            return;
        }
        self.close_chrome_popups();
        self.ink_enabled.set(false);
        self.chrome
            .update(|chrome| *chrome = chrome.select_mode(mode, blocked));
        self.overview_visible.set(mode == Mode::Pages);
        if mode == Mode::Pages {
            self.overview_selection
                .update(|s| s.reset(self.page.get_untracked(), self.count()));
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                self.measure_overview(true)
            });
        }
        self.message
            .set("选择二级工具后才会修改文档；灰色工具尚未迁移。".into());
    }
    fn select_panel(self, panel: Option<Panel>) {
        if panel == Some(Panel::Search) {
            self.show_search_sidebar();
            return;
        }
        let blocked = self.blocked();
        if blocked {
            return;
        }
        if matches!(
            panel,
            Some(Panel::Pages | Panel::Text | Panel::Notes | Panel::Ink | Panel::Shapes)
        ) {
            self.overview_visible.set(false);
        }
        self.close_chrome_popups();
        if panel != Some(Panel::Ink) {
            self.ink_enabled.set(false);
        }
        self.chrome
            .update(|chrome| *chrome = chrome.select_panel(panel, blocked));
    }
    fn toggle_sidebar(self) {
        let blocked = self.blocked();
        self.chrome
            .update(|chrome| *chrome = chrome.toggle_sidebar(blocked));
    }
    fn choose_zoom(self, value: &str) {
        if self.blocked() || self.overview_visible.get_untracked() {
            return;
        }
        match value {
            "page" => self.fit_page(),
            "width" => self.fit_width(),
            "1" | "1.25" | "1.5" | "2" | "3" => {
                self.zoom.set(value.parse().unwrap_or(1.));
                self.refresh();
            }
            _ => {}
        }
    }
    fn fit_page(self) {
        if self.blocked() {
            return;
        }
        if self.overview_visible.get_untracked() {
            self.overview_scale.set(1.);
            self.overview_zoom(1.);
            return;
        }
        self.close_chrome_popups();
        let Some(reader) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("pdf-reader"))
        else {
            return;
        };
        let zoom = self.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(self.page.get_untracked()))
                .and_then(|p| {
                    fit_page_zoom(
                        p.width,
                        p.height,
                        p.rotation,
                        reader.client_width() as f32 - 40.,
                        reader.client_height() as f32 - 40.,
                    )
                })
        });
        if let Some(zoom) = zoom {
            self.zoom.set(zoom);
            self.refresh();
        }
    }
    fn run_strip_action(self, action: Action) {
        if self.blocked() {
            return;
        }
        match action {
            Action::Panel(panel) => self.select_panel(Some(panel)),
            Action::CopyPages => self.copy_selected_pages(false),
            Action::PastePages => self.paste_selected_pages(),
            Action::InsertBlank => {
                if let Some(index) = self.page_insertion_index() {
                    self.command_at_page("edit_document",json!({"edit":{"kind":"insert_blank","index":index,"width":595.,"height":842.}}),Some(index));
                }
            }
            Action::InsertPdf => {
                if let Some(index) = self.page_insertion_index() {
                    self.command_at_page(
                        "insert_pdf_document",
                        json!({"index":index}),
                        Some(index),
                    );
                }
            }
            Action::Rotate => {
                if self.overview_visible.get_untracked() {
                    self.overview_batch("rotate_90");
                } else {
                    self.edit(
                        json!({"kind":"rotate","page":self.page.get_untracked(),"degrees":90}),
                    );
                }
            }
            Action::Extract => {
                let range = if self.overview_visible.get_untracked() {
                    self.overview_selection.with_untracked(|s| {
                        s.pages()
                            .iter()
                            .map(|p| (p + 1).to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                } else {
                    (self.page.get_untracked() + 1).to_string()
                };
                self.select_panel(Some(Panel::Batch));
                self.page_range.set(range);
            }
            Action::DeleteCurrent => {
                if self.overview_visible.get_untracked() {
                    self.overview_batch("delete");
                } else {
                    self.delete_current_page();
                }
            }
            Action::Export => self.command("export_document", json!({})),
        }
    }
    fn page_insertion_index(self) -> Option<usize> {
        if self.overview_visible.get_untracked() {
            self.overview_selection
                .with_untracked(|s| s.pages().last().map(|p| p + 1))
        } else {
            Some(self.page.get_untracked() + 1)
        }
    }
    fn switch_to(self, target: Route) {
        let current = self.route.get_untracked();
        let next = switch_route(
            current,
            target,
            self.loaded(),
            self.busy.get_untracked() || self.has_note_draft() || self.has_ink(),
        );
        if next != target {
            self.message
                .set("请先结束当前操作，保存或取消便笺／手绘编辑，再切换界面。".into());
        }
        self.route.set(next);
        if next != current && matches!(next, Route::Home(_)) {
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                if let Some(scroll) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("home-scroll"))
                {
                    scroll.set_scroll_top(0);
                }
            });
        }
    }
    fn show_tool(self, id: &'static str) {
        if id == "blank" {
            self.command("new_document", json!({}));
            return;
        }
        let Some(target) = pdfmaster_ui::shell::tool(id).and_then(|t| t.workbench_target) else {
            return;
        };
        self.switch_to(Route::Document);
        if self.route.get_untracked() == Route::Document {
            let mode = match id {
                "pages" | "merge" => Mode::Pages,
                "draw" => Mode::Drawing,
                _ => Mode::Annotation,
            };
            self.select_mode(mode);
            let panel = match target {
                "tool-notes" => Some(Panel::Notes),
                "tool-pages" => Some(Panel::Pages),
                "tool-batch" => Some(Panel::Batch),
                "tool-ink" => Some(Panel::Ink),
                _ => None,
            };
            self.select_panel(panel);
        }
    }
    fn complete_ink_handoff(self) {
        self.ink_handoff.set(false);
        self.ink_image.set(None);
        self.ink_gesture.set(None);
        self.busy.set(false);
    }
    fn image_loaded(self, source: Option<String>, owner: Option<u64>) {
        if self.annotation_image_ticket.get_untracked() != owner
            || !self
                .image
                .with_untracked(|current| source.as_ref() == Some(current))
        {
            return;
        }
        self.image_ready.set(true);
        if self.ink_handoff.get_untracked() {
            let ready = self.ink_image.with_untracked(|h| {
                h.as_ref()
                    .is_some_and(|h| source.as_ref().is_some_and(|s| h.loaded(h.revision, s)))
            });
            if ready {
                self.complete_ink_handoff();
            }
        }
        self.queue_annotation_reveal();
    }
    fn page_image_failed(self, owner: Option<u64>, source: &str) {
        if let Some(ticket) = owner {
            self.annotation_navigation.update(|nav| nav.abandon(ticket));
        }
        if self.annotation_image_ticket.get_untracked() == owner
            && self.image.with_untracked(|current| current == source)
        {
            self.image_ready.set(false);
            if let Some(h) = self.ink_image.get_untracked() {
                self.ink_image_failed(h.revision);
            }
        }
    }
    fn ink_image_decoded(self, revision: u32) {
        let mut image = None;
        self.ink_image.update(|h| {
            if let Some(h) = h {
                image = h.decoded(revision);
            }
        });
        if let Some(image) = image {
            if self.image.with_untracked(|current| current == &image)
                && annotation_view::decoded_page_image(self).is_some()
            {
                // The primary element may have loaded before its staged twin.
                // A keyed identical source will not emit a second load event.
                self.image_loaded(Some(image), self.annotation_image_ticket.get_untracked());
            } else {
                self.image_ready.set(false);
                self.image.set(image);
            }
        }
    }
    fn ink_image_failed(self, revision: u32) {
        let previous = self
            .ink_image
            .with_untracked(|h| h.as_ref().and_then(|h| h.failed(revision)));
        if let Some(previous) = previous {
            self.ink_image.set(None);
            self.image.set(previous);
            self.ink_handoff.set(false);
            self.image_ready.set(false);
            self.ink_gesture.update(|s| {
                if let Some(s) = s {
                    s.committing = false;
                }
            });
            self.busy.set(false);
            self.message.set("笔画原生状态已更新，但页面图片未加载。旧画面和本笔输入仍保留；请重试提交刷新或取消／同步手绘核对，重试不会重复添加已提交笔画。".into());
        }
    }
    fn pointer_down(self, event: web_sys::PointerEvent, surface: NodeRef<leptos::html::Div>) {
        if !self.ink_enabled.get_untracked()
            || self.blocked()
            || !self.image_ready.get_untracked()
            || !event.is_primary()
            || event.button() != 0
        {
            return;
        }
        let Some(node) = surface.get() else {
            return;
        };
        let Some(page) = self.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(self.page.get_untracked()))
                .cloned()
        }) else {
            return;
        };
        let rect = node.get_bounding_client_rect();
        let result = InkGesture::begin(
            event.pointer_id(),
            self.page.get_untracked(),
            page.width,
            page.height,
            page.rotation,
            Frame {
                left: rect.left(),
                top: rect.top(),
                width: rect.width(),
                height: rect.height(),
            },
            f64::from(event.client_x()),
            f64::from(event.client_y()),
            &self.shape_stroke.get_untracked(),
            &self.shape_line.get_untracked(),
        );
        let mut gesture = match result {
            Ok(s) => s,
            Err(e) => {
                self.message.set(e);
                return;
            }
        };
        let mut random = [0u8; 16];
        let Some(crypto) = web_sys::window().and_then(|w| w.crypto().ok()) else {
            self.message.set("无法建立手绘请求身份，未开始。".into());
            return;
        };
        if crypto.get_random_values_with_u8_array(&mut random).is_err() {
            self.message.set("无法建立手绘请求身份，未开始。".into());
            return;
        }
        gesture.request = random.iter().map(|b| format!("{b:02x}")).collect();
        if node.set_pointer_capture(event.pointer_id()).is_err() {
            self.message.set("无法捕获绘制指针，未开始手绘。".into());
            return;
        }
        event.prevent_default();
        let args = json!({"request":gesture.request,"page":gesture.page,"color":gesture.color,"line_width":gesture.width});
        self.ink_gesture.set(Some(gesture));
        self.message.set("正在保护手绘会话…".into());
        spawn_local(async move {
            match bridge::call_bounded::<InkDraft>("begin_ink_document", &args).await {
                Ok(editor) => {
                    // A pending capture is retained through cancellation until this acknowledgement.
                    self.document.update(|d| {
                        if let Some(d) = d {
                            d.ink_editor = Some(editor.clone());
                        }
                    });
                    let mut action = InkAction::Wait;
                    self.ink_gesture.update(|s| {
                        if let Some(s) = s {
                            action = s.acknowledge(editor.id);
                        }
                    });
                    self.message
                        .set("手绘中；抬笔提交。尚未提交的采样点未自动持久化。".into());
                    self.ink_action(action);
                }
                Err(e) => {
                    self.ink_gesture.update(|s| {
                        if let Some(s) = s {
                            s.begin_pending = false;
                            s.ended = true;
                        }
                    });
                    self.message
                        .set(format!("{e}；请使用取消／同步手绘核对原生会话。"));
                }
            }
        });
    }
    fn pointer_sample(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        if !self.ink_gesture.with_untracked(|s| {
            s.as_ref()
                .is_some_and(|s| s.pointer == event.pointer_id() && !s.ended && !s.committing)
        }) {
            return false;
        }
        event.prevent_default();
        let Some(node) = surface.get() else {
            self.cancel_ink();
            return false;
        };
        let r = node.get_bounding_client_rect();
        let mut error = None;
        self.ink_gesture.update(|s| {
            if let Some(s) = s
                && let Err(e) = s.sample(
                    event.pointer_id(),
                    Frame {
                        left: r.left(),
                        top: r.top(),
                        width: r.width(),
                        height: r.height(),
                    },
                    f64::from(event.client_x()),
                    f64::from(event.client_y()),
                )
            {
                error = Some(e);
            }
        });
        if let Some(e) = error {
            self.cancel_ink();
            self.message.set(e);
            return false;
        }
        true
    }
    fn pointer_up(self, event: web_sys::PointerEvent, surface: NodeRef<leptos::html::Div>) {
        if !self.pointer_sample(&event, surface) {
            return;
        }
        let mut action = InkAction::Wait;
        self.ink_gesture.update(|s| {
            if let Some(s) = s {
                action = s.finish(event.pointer_id());
            }
        });
        if let Some(node) = surface.get() {
            let _ = node.release_pointer_capture(event.pointer_id());
        }
        self.ink_action(action);
    }
    fn pointer_cancel(self, event: web_sys::PointerEvent) {
        if self.ink_gesture.with_untracked(|s| {
            s.as_ref()
                .is_some_and(|s| s.pointer == event.pointer_id() && !s.ended && !s.committing)
        }) {
            self.cancel_ink();
        }
    }
    fn retry_ink(self) {
        let mut action = InkAction::Wait;
        self.ink_gesture.update(|s| {
            if let Some(s) = s {
                action = s.retry();
            }
        });
        self.ink_action(action);
    }
    fn cancel_ink(self) {
        if self.busy.get_untracked() {
            return;
        }
        if self.ink_gesture.with_untracked(|s| {
            s.as_ref()
                .is_some_and(|s| s.begin_pending || s.guard.is_some() || !s.ended)
        }) {
            let mut action = InkAction::Wait;
            self.ink_gesture.update(|s| {
                if let Some(s) = s {
                    action = s.cancel();
                }
            });
            self.ink_action(action);
            return;
        }
        if let Some(request) = self
            .ink_gesture
            .with_untracked(|s| s.as_ref().map(|s| s.request.clone()))
        {
            self.ink_action(InkAction::CancelRequest { request });
            return;
        }
        // Includes a lost begin acknowledgement and a reloaded WebView: inspect read-only
        // document state first; never assume that a failed reply means no native guard.
        self.busy.set(true);
        spawn_local(async move {
            let result =
                bridge::call_bounded::<Option<Document>>("document_info", &json!({})).await;
            self.busy.set(false);
            match result {
                Ok(Some(doc)) => {
                    let request = doc.ink_editor.as_ref().map(|e| e.request.clone());
                    self.replace_document_view(Some(doc));
                    if let Some(request) = request {
                        self.ink_action(InkAction::CancelRequest { request });
                    } else {
                        self.ink_gesture.set(None);
                        self.message
                            .set("原生手绘会话已结束；已提交的笔画仍保留，可撤销。".into());
                        self.refresh();
                    }
                }
                Ok(None) => {
                    self.ink_gesture.set(None);
                    self.replace_document_view(None);
                }
                Err(e) => self.message.set(e),
            }
        });
    }
    fn ink_action(self, action: InkAction) {
        let (command, args) = match action {
            InkAction::Wait => return,
            InkAction::Commit { id, points } => {
                ("commit_ink_document", json!({"id":id,"strokes":[points]}))
            }
            InkAction::Cancel { id } => ("cancel_ink_document", json!({"id":id})),
            InkAction::CancelRequest { request } => {
                ("cancel_ink_request_document", json!({"request":request}))
            }
        };
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call_bounded::<Document>(command, &args).await {
                Ok(doc) => {
                    self.replace_document_view(Some(doc));
                    self.search_result.set(None);
                    self.ink_handoff.set(true);
                    self.message.set(
                        if command == "commit_ink_document" {
                            "手绘已提交，可撤销；请导出 PDF 保存。"
                        } else {
                            "手绘会话已结束；已提交笔画仍保留，可撤销。"
                        }
                        .into(),
                    );
                    if !self.render().await {
                        self.ink_handoff.set(false);
                        self.ink_gesture.update(|s| {
                            if let Some(s) = s {
                                s.committing = false;
                            }
                        });
                        self.busy.set(false);
                    }
                    // Otherwise keep the preview and operation lock until the PDF image's
                    // load event; bridge completion alone does not mean visible handoff.
                }
                Err(e) => {
                    self.ink_gesture.update(|s| {
                        if let Some(s) = s {
                            s.committing = false;
                        }
                    });
                    self.message
                        .set(format!("{e}；笔画输入仍保留，请重试提交或取消／同步手绘。"));
                    self.busy.set(false);
                }
            }
        });
    }
    fn loaded(self) -> bool {
        self.document.with(|d| d.is_some())
    }
    fn count(self) -> usize {
        self.document
            .with(|d| d.as_ref().map_or(0, |d| d.info.page_count))
    }
    fn blocked(self) -> bool {
        self.busy.get() || !self.loaded() || self.has_note_draft() || self.has_ink()
    }
    fn has_ink(self) -> bool {
        self.ink_gesture.with(|s| s.is_some())
            || self
                .document
                .with(|d| d.as_ref().is_some_and(|d| d.ink_editor.is_some()))
    }
    fn has_note_draft(self) -> bool {
        self.note_draft.with(|draft| draft.is_some())
    }
    fn raster(self) -> Option<RasterPlan> {
        let density = web_sys::window().map_or(1., |w| w.device_pixel_ratio() as f32);
        self.document.with(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(self.page.get()))
                .map(|p| raster_plan(p.width, p.height, p.rotation, self.zoom.get(), density))
        })
    }
    fn command(self, command: &'static str, args: Value) {
        self.command_at_page(command, args, None);
    }
    fn command_at_page(self, command: &'static str, args: Value, selected_page: Option<usize>) {
        if self.busy.get_untracked() {
            return;
        }
        if self.has_note_draft() {
            self.message.set("请先保存或取消便笺编辑。".into());
            return;
        }
        if self.has_ink() {
            self.message.set("请先结束或取消手绘。".into());
            return;
        }
        self.remember_tab();
        let overview_insert = self.overview_visible.get_untracked()
            && (matches!(
                command,
                "insert_pdf_document" | "insert_pdfs_document" | "paste_pages_document"
            ) || (command == "edit_document"
                && args["edit"]["kind"].as_str() == Some("insert_blank")));
        let old_count = self.count();
        self.busy.set(true);
        spawn_local(async move {
            let result: Result<Option<Document>, _> = bridge::call(command, &args).await;
            match result {
                Ok(Some(doc)) => {
                    // Restore tab-local presentation first. Native unfinished
                    // editors are authoritative and must then reveal their own
                    // page and Save/Cancel panel after a frontend reload.
                    self.restore_tab(&doc);
                    self.page.update(|p| {
                        *p = selected_page
                            .unwrap_or(*p)
                            .min(doc.info.page_count.saturating_sub(1))
                    });
                    let mut status =
                        command_status_message(command, doc.restored, doc.warning.as_deref());
                    if let Some(editor) = &doc.note_editor {
                        self.page.set(editor.page);
                        self.note_edit_text.set(editor.original.contents.clone());
                        if doc.warning.is_none() {
                            status = "便笺编辑尚未结束；当前显示原始内容。编辑框输入未持久化，请核对后保存或取消。".into();
                        }
                    }
                    self.note_draft.set(doc.note_editor.clone());
                    if doc.note_editor.is_some() || doc.ink_editor.is_some() {
                        self.overview_visible.set(false);
                    }
                    if let Some(editor) = &doc.ink_editor {
                        self.page.set(editor.page);
                        status="手绘会话尚未结束；未提交的笔画无法从进程恢复，请取消／同步手绘后继续。".into();
                    }
                    let page_count = doc.info.page_count;
                    self.chrome.update(|chrome| {
                        *chrome = chrome.document_opened(
                            doc.note_editor.is_some(),
                            doc.ink_editor.is_some(),
                            matches!(command, "open_document" | "new_document"),
                        )
                    });
                    self.replace_document_view(Some(doc));
                    if overview_insert && let Some(start) = selected_page {
                        let end = start.saturating_add(page_count.saturating_sub(old_count));
                        self.overview_selection
                            .update(|s| s.restore(&(start..end).collect::<Vec<_>>(), page_count));
                        leptos::leptos_dom::helpers::request_animation_frame(move || {
                            self.measure_overview(true)
                        });
                    }
                    self.route
                        .update(|route| *route = document_reply_route(*route, command, true));
                    if matches!(
                        command,
                        "open_document"
                            | "new_document"
                            | "edit_document"
                            | "delete_note_document"
                            | "delete_shape_document"
                            | "delete_annotation_document"
                            | "insert_pdf_document"
                            | "insert_pdfs_document"
                            | "paste_pages_document"
                            | "undo_document"
                            | "redo_document"
                    ) {
                        self.search_result.set(None);
                    }
                    if page_count > 0 {
                        self.page_jump
                            .set((self.page.get_untracked() + 1).to_string());
                    }
                    if matches!(command, "open_document" | "new_document" | "document_info")
                        && web_sys::window()
                            .and_then(|w| w.inner_width().ok())
                            .and_then(|v| v.as_f64())
                            .is_some_and(|w| w <= 720.)
                    {
                        self.set_fit_width();
                    }
                    self.message.set(status);
                    self.render().await;
                }
                Ok(None) => self.message.set(if command == "document_info" {
                    String::new()
                } else {
                    "已取消，文档保持不变。".into()
                }),
                Err(e) => self.message.set(e),
            }
            self.busy.set(false);
        });
    }
    fn edit(self, edit: Value) {
        self.command("edit_document", json!({"edit":edit}));
    }
    async fn render(self) -> bool {
        self.notes.set(None);
        self.shapes.set(None);
        self.selected_shape.set(None);
        if !self.loaded() {
            return false;
        }
        let Some(plan) = self.raster() else {
            return false;
        };
        let args = json!({"page":self.page.get_untracked(),"scale":plan.scale});
        let annotation_ticket = self
            .annotation_navigation
            .with_untracked(|nav| nav.ticket());
        match bridge::call_bounded::<String>("render_page", &args).await {
            Ok(image) => {
                let same_decoded_image = self.image.with_untracked(|current| current == &image)
                    && annotation_view::decoded_page_image(self).is_some();
                if self.annotation_image_ticket.get_untracked() != annotation_ticket {
                    self.image_ready.set(false);
                }
                self.annotation_image_ticket.set(annotation_ticket);
                if same_decoded_image {
                    if self.ink_handoff.get_untracked() {
                        self.complete_ink_handoff();
                    }
                } else if self.ink_handoff.get_untracked() {
                    let Some(revision) = self.ink_image_revision.get_untracked().checked_add(1)
                    else {
                        self.message
                            .set("图片会话编号已耗尽，请先保存并正常重启。".into());
                        return false;
                    };
                    self.ink_image_revision.set(revision);
                    self.ink_image.set(Some(InkImageHandoff {
                        previous: self.image.get_untracked(),
                        next: image,
                        revision,
                        decoded: false,
                    }));
                    if set_timeout_with_handle(
                        move || self.ink_image_failed(revision),
                        std::time::Duration::from_secs(15),
                    )
                    .is_err()
                    {
                        self.ink_image_failed(revision);
                        return false;
                    }
                } else {
                    self.image_ready.set(false);
                    self.image.set(image);
                }
            }
            Err(e) => {
                if let Some(ticket) = annotation_ticket {
                    self.annotation_navigation.update(|nav| nav.abandon(ticket));
                }
                if !self.ink_handoff.get_untracked() {
                    self.image.set(String::new());
                    self.image_ready.set(false);
                }
                self.message.set(e);
                return false;
            }
        }
        if let Some(ticket) = annotation_ticket {
            self.annotation_navigation
                .update(|nav| nav.rendered(ticket));
            self.queue_annotation_reveal();
        }
        self.refresh_notes().await;
        self.shapes.set(Some(
            bridge::call_bounded::<ShapeList>(
                "shapes_document",
                &json!({"page":self.page.get_untracked()}),
            )
            .await,
        ));
        true
    }
    async fn refresh_notes(self) {
        self.notes.set(Some(
            bridge::call_bounded::<NoteList>(
                "notes_document",
                &json!({"page":self.page.get_untracked()}),
            )
            .await,
        ));
    }
    fn navigate(self, page: usize) {
        self.navigate_with_annotation(page, None);
    }
    fn navigate_with_annotation(
        self,
        page: usize,
        target: Option<pdfmaster_ui::annotation_navigation::Target>,
    ) {
        if self.busy.get_untracked() || !self.loaded() {
            return;
        }
        if self.has_note_draft() {
            self.message.set("请先保存或取消便笺编辑。".into());
            return;
        }
        if self.has_ink() {
            self.message.set("请先结束或取消手绘。".into());
            return;
        }
        if page >= self.count() {
            self.message.set("页码无效，请输入现有页面。".into());
            return;
        }
        self.page.set(page);
        self.overview_visible.set(false);
        self.page_jump.set((page + 1).to_string());
        self.refresh_with_annotation(target);
    }
    fn jump_to_page(self) {
        if self.blocked() {
            return;
        }
        match page_jump_index(&self.page_jump.get_untracked(), self.count()) {
            Ok(page) => {
                self.message
                    .update(|message| *message = page_jump_success_message(message));
                self.navigate(page);
            }
            Err(error) => self.message.set(error),
        }
    }
    fn batch_pages(self, operation: &'static str) {
        self.batch_range(self.page_range.get_untracked(), operation);
    }
    fn delete_current_page(self) {
        self.batch_range((self.page.get_untracked() + 1).to_string(), "delete");
    }
    fn move_page(self, delta: i32) {
        if self.blocked() {
            return;
        }
        if let Some((edit, target)) = page_move_edit(self.page.get_untracked(), delta, self.count())
        {
            self.command_at_page("edit_document", json!({"edit":edit}), Some(target));
        }
    }
    fn batch_range(self, range: String, operation: &'static str) {
        self.batch_range_with_selection(range, operation, None);
    }
    fn batch_range_with_selection(
        self,
        range: String,
        operation: &'static str,
        overview_selection: Option<pdfmaster_ui::overview::Selection>,
    ) {
        if self.blocked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let args = page_batch_args(&range, operation);
            let result = bridge::call::<PageBatchReply>("batch_pages", &args).await;
            match result {
                Ok(PageBatchReply::Applied {
                    document,
                    affected_count,
                }) => {
                    if operation == "delete"
                        && let Some(selection) = &overview_selection
                    {
                        self.page
                            .set(selection.pages().first().copied().unwrap_or(0));
                    }
                    self.page.update(|page| {
                        *page = (*page).min(document.info.page_count.saturating_sub(1))
                    });
                    self.page_jump
                        .set((self.page.get_untracked() + 1).to_string());
                    self.search_result.set(None);
                    self.message
                        .set(document.warning.clone().unwrap_or_else(|| {
                            if operation == "delete" {
                                format!("已删除 {affected_count} 页，可使用撤销恢复。")
                            } else {
                                format!("已顺时针旋转 {affected_count} 页。")
                            }
                        }));
                    self.replace_document_view(Some(*document));
                    if let Some(selection) = overview_selection {
                        self.overview_selection.update(|s| {
                            if operation == "delete" {
                                s.reset(self.page.get_untracked(), self.count());
                            } else {
                                *s = selection;
                            }
                        });
                        leptos::leptos_dom::helpers::request_animation_frame(move || {
                            self.measure_overview(true)
                        });
                    }
                    self.render().await;
                }
                Ok(PageBatchReply::Cancelled) => {
                    self.message.set("已取消批量删除，文档保持不变。".into());
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn invalidate_search(self) {
        self.stop_search_timer();
        self.search_flow.update(|flow| {
            flow.changed();
        });
        self.search_status.set("输入关键词搜索".into());
        self.search_selection.set(None);
        let summary = self.search_result.with_untracked(|result| {
            result.as_ref().map(|result| {
                search_status_message(&result.query, result.hits.len(), result.truncated)
            })
        });
        self.message
            .update(|message| *message = search_conditions_message(message, summary.as_deref()));
        self.search_result.set(None);
    }
    fn stop_search_timer(self) {
        if let Some(timer) = self.search_timer.get_untracked() {
            timer.clear();
            self.search_timer.set(None);
        }
    }
    fn search_input_blocked(self) -> bool {
        self.search_flow.with(|flow| {
            flow.input_blocked(
                self.loaded(),
                self.busy.get(),
                self.has_note_draft() || self.has_ink(),
            )
        })
    }
    fn show_search_sidebar(self) {
        if self.search_input_blocked() {
            return;
        }
        self.close_chrome_popups();
        self.overview_visible.set(false);
        self.ink_enabled.set(false);
        self.chrome.update(|chrome| {
            chrome.sidebar = true;
            chrome.panel = None;
        });
        self.sidebar_mode.set(pdfmaster_ui::outline::Mode::Search);
    }
    fn queue_live_search(self) {
        if self.search_input_blocked() {
            return;
        }
        self.invalidate_search();
        let revision = self.search_flow.with_untracked(|flow| flow.revision());
        let Some(revision) = revision else {
            return;
        };
        if self.search_query.get_untracked().trim().is_empty() {
            return;
        }
        self.search_status.set("正在搜索…".into());
        let timer = leptos::leptos_dom::helpers::set_timeout_with_handle(
            move || {
                if self.search_flow.with_untracked(|flow| flow.revision()) != Some(revision) {
                    return;
                }
                self.search_timer.set(None);
                self.search_flow.update(|flow| {
                    flow.queue(revision);
                });
            },
            std::time::Duration::from_millis(180),
        );
        match timer {
            Ok(timer) => self.search_timer.set(Some(timer)),
            Err(_) => self
                .search_status
                .set("无法安排搜索，请按回车重试。".into()),
        }
    }
    fn cancel_search(self) {
        if self.search_input_blocked() {
            return;
        }
        self.invalidate_search();
        self.search_status.set("搜索已取消".into());
    }
    fn select_search_hit(self, index: usize) {
        if self.blocked() {
            return;
        }
        let page = self
            .search_result
            .with_untracked(|result| result.as_ref()?.hits.get(index).map(|hit| hit.page_index));
        if let Some(page) = page.filter(|page| *page < self.count()) {
            self.search_selection.set(Some(index));
            self.navigate(page);
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                if self.search_selection.get_untracked() == Some(index) {
                    reveal_search_hit(index);
                }
            });
        }
    }
    fn step_search_hit(self, backward: bool) {
        if self.blocked() {
            return;
        }
        let count = self
            .search_result
            .with_untracked(|r| r.as_ref().map_or(0, |r| r.hits.len()));
        if let Some(index) =
            adjacent_search_hit(self.search_selection.get_untracked(), count, backward)
        {
            self.select_search_hit(index);
        }
    }
    fn search(self) {
        if self.blocked() {
            return;
        }
        self.stop_search_timer();
        let query = self.search_query.get_untracked();
        if query.trim().is_empty() {
            self.invalidate_search();
            self.message.set("请输入搜索关键词。".into());
            return;
        }
        let mut ticket = None;
        self.search_flow.update(|flow| {
            flow.request();
            ticket = flow.begin();
        });
        let Some(ticket) = ticket else {
            return;
        };
        let case_sensitive = self.search_case_sensitive.get_untracked();
        let requested_query = query.trim().to_owned();
        self.search_selection.set(None);
        self.search_result.set(None);
        self.busy.set(true);
        self.search_status.set("正在搜索…".into());
        spawn_local(async move {
            let args = json!({"query":query,"case_sensitive":case_sensitive});
            let response = bridge::call_bounded::<SearchResponse>("search_document", &args).await;
            if response
                .as_ref()
                .is_err_and(|error| error.starts_with("原生响应超时；"))
            {
                self.search_flow.update(|flow| {
                    flow.suspend(ticket);
                });
                self.search_status
                    .set("搜索已暂停；请先保存文档，再正常重启候选。".into());
                self.busy.set(false);
                return;
            }
            let mut current = None;
            self.search_flow
                .update(|flow| current = flow.finish(ticket));
            let Some(current) = current else {
                return;
            };
            if !current {
                self.busy.set(false);
                return;
            }
            match response {
                Ok(result)
                    if search_response_is_current(
                        &requested_query,
                        case_sensitive,
                        &result.query,
                        result.case_sensitive,
                    ) =>
                {
                    self.search_status.set(if result.hits.is_empty() {
                        format!("未找到“{}”", result.query)
                    } else {
                        format!(
                            "1 / {}{}",
                            result.hits.len(),
                            if result.truncated {
                                " · 已达上限，请细化关键词"
                            } else {
                                ""
                            }
                        )
                    });
                    self.message.set(search_status_message(
                        &result.query,
                        result.hits.len(),
                        result.truncated,
                    ));
                    self.search_result.set(Some(result));
                }
                Ok(_) => {
                    self.search_result.set(None);
                    self.search_status
                        .set("搜索条件已改变，请重新搜索。".into());
                    self.message.set("搜索条件已改变，请重新搜索。".into());
                }
                Err(error) => {
                    self.search_result.set(None);
                    self.search_status.set(format!("搜索不可用：{error}"));
                    self.message.set(error);
                }
            }
            self.busy.set(false);
            self.select_search_hit(0);
        });
    }
    fn refresh(self) {
        self.refresh_with_annotation(None);
    }
    fn refresh_with_annotation(self, target: Option<pdfmaster_ui::annotation_navigation::Target>) {
        if self.busy.get_untracked() {
            return;
        }
        self.annotation_navigation.update(|nav| {
            nav.begin(target);
        });
        self.busy.set(true);
        spawn_local(async move {
            self.render().await;
            self.busy.set(false);
        });
    }
    fn zoom_by(self, factor: f32) {
        if self.busy.get_untracked() {
            return;
        }
        if self.overview_visible.get_untracked() {
            self.overview_zoom(if factor > 1. { 1.2 } else { 1. / 1.2 });
            return;
        }
        self.zoom.update(|z| *z = step_zoom(*z, factor));
        self.refresh();
    }
    fn set_fit_width(self) {
        let Some(reader) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("pdf-reader"))
        else {
            return;
        };
        let mobile = web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .is_some_and(|w| w <= 720.);
        let shell_width = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("shell-tabs"))
            .map(|e| e.client_width());
        let Some(available) =
            pdfmaster_ui::shell::reader_fit_available(reader.client_width(), shell_width, mobile)
        else {
            return;
        };
        if let Some(zoom) = self.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(self.page.get_untracked()))
                .map(|p| fit_width_zoom(p.width, p.height, p.rotation, available))
        }) {
            self.zoom.set(zoom);
        }
    }
    fn fit_width(self) {
        if self.blocked() {
            return;
        }
        self.close_chrome_popups();
        self.set_fit_width();
        self.refresh();
    }
    fn add_text(self) {
        let text = self.text.get_untracked();
        if text.trim().is_empty() {
            self.message.set("请先输入文字。".into());
            return;
        }
        let (width, height) = self
            .document
            .with_untracked(|d| {
                d.as_ref()
                    .and_then(|d| d.info.pages.get(self.page.get_untracked()))
                    .map(|p| (p.width, p.height))
            })
            .unwrap_or((595., 842.));
        self.edit(json!({"kind":"add_text","page":self.page.get_untracked(),"bounds":{"x":36.,"y":height*0.5,"width":(width-72.).max(20.),"height":60.},"text":text,"font_size":18.,"color":{"r":0.1,"g":0.1,"b":0.1}}));
    }
    fn select_shape(self, shape: Value) {
        if self.blocked() {
            return;
        }
        let b = &shape["bounds"];
        self.shape_bounds.set(format!(
            "{},{},{},{}",
            b["x"], b["y"], b["width"], b["height"]
        ));
        self.shape_stroke
            .set(shape_color_hex(&shape["stroke"]).unwrap_or_default());
        self.shape_fill
            .set(shape_color_hex(&shape["fill"]).unwrap_or_default());
        self.shape_line.set(shape["line_width"].to_string());
        self.selected_shape.set(Some(shape));
        self.message
            .set("已选择图形；参数未应用前不会保存。应用后仍需导出 PDF。".into());
    }
    fn update_shape(self) {
        if self.blocked() {
            return;
        }
        let Some(expected) = self.selected_shape.get_untracked() else {
            return;
        };
        let page = self.page.get_untracked();
        let edit = self.document.with_untracked(|d| {
            d.as_ref().and_then(|d| d.info.pages.get(page)).map(|p| {
                update_shape_edit(
                    page,
                    p.width,
                    p.height,
                    &expected,
                    &self.shape_bounds.get_untracked(),
                    &self.shape_stroke.get_untracked(),
                    &self.shape_fill.get_untracked(),
                    &self.shape_line.get_untracked(),
                )
            })
        });
        match edit {
            Some(Ok(edit)) => self.edit(edit),
            Some(Err(e)) => self.message.set(e),
            None => self.message.set("请先打开 PDF。".into()),
        }
    }
    fn delete_shape(self, expected: Value) {
        if self.blocked() {
            return;
        }
        self.command(
            "delete_shape_document",
            json!({"page":self.page.get_untracked(),"expected":expected}),
        );
    }
    fn add_shape(self, shape: &'static str) {
        if self.blocked() {
            return;
        }
        let page = self.page.get_untracked();
        let edit = self.document.with_untracked(|document| {
            document
                .as_ref()
                .and_then(|d| d.info.pages.get(page))
                .map(|info| {
                    shape_edit(
                        page,
                        info.width,
                        info.height,
                        shape,
                        &self.shape_bounds.get_untracked(),
                        &self.shape_stroke.get_untracked(),
                        &self.shape_fill.get_untracked(),
                        &self.shape_line.get_untracked(),
                    )
                })
        });
        match edit {
            Some(Ok(edit)) => self.edit(edit),
            Some(Err(error)) => self.message.set(error),
            None => self.message.set("请先打开 PDF。".into()),
        }
    }
    fn add_line(self, mode: &'static str) {
        if self.blocked() {
            return;
        }
        let page = self.page.get_untracked();
        let edit = self.document.with_untracked(|d| {
            d.as_ref().and_then(|d| d.info.pages.get(page)).map(|p| {
                line_edit(
                    page,
                    p.width,
                    p.height,
                    &self.line_points.get_untracked(),
                    &self.shape_stroke.get_untracked(),
                    &self.shape_fill.get_untracked(),
                    &self.shape_line.get_untracked(),
                    mode,
                )
            })
        });
        match edit {
            Some(Ok(edit)) => self.edit(edit),
            Some(Err(e)) => self.message.set(e),
            None => self.message.set("请先打开 PDF。".into()),
        }
    }
    fn add_note(self) {
        if self.blocked() {
            return;
        }
        let page = self.page.get_untracked();
        let edit = self.document.with_untracked(|document| {
            document
                .as_ref()
                .and_then(|document| document.info.pages.get(page))
                .map(|info| {
                    note_edit(
                        page,
                        info.width,
                        info.height,
                        info.rotation,
                        &self.note_text.get_untracked(),
                    )
                })
        });
        match edit {
            Some(Ok(edit)) => self.edit(edit),
            Some(Err(error)) => self.message.set(error),
            None => self.message.set("请先打开 PDF。".into()),
        }
    }
    fn delete_note(self, note: TextNote) {
        if self.blocked() {
            return;
        }
        self.command("delete_note_document", json!({"page":self.page.get_untracked(),"annotation_index":note.annotation_index,"expected_contents":note.contents}));
    }
    fn begin_note_edit(self, note: TextNote) {
        if self.blocked() {
            return;
        }
        self.busy.set(true);
        let args = json!({"page":self.page.get_untracked(),"annotation_index":note.annotation_index,"expected_contents":note.contents});
        spawn_local(async move {
            // Expose input only after the native guard acknowledges this snapshot.
            match bridge::call::<NoteDraft>("begin_note_editor", &args).await {
                Ok(editor) => {
                    self.note_edit_text.set(editor.original.contents.clone());
                    self.note_draft.set(Some(editor));
                    self.message
                        .set("编辑便笺中；保存修改后仍需导出 PDF。".into());
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn cancel_note_edit(self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(draft) = self.note_draft.get_untracked() else {
            return;
        };
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Document>("cancel_note_editor", &json!({"id":draft.id})).await {
                Ok(document) => {
                    self.replace_document_view(Some(document));
                    self.note_draft.set(None);
                    self.note_edit_text.set(String::new());
                    self.message
                        .set("已取消便笺编辑；当前 PDF 会话已刷新。".into());
                    self.render().await;
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn save_note(self) {
        if self.busy.get_untracked() || !self.loaded() {
            return;
        }
        let Some(draft) = self.note_draft.get_untracked() else {
            return;
        };
        if draft.page != self.page.get_untracked() {
            self.message
                .set("页面已改变；请取消后重新选择便笺。".into());
            return;
        }
        let contents = self.note_edit_text.get_untracked();
        if let Err(error) = update_note_edit(
            draft.page,
            draft.original.annotation_index,
            &draft.original.contents,
            &contents,
        ) {
            self.message.set(error);
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Document>(
                "commit_note_editor",
                &json!({"id":draft.id,"contents":contents}),
            )
            .await
            {
                Ok(document) => {
                    self.message.set(
                        document
                            .warning
                            .clone()
                            .unwrap_or_else(|| "便笺已修改；请导出副本保存。".into()),
                    );
                    self.replace_document_view(Some(document));
                    self.note_draft.set(None);
                    self.note_edit_text.set(String::new());
                    self.search_result.set(None);
                    self.render().await;
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
}

#[component]
pub fn Workspace() -> impl IntoView {
    let m = Model {
        route: RwSignal::new(Route::Home(HomeSection::Home)),
        chrome: RwSignal::new(Chrome {
            sidebar: !web_sys::window()
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
                .is_some_and(|w| w <= 720.),
            ..Chrome::default()
        }),
        document: RwSignal::new(None),
        tab_presentations: RwSignal::new(std::collections::BTreeMap::new()),
        thumbnail_cache: RwSignal::new(pdfmaster_ui::thumbnails::Cache::default()),
        thumbnail_window: RwSignal::new(pdfmaster_ui::thumbnails::Window::new(0., 600., 0)),
        overview_visible: RwSignal::new(false),
        overview_scale: RwSignal::new(1.),
        overview_selection: RwSignal::new(pdfmaster_ui::overview::Selection::default()),
        overview_layout: RwSignal::new(pdfmaster_ui::overview::Layout::default()),
        overview_window: RwSignal::new(pdfmaster_ui::thumbnails::Window::for_pages(0..0, 0)),
        overview_cache: RwSignal::new(pdfmaster_ui::thumbnails::Cache::default()),
        overview_preparation: RwSignal::new(pdfmaster_ui::outline::ProjectionCache::default()),
        overview_drag: RwSignal::new(None),
        overview_marker: RwSignal::new(None),
        file_drop_request: RwSignal::new(None),
        sidebar_mode: RwSignal::new(pdfmaster_ui::outline::Mode::default()),
        sidebar_width: RwSignal::new(thumbnail_view::restored_sidebar_width()),
        sidebar_viewport: RwSignal::new(thumbnail_view::reader_viewport()),
        outline_cache: RwSignal::new(pdfmaster_ui::outline::Cache::default()),
        outline_selection: RwSignal::new(None),
        annotation_cache: RwSignal::new(pdfmaster_ui::outline::ProjectionCache::default()),
        annotation_selection: RwSignal::new(None),
        annotation_sort: RwSignal::new(pdfmaster_ui::annotations::Sort::default()),
        annotation_copying: RwSignal::new(false),
        annotation_navigation: RwSignal::new(
            pdfmaster_ui::annotation_navigation::Navigation::default(),
        ),
        annotation_image_ticket: RwSignal::new(None),
        page: RwSignal::new(0),
        zoom: RwSignal::new(1.),
        image: RwSignal::new(String::new()),
        message: RwSignal::new(String::new()),
        busy: RwSignal::new(false),
        text: RwSignal::new(String::new()),
        note_text: RwSignal::new(String::new()),
        notes: RwSignal::new(None),
        note_draft: RwSignal::new(None),
        note_edit_text: RwSignal::new(String::new()),
        search_query: RwSignal::new(String::new()),
        search_case_sensitive: RwSignal::new(false),
        search_result: RwSignal::new(None),
        search_selection: RwSignal::new(None),
        search_flow: RwSignal::new(pdfmaster_ui::search_flow::Flow::default()),
        search_timer: RwSignal::new(None),
        search_status: RwSignal::new("输入关键词搜索".into()),
        page_jump: RwSignal::new(String::new()),
        page_range: RwSignal::new(String::new()),
        shape_bounds: RwSignal::new("36,36,120,80".into()),
        shape_stroke: RwSignal::new("#4F46E5".into()),
        shape_fill: RwSignal::new(String::new()),
        shape_line: RwSignal::new("2".into()),
        line_points: RwSignal::new("72,200,300,300".into()),
        shapes: RwSignal::new(None),
        selected_shape: RwSignal::new(None),
        ink_enabled: RwSignal::new(false),
        ink_gesture: RwSignal::new(None),
        ink_handoff: RwSignal::new(false),
        ink_image: RwSignal::new(None),
        ink_image_revision: RwSignal::new(0),
        image_ready: RwSignal::new(false),
    };
    on_cleanup(move || m.stop_search_timer());
    file_drop_view::listen(m);
    Effect::new(move |_| {
        let ready = m.search_flow.with(|flow| flow.can_begin());
        if ready && !m.blocked() && m.route.get() == Route::Document {
            m.search();
        }
    });
    let surface = NodeRef::<leptos::html::Div>::new();
    m.command("document_info", json!({}));
    view! {
        <div class="application-shell">
            <nav class="shell-tabs" id="shell-tabs" aria-label="首页和文档标签">
                <button class="shell-home-tab" class:active=move||matches!(m.route.get(),Route::Home(_)) aria-label="主界面标签" title="主界面" disabled=move||m.busy.get()||m.has_note_draft()||m.has_ink() on:click=move|_|m.switch_to(Route::Home(HomeSection::Home))><img src="32x32.png" alt=""/></button>
                <For each=move||m.document.with(|document|document.as_ref().map(|d|if d.tabs.is_empty(){vec![DocumentTab{id:d.id,name:d.name.clone(),dirty:d.info.dirty}]}else{d.tabs.clone()}).unwrap_or_default()) key=|tab|(tab.id,tab.name.clone(),tab.dirty) children=move|tab|{
                    let id=tab.id;
                    let label=format!("{}{}",tab.name,if tab.dirty{" •"}else{""});
                    let chars:Vec<char>=label.chars().collect();
                    let split=if chars.len()>48{chars.len()-16}else{chars.len()};
                    let prefix:String=chars[..split].iter().collect();
                    let suffix:String=chars[split..].iter().collect();
                    view!{<div class="shell-document-box" class:active=move||m.route.get()==Route::Document&&m.document.with(|d|d.as_ref().is_some_and(|d|d.id==id))>
                        <button class="shell-document-tab" aria-label=format!("文档标签 {}",tab.name) aria-pressed=move||(m.route.get()==Route::Document&&m.document.with(|d|d.as_ref().is_some_and(|d|d.id==id))).to_string() title=label disabled=move||m.blocked() on:click=move|_|m.command("activate_document",json!({"id":id}))><span class="tab-title-prefix">{prefix}</span><span class="tab-title-suffix">{suffix}</span></button>
                        <button class="shell-tab-close" aria-label=format!("关闭 {}",tab.name) title="关闭文档" disabled=move||m.blocked() on:click=move|_|m.close_tab(id)>"×"</button>
                    </div>}
                }/>
                <button class="shell-plus" aria-label="新标签" title="新标签" disabled=move||m.busy.get()||m.has_note_draft()||m.has_ink() on:click=move|_|m.switch_to(Route::Home(HomeSection::Home))>"+"</button>
            </nav>
            <div class="home-host" hidden=move||m.route.get()==Route::Document><home::Dashboard model=m/></div>
        <main class="workspace" id="reader-workspace" style=move||format!("--reader-sidebar-width:{}px",m.sidebar_width.with(|w|w.effective(m.sidebar_viewport.get()))) class:reader-sidebar-hidden=move||!m.chrome.get().sidebar||m.overview_visible.get() class:has-secondary=move||m.chrome.get().mode.is_some() hidden=move||m.route.get()!=Route::Document>
            <workbench::Toolbar model=m/>
            <workbench::SecondaryTools model=m/>
            <thumbnail_view::Sidebar model=m/>
            <thumbnail_view::SidebarResizer model=m/>
            <overview_view::Overview model=m/>
            <section id="pdf-reader" class="reader" aria-label="PDF 阅读区域" aria-busy=move||m.busy.get().to_string() hidden=move||m.overview_visible.get()>
                <Show when=move||m.loaded() fallback=move||view!{
                    <div class="empty-state"><div class="paper-emblem">"PDF"</div><h1>"让文档回到工作中心。"</h1><p>"打开一份 PDF，或从空白页开始。"</p><button class="primary" disabled=move||m.busy.get() on:click=move|_|m.command("open_document",json!({}))>"选择 PDF 文件"</button><div class="safety-note">"仅在会话副本中编辑，原文件不会自动覆盖。"</div></div>
                }>
                    <div class="paper-frame"><Show when=move||!m.image.get().is_empty() fallback=move||view!{<p class="render-placeholder">"等待页面渲染"</p>}>
                        <div class="drawing-surface" class:inking=move||m.ink_enabled.get() node_ref=surface
                            on:pointerdown=move|e|m.pointer_down(e,surface)
                            on:pointermove=move|e|{m.pointer_sample(&e,surface);}
                            on:pointerup=move|e|m.pointer_up(e,surface)
                            on:pointercancel=move|e|m.pointer_cancel(e)
                            on:lostpointercapture=move|e|m.pointer_cancel(e)>
                            <For each=move||vec![(m.image.get(),m.annotation_image_ticket.get())] key=|(source,owner)|(source.clone(),*owner) children=move|(source,owner)|{
                                let failed_source=source.clone();
                                view!{<img id="pdf-page-image" class="pdf-page" src=source data-annotation-owner=owner.map(|t|t.to_string()).unwrap_or_default() style=move||m.raster().map(|p|p.image_style()).unwrap_or_default() alt=move||format!("PDF 第 {} 页",m.page.get()+1) draggable="false" on:load=move|e|{if image_event_is_current(&e){m.image_loaded(image_event_source(&e),owner);}} on:error=move|e|{if image_event_is_current(&e){m.page_image_failed(owner, &failed_source);}}/>}
                            }/>
                            {move||m.ink_image.get().filter(|h|!h.decoded).map(|h|{let revision=h.revision;view!{<img class="ink-staged-image" src=h.next alt="" aria-hidden="true" on:load=move|_|m.ink_image_decoded(revision) on:error=move|_|m.ink_image_failed(revision)/>}})}
                            {move||m.ink_gesture.with(|s|s.as_ref().filter(|s|s.guard.is_some()&&!s.cancelled).map(|s|{
                                let(w,h)=if s.rotation.rem_euclid(180)==0{(s.page_width,s.page_height)}else{(s.page_height,s.page_width)};
                                view!{<svg class="ink-preview" viewBox=format!("0 0 {w} {h}") preserveAspectRatio="none" aria-hidden="true"><polyline points=s.preview() fill="none" stroke=s.stroke.clone() stroke-width=s.width stroke-linecap="round" stroke-linejoin="round"/></svg>}
                            }))}
                        </div>
                    </Show></div>
                </Show>
            </section>
            <aside class="inspector" aria-label="当前工具参数" hidden=move||m.chrome.get().panel.is_none()>
                <div class="inspector-heading"><h2>"工具参数"</h2><button aria-label="关闭工具参数" disabled=move||m.blocked() on:click=move|_|m.select_panel(None)>"×"</button></div>
                <workbench::FileActions model=m/>
                <div class="tool-group" id="tool-pages" hidden=move||m.chrome.get().panel!=Some(Panel::Pages)><h3>"当前页操作"</h3>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"rotate","page":m.page.get_untracked(),"degrees":90}))>"顺时针旋转 90°"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"duplicate_page","page":m.page.get_untracked(),"index":m.page.get_untracked()+1}))>"复制当前页"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.run_strip_action(Action::InsertBlank)>"在后方插入空白页"</button>
                    <button disabled=move||m.blocked()||m.page.get()==0 on:click=move|_|m.move_page(-1)>"当前页前移"</button>
                    <button disabled={move||m.blocked()||m.page.get()+1>=m.count()} on:click=move|_|m.move_page(1)>"当前页后移"</button>
                    <button class="danger" disabled={move || m.blocked()||m.count()<=1} on:click=move|_|m.delete_current_page()>"删除当前页"</button>
                </div>
                <div class="tool-group" id="tool-batch" hidden=move||m.chrome.get().panel!=Some(Panel::Batch)><h3>"批量页面"</h3>
                    <input aria-label="批量页码范围" disabled=move||m.busy.get() placeholder="例如 1-3,5" prop:value=move||m.page_range.get() on:input=move|e|m.page_range.set(event_target_value(&e))/>
                    <button disabled=move||m.blocked() on:click=move|_|m.batch_pages("rotate_90")>"批量顺时针旋转 90°"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.command("extract_pages_document",json!({"range":m.page_range.get_untracked()}))>"提取页面到新 PDF"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.run_strip_action(Action::InsertPdf)>"在当前页后插入 PDF"</button>
                    <button disabled=move||m.blocked() on:click=move|_|{if let Some(index)=m.page_insertion_index(){m.command_at_page("insert_pdfs_document",json!({"index":index}),Some(index));}}>"批量插入 PDF（整批可撤销）"</button>
                    <button class="danger" disabled=move||m.blocked()||m.count()<=1 on:click=move|_|m.batch_pages("delete")>"批量删除"</button>
                    <p class="muted">"页码从 1 开始；删除会先显示选中页数并要求确认。"</p>
                </div>
                <div class="tool-group" id="tool-text" hidden=move||m.chrome.get().panel!=Some(Panel::Text)><h3>"文字批注"</h3><textarea aria-label="文字批注内容" placeholder="输入要添加的文字" prop:value=move||m.text.get() on:input=move|e|m.text.set(event_target_value(&e))></textarea><button disabled=move||m.blocked() on:click=move|_|m.add_text()>"添加到页面中部"</button><p class="muted">"字体与复杂 PDF 兼容性仍在迁移验证中。"</p></div>
                <div class="tool-group notes-panel" id="tool-notes" hidden=move||m.chrome.get().panel!=Some(Panel::Notes)><h3>"便笺与评论"</h3>
                    <textarea aria-label="便笺内容" disabled=move||m.busy.get() placeholder="支持中文和 emoji；保存为 PDF 便笺" prop:value=move||m.note_text.get() on:input=move|event|m.note_text.set(event_target_value(&event))></textarea>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_note()>"添加便笺到左上方"</button>
                    <div class="note-list" aria-label="当前页便笺">
                        {move||match m.notes.get() {
                            None => view!{<p class="note-meta">{if m.loaded(){"正在读取当前页便笺…"}else{"打开 PDF 后显示当前页便笺。"}}</p>}.into_any(),
                            Some(Err(error)) => view!{<p class="note-error">{format!("无法读取批注：{error}")}</p>}.into_any(),
                            Some(Ok(list)) => view!{
                                <p class="note-meta">{format!("当前页 {} 条便笺",list.notes.len())}</p>
                                <p class="note-meta">{if list.unsupported_count>0{format!("另有 {} 条其他类型批注未列入便笺列表；页面外观取决于渲染兼容性。",list.unsupported_count)}else{String::new()}}</p>
                                {list.notes.into_iter().map(|note| {let edit_note=note.clone();view!{
                                    <article class="note-item"><strong>{format!("便笺 {}",note.annotation_index+1)}</strong><p>{note.contents.clone()}</p><small>{format!("PDF 坐标 {:.0}, {:.0} · {:.0} × {:.0} pt",note.bounds.x,note.bounds.y,note.bounds.width,note.bounds.height)}</small><button disabled=move||m.blocked() on:click=move|_|m.begin_note_edit(edit_note.clone())>"编辑便笺"</button><button class="danger" disabled=move||m.blocked() on:click=move|_|m.delete_note(note.clone())>"删除便笺…"</button></article>
                                }}).collect_view()}
                            }.into_any(),
                        }}
                    </div>
                    <Show when=move||m.has_note_draft()>
                        <div class="note-editor" aria-label="便笺编辑">
                            <textarea aria-label="编辑便笺内容" disabled=move||m.busy.get() prop:value=move||m.note_edit_text.get() on:input=move|event|m.note_edit_text.set(event_target_value(&event))></textarea>
                            <button disabled=move||m.busy.get() on:click=move|_|m.save_note()>"保存便笺修改"</button>
                            <button disabled=move||m.busy.get() on:click=move|_|m.cancel_note_edit()>"取消便笺编辑"</button>
                            <p class="note-meta">"编辑期间阻止正常关闭；输入未自动持久化，崩溃或强制退出仍可能丢失。保存后需导出 PDF。"</p>
                        </div>
                    </Show>
                    <Show when=move||m.loaded()&&!m.has_note_draft()>
                        <button disabled=move||m.busy.get() on:click=move|_|m.command("document_info",json!({}))>"同步便笺编辑状态"</button>
                        <p class="note-meta">"编辑确认未返回时，可恢复当前进程的编辑入口；不会恢复未持久化的输入。"</p>
                    </Show>
                    <p class="muted">"便笺文字保存为 Unicode 元数据；图标不是正文文字。仅支持受限 Text 便笺；标签、回复线程及歧义弹窗关系暂不编辑或删除。关系检查最多 16384 条。定位和弹窗显示仍在迁移；修改和删除均不是安全擦除。"</p>
                </div>
                <div class="tool-group ink-panel" id="tool-ink" hidden=move||m.chrome.get().panel!=Some(Panel::Ink)><h3>"手绘"</h3>
                    <label class="shape-label">"笔迹颜色 #RRGGBB"<input aria-label="笔迹颜色" disabled=move||m.blocked() prop:value=move||m.shape_stroke.get() on:input=move|e|m.shape_stroke.set(event_target_value(&e))/></label>
                    <label class="shape-label">"笔迹线宽 (pt)"<input aria-label="笔迹线宽" inputmode="decimal" disabled=move||m.blocked() prop:value=move||m.shape_line.get() on:input=move|e|m.shape_line.set(event_target_value(&e))/></label>
                    <button disabled=move||m.blocked() aria-pressed=move||m.ink_enabled.get().to_string() on:click=move|_|m.ink_enabled.update(|v|*v = !*v)>{move||if m.ink_enabled.get(){"退出手绘模式"}else{"进入手绘模式"}}</button>
                    <p class="note-meta">{move||if m.ink_enabled.get(){"在页面上拖动绘制；抬笔后提交一笔。阅读滚动请先退出手绘模式。"}else{"可使用鼠标、触控或笔绘制。上方笔迹颜色与线宽在落笔时固定；当前仍与图形默认参数共享，独立样式待迁移。"}}</p>
                    <Show when=move||m.has_ink()>
                        <button disabled=move||m.busy.get()||!m.ink_gesture.with(|s|s.as_ref().is_some_and(|s|s.ended&&s.guard.is_some()&&!s.committing&&!s.cancelled)) on:click=move|_|m.retry_ink()>"重试提交手绘"</button>
                    </Show>
                    <button disabled=move||m.busy.get()||!m.loaded() on:click=move|_|m.cancel_ink()>"取消／同步手绘"</button>
                    <p class="muted">"手绘中阻止正常关闭及冲突操作；采样点在提交前未持久化，崩溃或强制退出仍可能丢失。当前仅迁移添加与整笔撤销，选取、改样式与橡皮擦仍待迁移。"</p>
                </div>
                <div class="tool-group shape-panel" hidden=move||m.chrome.get().panel!=Some(Panel::Shapes)><h3>"图形批注"</h3>
                    <div class="note-list" aria-label="当前页图形批注">{move||match m.shapes.get() {
                        None=>view!{<p class="note-meta">"打开 PDF 后显示当前页受支持样式的图形。"</p>}.into_any(),
                        Some(Err(e))=>view!{<p class="note-error">{format!("无法读取图形：{e}")}</p>}.into_any(),
                        Some(Ok(list))=>view!{
                            <p class="note-meta">{format!("当前页 {} 个受支持样式的图形；{} 条其他或暂不支持样式的批注未列入。标签、回复和弹窗关系仍需操作时检查。",list.shapes.len(),list.unsupported_count)}</p>
                            {list.shapes.into_iter().map(|shape|{let selected=shape.clone();view!{
                                <article class="note-item"><strong>{format!("{} · 批注 {}",match shape["shape"].as_str(){Some("line")=>"直线／箭头",Some("ellipse")=>"椭圆",_=>"矩形"},shape["annotation_index"].as_u64().unwrap_or(0)+1)}</strong>
                                <button disabled=move||m.blocked() on:click=move|_|m.select_shape(selected.clone())>"选择图形参数"</button>
                                <button class="danger" disabled=move||m.blocked() on:click=move|_|m.delete_shape(shape.clone())>"删除图形…"</button></article>
                            }}).collect_view()}
                        }.into_any(),
                    }}</div>
                    <label class="shape-label">"范围 x,y,宽,高 (pt)"<input aria-label="图形范围 x,y,宽,高" disabled=move||m.blocked() prop:value=move||m.shape_bounds.get() on:input=move|e|m.shape_bounds.set(event_target_value(&e))/></label>
                    <label class="shape-label">"描边 #RRGGBB"<input aria-label="图形描边颜色" disabled=move||m.blocked() prop:value=move||m.shape_stroke.get() on:input=move|e|m.shape_stroke.set(event_target_value(&e))/></label>
                    <label class="shape-label">"填充（留空透明）"<input aria-label="图形填充颜色" disabled=move||m.blocked() prop:value=move||m.shape_fill.get() on:input=move|e|m.shape_fill.set(event_target_value(&e))/></label>
                    <label class="shape-label">"线宽 (pt)"<input aria-label="图形线宽" inputmode="decimal" disabled=move||m.blocked() prop:value=move||m.shape_line.get() on:input=move|e|m.shape_line.set(event_target_value(&e))/></label>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_shape("rectangle")>"添加矩形批注"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_shape("ellipse")>"添加椭圆批注"</button>
                    <label class="shape-label">"直线端点 x1,y1,x2,y2 (pt)"<input aria-label="直线端点 x1,y1,x2,y2" disabled=move||m.blocked() prop:value=move||m.line_points.get() on:input=move|e|m.line_points.set(event_target_value(&e))/></label>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_line("line")>"添加直线批注"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_line("arrow")>"添加单箭头批注"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_line("double_arrow")>"添加双箭头批注"</button>
                    <p class="note-meta">"新线使用独立端点和上方颜色／线宽（0.25–32 pt）。选中已有线后修改范围会按比例移动／缩放原端点，保留端点样式与虚线；外观范围必要时自动扩展以容纳粗线／箭头，页边仍按页面裁切。新线端点输入不用于修改已有线。"</p>
                    <button disabled=move||m.blocked()||m.selected_shape.get().is_none() on:click=move|_|m.update_shape()>"应用图形修改"</button>
                    <button disabled=move||m.blocked()||m.selected_shape.get().is_none() on:click=move|_|m.selected_shape.set(None)>"取消图形选择"</button>
                    <p class="shape-label">"原始页面坐标，左下角为原点；旋转页也使用未旋转坐标。参数未应用前不保存，切页或重新渲染后需重新选择。独立批注可撤销；修改和删除不是安全擦除。标签、回复、特殊样式暂不修改；画布拖拽和手绘仍在迁移。"</p>
                </div>
                <div class="migration-note"><h3>"完整迁移进行中"</h3><p>"本候选用于验证 Rust 文档核心。画布、AI、OCR 和完整编辑工具尚未迁移，不会显示为已完成。"</p></div>
            </aside>
            <footer class="statusbar"><span role="status" aria-live="polite">{move||m.document.with(|d|visible_status(d.as_ref().and_then(|d|d.warning.as_deref()), &m.message.get(), m.busy.get()))}</span><span class="flex"></span><Show when=move||m.file_drop_request.get().is_some()><button aria-label="取消文件导入" on:click=move|_|m.cancel_file_import()>"取消文件导入"</button></Show><Show when=move||m.loaded()><button aria-label="上一页" disabled=move||m.blocked()||m.page.get()==0 on:click=move|_|m.navigate(adjacent_page(m.page.get_untracked(),-1,m.count()))>"‹"</button><span>{move||format!("{} / {}",m.page.get()+1,m.count())}</span><button aria-label="下一页" disabled={move || m.blocked()||m.page.get()+1>=m.count()} on:click=move|_|m.navigate(adjacent_page(m.page.get_untracked(),1,m.count()))>"›"</button><label class="page-jump-label"><span>"页码"</span><input aria-label="跳转页码" inputmode="numeric" prop:value=move||m.page_jump.get() on:input=move|e|m.page_jump.set(event_target_value(&e))/></label><button class="page-jump-button" disabled=move||m.blocked() on:click=move|_|m.jump_to_page()>"转到"</button><span class="page-dimensions">{move||m.document.with(|d|d.as_ref().and_then(|d|d.info.pages.get(m.page.get())).map(|p|format!("{:.0} × {:.0} pt · {}°",p.width,p.height,p.rotation)).unwrap_or_default())}</span></Show></footer>
        </main>
        </div>
    }
}
