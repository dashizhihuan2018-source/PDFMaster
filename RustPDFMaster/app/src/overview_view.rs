use super::{Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    overview::{Layout, Move, Selection, scale},
    shell::Route,
    thumbnails::{Image, Window},
    workbench::Mode,
};
use serde_json::json;
use wasm_bindgen::JsCast;

#[derive(Clone)]
pub(super) struct Pointer {
    id: i32,
    token: u32,
    selection: Selection,
    page: usize,
    plain: bool,
    x: f64,
    y: f64,
    last_y: f64,
    time: f64,
    touch: bool,
    dragging: bool,
    scrolling: bool,
}

fn element() -> Option<web_sys::HtmlElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id("pdf-overview")?
        .dyn_into()
        .ok()
}
fn focus() {
    if let Some(grid) = element() {
        let _ = grid.focus();
    }
}

impl Model {
    fn overview_open_at(self, event: web_sys::MouseEvent) {
        if self.blocked() {
            return;
        }
        let Some(grid) = element() else {
            return;
        };
        let bounds = grid.get_bounding_client_rect();
        let x = f64::from(event.client_x()) - bounds.left() + f64::from(grid.scroll_left());
        let y = f64::from(event.client_y()) - bounds.top() + f64::from(grid.scroll_top());
        let page = self.overview_layout.with_untracked(|l| {
            l.tiles
                .iter()
                .find(|t| {
                    x >= t.left && x <= t.left + t.width && y >= t.top && y <= t.top + t.height
                })
                .map(|t| t.page)
        });
        if let Some(page) = page {
            self.set_overview(false);
            self.navigate(page);
        }
    }
    fn overview_token(self) -> Option<u32> {
        self.overview_preparation.with_untracked(|cache| {
            cache
                .result()
                .and_then(|r| r.as_ref().ok())
                .and_then(|v| v.first().copied())
        })
    }
    pub(super) fn set_overview(self, visible: bool) {
        if self.blocked() {
            return;
        }
        self.overview_drag.set(None);
        self.overview_marker.set(None);
        self.overview_visible.set(visible);
        self.ink_enabled.set(false);
        self.close_chrome_popups();
        self.chrome.update(|c| {
            c.panel = None;
            c.mode = visible.then_some(Mode::Pages);
        });
        if visible {
            self.overview_selection
                .update(|s| s.reset(self.page.get_untracked(), self.count()));
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                self.measure_overview(true);
                focus();
            });
        }
    }
    pub(super) fn measure_overview(self, reveal: bool) {
        if !self.overview_visible.get_untracked() {
            return;
        }
        let Some(grid) = element() else {
            return;
        };
        let pages = self.document.with_untracked(|d| {
            d.as_ref()
                .map(|d| {
                    d.info
                        .pages
                        .iter()
                        .map(|p| (f64::from(p.width), f64::from(p.height), p.rotation))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        let layout = Layout::new(
            f64::from(grid.client_width()),
            self.overview_scale.get_untracked(),
            &pages,
        );
        if self.overview_layout.with_untracked(|old| old != &layout) {
            self.overview_layout.set(layout.clone());
            self.overview_drag.set(None);
            self.overview_marker.set(None);
        }
        if reveal
            && let Some(page) = self.overview_selection.with_untracked(|s| s.current())
            && let Some(tile) = layout.tiles.get(page)
        {
            let next = pdfmaster_ui::reveal_scroll(
                f64::from(grid.scroll_top()),
                tile.top,
                tile.top + tile.height,
                f64::from(grid.client_height()),
            );
            grid.set_scroll_top(next as i32);
        }
        let window = Window::for_pages(
            layout.visible(
                f64::from(grid.scroll_top()),
                f64::from(grid.client_height()),
            ),
            self.count(),
        );
        if self.overview_window.get_untracked() != window {
            self.overview_window.set(window);
        }
    }
    pub(super) fn overview_zoom(self, factor: f64) {
        if self.blocked() {
            return;
        }
        self.overview_scale.update(|s| *s = scale(*s * factor));
        self.overview_cache.update(|cache| {
            cache.invalidate();
        });
        self.measure_overview(true);
    }
    fn prepare_overview(self) {
        let mut ticket = None;
        self.overview_preparation.update(|c| ticket = c.begin());
        let Some(ticket) = ticket else {
            return;
        };
        spawn_local(async move {
            let result = bridge::call_bounded::<u32>("prepare_page_overview", &json!({})).await;
            self.overview_preparation.update(|cache| {
                if result
                    .as_ref()
                    .is_err_and(|e| e.starts_with("原生响应超时；"))
                {
                    cache.suspend(ticket);
                } else {
                    cache.finish(ticket, result.map(|id| vec![id]));
                }
            });
        });
    }
    fn load_overview_image(self, page: usize, window: Window, token: u32) {
        let mut ticket = None;
        self.overview_cache
            .update(|c| ticket = c.begin(page, window));
        let Some(ticket) = ticket else {
            return;
        };
        let desired = self
            .overview_layout
            .with_untracked(|l| l.tiles.get(page).map(|t| t.width))
            .unwrap_or(180.);
        let raster_scale = self.document.with_untracked(|d| {
            d.as_ref().and_then(|d| d.info.pages.get(page)).map(|p| {
                let (w, h) = if p.rotation.rem_euclid(180) == 90 {
                    (p.height, p.width)
                } else {
                    (p.width, p.height)
                };
                let density = web_sys::window()
                    .map(|w| w.device_pixel_ratio())
                    .unwrap_or(1.)
                    .clamp(1., 2.);
                ((desired * density / f64::from(w))
                    .min(1024. / f64::from(w.max(h)))
                    .min(3.) as f32)
                    .max(0.001)
            })
        });
        spawn_local(async move {
            let mut result = Err("页面尺寸无效，暂无法预览。".into());
            if let Some(mut scale) = raster_scale {
                // Sequential bounded retries only for oversized previews; no
                // replacement request after a timeout or stale-document error.
                for _ in 0..3 {
                    result = bridge::call_bounded::<String>(
                        "render_overview_page",
                        &json!({"token":token,"page":page,"scale":scale}),
                    )
                    .await;
                    if !result.as_ref().is_ok_and(|image| image.len() > 262144) {
                        break;
                    }
                    scale *= 0.5;
                }
            }
            let current = self.overview_window.get_untracked();
            self.overview_cache.update(|c| {
                if result
                    .as_ref()
                    .is_err_and(|e| e.starts_with("原生响应超时；"))
                {
                    c.suspend(ticket);
                } else {
                    c.finish(ticket, current, result);
                }
            });
        });
    }
    fn overview_down(self, event: web_sys::PointerEvent) {
        if self.blocked()
            || event.button() != 0
            || self.overview_drag.with_untracked(|d| d.is_some())
        {
            return;
        }
        let Some(token) = self.overview_token() else {
            return;
        };
        let page = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|t| t.closest(".overview-tile").ok().flatten())
            .and_then(|t| t.get_attribute("data-page"))
            .and_then(|p| p.parse::<usize>().ok());
        let Some(page) = page.filter(|&p| p < self.count()) else {
            return;
        };
        let Some(grid) = element() else {
            return;
        };
        if grid.set_pointer_capture(event.pointer_id()).is_err() {
            return;
        }
        event.prevent_default();
        focus();
        let shift = event.shift_key();
        let toggle = event.meta_key() || event.ctrl_key();
        let plain =
            self.overview_selection.with_untracked(|s| s.contains(page)) && !shift && !toggle;
        if !plain {
            self.overview_selection
                .update(|s| s.click(page, self.count(), shift, toggle));
        }
        self.overview_drag.set(Some(Pointer {
            id: event.pointer_id(),
            token,
            selection: self.overview_selection.get_untracked(),
            page,
            plain,
            x: f64::from(event.client_x()),
            y: f64::from(event.client_y()),
            last_y: f64::from(event.client_y()),
            time: event.time_stamp(),
            touch: event.pointer_type() == "touch",
            dragging: false,
            scrolling: false,
        }));
    }
    fn overview_sample(self, event: &web_sys::PointerEvent) {
        let Some(mut drag) = self
            .overview_drag
            .get_untracked()
            .filter(|d| d.id == event.pointer_id())
        else {
            return;
        };
        if self.blocked() || self.overview_token() != Some(drag.token) {
            self.overview_cancel(event.pointer_id());
            return;
        }
        let Some(grid) = element() else {
            return;
        };
        let x = f64::from(event.client_x());
        let y = f64::from(event.client_y());
        let moved = (x - drag.x).hypot(y - drag.y) > 6.;
        if drag.touch
            && !drag.dragging
            && !drag.scrolling
            && moved
            && event.time_stamp() - drag.time < 350.
        {
            drag.scrolling = true;
        }
        if drag.scrolling {
            grid.set_scroll_top((f64::from(grid.scroll_top()) + drag.last_y - y) as i32);
            drag.last_y = y;
            self.overview_drag.set(Some(drag));
            self.measure_overview(false);
            return;
        }
        if moved {
            drag.dragging = true;
        }
        if drag.dragging {
            let bounds = grid.get_bounding_client_rect();
            if y < bounds.top() + 24. {
                grid.set_scroll_top(grid.scroll_top().saturating_sub(18));
            } else if y > bounds.bottom() - 24. {
                grid.set_scroll_top(grid.scroll_top().saturating_add(18));
            }
            self.measure_overview(false);
            let marker = self.overview_layout.with_untracked(|l| {
                l.insertion(
                    x - bounds.left() + f64::from(grid.scroll_left()),
                    y - bounds.top() + f64::from(grid.scroll_top()),
                )
            });
            self.overview_marker.set(marker);
        }
        self.overview_drag.set(Some(drag));
    }
    fn overview_up(self, event: web_sys::PointerEvent) {
        let Some(drag) = self
            .overview_drag
            .get_untracked()
            .filter(|d| d.id == event.pointer_id())
        else {
            return;
        };
        self.overview_sample(&event);
        let Some(current) = self.overview_drag.get_untracked() else {
            return;
        };
        let plan = if current.dragging {
            self.overview_marker
                .get_untracked()
                .and_then(|i| current.selection.movement(i, self.count()))
        } else {
            None
        };
        if current.plain && !current.dragging && !current.scrolling {
            self.overview_selection
                .update(|s| s.click(current.page, self.count(), false, false));
        }
        self.overview_cancel(drag.id);
        if let Some(plan) = plan {
            self.move_overview(plan, drag.token);
        }
    }
    fn overview_cancel(self, id: i32) {
        if self
            .overview_drag
            .with_untracked(|d| d.as_ref().is_some_and(|d| d.id == id))
        {
            self.overview_drag.set(None);
            self.overview_marker.set(None);
            if let Some(grid) = element() {
                let _ = grid.release_pointer_capture(id);
            }
        }
    }
    fn move_overview(self, plan: Move, token: u32) {
        if self.blocked() || self.overview_token() != Some(token) {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Option<super::Document>>(
                "move_overview_pages",
                &json!({"token":token,"pages":plan.pages,"insertion":plan.insertion}),
            )
            .await
            {
                Ok(Some(document)) => {
                    let warning = document.warning.clone();
                    self.page.set(plan.selected.start);
                    self.page_jump.set((plan.selected.start + 1).to_string());
                    self.replace_document_view(Some(document));
                    self.overview_selection.update(|s| {
                        s.restore(&plan.selected.clone().collect::<Vec<_>>(), self.count())
                    });
                    self.message.set(
                        warning.unwrap_or_else(|| {
                            format!("已移动 {} 页，可撤销。", plan.selected.len())
                        }),
                    );
                    self.render().await;
                    leptos::leptos_dom::helpers::request_animation_frame(move || {
                        self.measure_overview(true);
                        focus();
                    });
                }
                Ok(None) => self.message.set("移动未执行，文档保持不变。".into()),
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    pub(super) fn overview_batch(self, operation: &'static str) {
        let pages = self.overview_selection.with_untracked(|s| s.pages());
        if pages.is_empty() {
            return;
        }
        let mut ranges = Vec::new();
        let mut first = pages[0];
        let mut last = first;
        for page in pages.into_iter().skip(1).chain(std::iter::once(usize::MAX)) {
            if page == last + 1 {
                last = page;
                continue;
            }
            ranges.push(if first == last {
                (first + 1).to_string()
            } else {
                format!("{}-{}", first + 1, last + 1)
            });
            first = page;
            last = page;
        }
        self.batch_range_with_selection(
            ranges.join(","),
            operation,
            Some(self.overview_selection.get_untracked()),
        );
    }
    fn overview_key(self, event: web_sys::KeyboardEvent) {
        if self.blocked()
            || event
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .is_none_or(|t| t.id() != "pdf-overview")
        {
            return;
        }
        let modifier = event.ctrl_key() || event.meta_key();
        if modifier && event.key().eq_ignore_ascii_case("a") && !event.alt_key() {
            event.prevent_default();
            self.overview_selection
                .update(|s| s.restore(&(0..self.count()).collect::<Vec<_>>(), self.count()));
            return;
        }
        if modifier || event.alt_key() {
            return;
        }
        match event.key().as_str() {
            "Delete" | "Backspace" if !event.shift_key() && !event.repeat() => {
                event.prevent_default();
                self.overview_batch("delete");
            }
            "Enter" => {
                event.prevent_default();
                if let Some(page) = self.overview_selection.with_untracked(|s| s.current()) {
                    self.set_overview(false);
                    self.navigate(page);
                }
            }
            "Escape" => {
                event.prevent_default();
                self.set_overview(false);
            }
            key => {
                let current = self
                    .overview_selection
                    .with_untracked(|s| s.current())
                    .unwrap_or(0);
                let columns = self.overview_layout.with_untracked(|l| l.columns.max(1));
                let page = match key {
                    "ArrowLeft" => current.saturating_sub(1),
                    "ArrowRight" => current.saturating_add(1),
                    "ArrowUp" => current.saturating_sub(columns),
                    "ArrowDown" => current.saturating_add(columns),
                    "Home" => 0,
                    "End" => self.count().saturating_sub(1),
                    _ => return,
                };
                event.prevent_default();
                self.overview_selection.update(|s| {
                    s.click(
                        page.min(self.count().saturating_sub(1)),
                        self.count(),
                        event.shift_key(),
                        false,
                    )
                });
                self.measure_overview(true);
            }
        }
    }
}

#[component]
pub(super) fn Overview(model: Model) -> impl IntoView {
    let m = model;
    let resize =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::resize, move |_| {
            m.measure_overview(false)
        });
    let blur = leptos::leptos_dom::helpers::window_event_listener(leptos::ev::blur, move |_| {
        if let Some(d) = m.overview_drag.get_untracked() {
            m.overview_cancel(d.id);
        }
    });
    on_cleanup(move || {
        resize.remove();
        blur.remove();
    });
    Effect::new(move |_| {
        let _ = m.document.get();
        if m.overview_visible.get() && m.route.get() == Route::Document {
            leptos::leptos_dom::helpers::request_animation_frame(move || m.measure_overview(false));
        }
    });
    Effect::new(move |_| {
        if m.overview_visible.get()
            && m.route.get() == Route::Document
            && !m.busy.get()
            && m.loaded()
            && m.overview_preparation.with(|c| c.can_begin())
        {
            m.prepare_overview();
        }
    });
    Effect::new(move |_| {
        let token = m.overview_preparation.with(|c| {
            c.result()
                .and_then(|r| r.as_ref().ok())
                .and_then(|v| v.first().copied())
        });
        let window = m.overview_window.get();
        if m.overview_visible.get()
            && m.route.get() == Route::Document
            && !m.busy.get()
            && let Some(token) = token
            && let Some(page) = m.overview_cache.with(|c| c.next(window))
        {
            m.load_overview_image(page, window, token);
        }
    });
    view! {
        <section id="pdf-overview" class="page-overview" role="listbox" aria-label="PDF 整页缩略图网格" aria-multiselectable="true" aria-busy=move||m.busy.get().to_string() tabindex="0" hidden=move||!m.overview_visible.get()
            on:scroll=move|_|m.measure_overview(false) on:keydown=move|e|m.overview_key(e) on:dblclick=move|e|m.overview_open_at(e)
            on:pointerdown=move|e|m.overview_down(e) on:pointermove=move|e|m.overview_sample(&e) on:pointerup=move|e|m.overview_up(e)
            on:pointercancel=move|e|m.overview_cancel(e.pointer_id()) on:lostpointercapture=move|e|m.overview_cancel(e.pointer_id())
            on:dragover=move|e|e.prevent_default() on:drop=move|e|{e.prevent_default();m.message.set("外部文件拖入总览尚未迁移；请使用插入 PDF。".into());}>
            <div class="overview-content" style=move||format!("height:{}px",m.overview_layout.with(|l|l.height))>
                <For each=move||{let window=m.overview_window.get();m.overview_layout.with(|l|l.tiles.get(window.rows()).unwrap_or_default().to_vec())} key=|t|(t.page,t.width.to_bits(),t.height.to_bits(),t.top.to_bits()) children=move|tile|{
                    let page=tile.page;
                    view!{<div class="overview-tile" role="option" data-page=page aria-label=format!("第 {} 页",page+1) aria-selected=move||m.overview_selection.with(|s|s.contains(page)).to_string()
                        class:selected=move||m.overview_selection.with(|s|s.contains(page)) style=format!("left:{}px;top:{}px;width:{}px;height:{}px",tile.left,tile.top,tile.width,tile.height)
                        >
                        <span class="overview-image">{move||m.overview_cache.with(|c|match c.image(page){
                            Some(Image::Ready(source))=>view!{<img src=source.clone() alt="" draggable="false"/>}.into_any(),
                            Some(Image::Failed(message))=>{let message=message.clone();view!{<button class="overview-retry" title=message aria-label=format!("重试第 {} 页总览图",page+1) disabled=move||m.blocked()||m.overview_cache.with(|c|c.suspended()) on:pointerdown=move|e|e.stop_propagation() on:click=move|_|m.overview_cache.update(|c|{c.retry(page);})>"重试预览"</button>}.into_any()},
                            None=>view!{<span class="overview-placeholder">"加载中"</span>}.into_any()
                        })}</span>
                        <span class="overview-page-number">{page+1}</span>
                    </div>}
                }/>
                {move||m.overview_marker.get().and_then(|i|m.overview_layout.with(|l|{
                    let last=i==l.tiles.len();let tile=if last{l.tiles.last()}else{l.tiles.get(i)}?;
                    Some(view!{<div class="overview-insertion" style=format!("left:{}px;top:{}px;height:{}px",if last{tile.left+tile.width+8.}else{tile.left-8.},tile.top,tile.height) aria-hidden="true"/>})
                }))}
            </div>
            <Show when=move||m.overview_preparation.with(|c|c.suspended())||m.overview_cache.with(|c|c.suspended())><p class="overview-state" role="status">"总览预览已暂停；请先保存文档，再正常重启候选。"</p></Show>
            {move||m.overview_preparation.with(|c|c.result().and_then(|r|r.as_ref().err()).cloned()).map(|error|view!{<p class="overview-state" role="status">{error}<button disabled=move||m.blocked() on:click=move|_|m.overview_preparation.update(|c|c.invalidate())>"重新读取总览"</button></p>})}
        </section>
    }
}
