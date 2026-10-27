use super::annotation_view::AnnotationSidebar;
use super::{Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::sidebar_width::{STORAGE_KEY, Width};
use pdfmaster_ui::{
    outline::{Mode, Row},
    shell::Route,
    thumbnails::{Image, ROW_HEIGHT, Window, thumbnail_scale},
};
use serde_json::json;
use wasm_bindgen::JsCast;

pub(super) fn reader_viewport() -> f64 {
    let window = web_sys::window();
    window
        .as_ref()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("reader-workspace"))
        .map(|e| f64::from(e.client_width()))
        .filter(|w| *w > 0.)
        .or_else(|| {
            window
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
        })
        .unwrap_or(360.)
}

pub(super) fn restored_sidebar_width() -> Width {
    let saved = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(STORAGE_KEY).ok().flatten());
    Width::restore(saved.as_deref())
}

impl Model {
    fn refresh_sidebar_viewport(self) -> f64 {
        let viewport = reader_viewport();
        self.sidebar_viewport.set(viewport);
        viewport
    }

    fn save_sidebar_width(self) {
        let saved = self.sidebar_width.with_untracked(|width| width.saved());
        let stored = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .is_some_and(|storage| storage.set_item(STORAGE_KEY, &saved).is_ok());
        if !stored {
            self.message
                .set("侧栏宽度已调整；本机设置未能保存。".into());
        }
    }
}

#[component]
pub(super) fn SidebarResizer(model: Model) -> impl IntoView {
    let m = model;
    let handle = NodeRef::<leptos::html::Div>::new();
    let resize =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::resize, move |_| {
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                m.refresh_sidebar_viewport();
            });
        });
    let blur = leptos::leptos_dom::helpers::window_event_listener(leptos::ev::blur, move |_| {
        m.sidebar_width.update(|w| w.cancel_active());
    });
    on_cleanup(move || {
        resize.remove();
        blur.remove();
    });
    Effect::new(move |_| {
        if !m.chrome.get().sidebar || m.route.get() != Route::Document || m.blocked() {
            m.sidebar_width.update(|w| w.cancel_active());
        }
        if m.route.get() == Route::Document {
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                m.refresh_sidebar_viewport();
            });
        }
    });
    view! {
        <div class="sidebar-resizer" role="separator" tabindex="0" aria-label="调整阅读器侧栏宽度" title="拖动调整侧栏宽度；方向键微调，Home / End 到边界"
            aria-orientation="vertical" aria-controls="pdf-reader-sidebar" hidden=move||!m.chrome.get().sidebar
            aria-disabled=move||m.blocked().to_string()
            aria-valuemin=move||Width::bounds(m.sidebar_viewport.get()).0.to_string()
            aria-valuemax=move||Width::bounds(m.sidebar_viewport.get()).1.to_string()
            aria-valuenow=move||m.sidebar_width.with(|w|w.effective(m.sidebar_viewport.get()).round().to_string())
            class:dragging=move||m.sidebar_width.with(|w|w.dragging()) node_ref=handle
            on:pointerdown=move|e|{
                if m.blocked() || !e.is_primary() || e.button()!=0 { return; }
                let Some(node) = handle.get() else { return; };
                let mut began=false;
                let viewport=m.refresh_sidebar_viewport();
                m.sidebar_width.update(|w|began=w.begin(e.pointer_id(),f64::from(e.client_x()),viewport));
                if !began { return; }
                if node.set_pointer_capture(e.pointer_id()).is_err() {
                    m.sidebar_width.update(|w|{w.cancel(e.pointer_id());});
                    return;
                }
                let _=node.focus(); e.prevent_default();
            }
            on:pointermove=move|e|{
                let viewport=m.refresh_sidebar_viewport();
                m.sidebar_width.update(|w|{w.drag(e.pointer_id(),f64::from(e.client_x()),viewport);});
            }
            on:pointerup=move|e|{
                let mut finished=false;
                m.sidebar_width.update(|w|finished=w.finish(e.pointer_id()));
                if finished { m.save_sidebar_width(); }
                if let Some(node)=handle.get(){let _=node.release_pointer_capture(e.pointer_id());}
            }
            on:pointercancel=move|e|m.sidebar_width.update(|w|{w.cancel(e.pointer_id());})
            on:lostpointercapture=move|e|m.sidebar_width.update(|w|{w.cancel(e.pointer_id());})
            on:keydown=move|e|{
                if e.key()=="Escape" {
                    m.sidebar_width.update(|w|w.cancel_active()); e.prevent_default(); return;
                }
                if m.blocked() { return; }
                let mut changed=false;
                let viewport=m.refresh_sidebar_viewport();
                m.sidebar_width.update(|w|changed=w.key(&e.key(),viewport));
                if changed { e.prevent_default(); m.save_sidebar_width(); }
            }/>
    }
}

impl Model {
    fn select_sidebar_mode(self, mode: Mode) {
        if !self.blocked() {
            self.sidebar_mode.set(mode);
        }
    }
    fn select_outline_row(self, index: usize) {
        let page = self
            .outline_cache
            .with_untracked(|cache| cache.page(index, self.count(), self.blocked()));
        if let Some(page) = page {
            self.outline_selection.set(Some(index));
            self.navigate(page);
        }
    }
    fn load_outline(self) {
        let mut ticket = None;
        self.outline_cache.update(|cache| ticket = cache.begin());
        let Some(ticket) = ticket else {
            return;
        };
        spawn_local(async move {
            let result = bridge::call_bounded::<Vec<Row>>("outline_document", &json!({})).await;
            self.outline_cache.update(|cache| {
                if result
                    .as_ref()
                    .is_err_and(|e| e.starts_with("原生响应超时；"))
                {
                    cache.suspend(ticket);
                } else {
                    cache.finish(ticket, result);
                }
            });
        });
    }
    fn thumbnail_viewport(self, reveal: bool) {
        let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("pdf-thumbnails"))
        else {
            return;
        };
        let height = f64::from(element.client_height());
        if height <= 0. {
            return;
        }
        let mut scroll = f64::from(element.scroll_top());
        if reveal {
            let top = self.page.get_untracked() as f64 * ROW_HEIGHT;
            if top < scroll || top + ROW_HEIGHT > scroll + height {
                scroll = (top - (height - ROW_HEIGHT).max(0.) / 2.).max(0.);
                element.set_scroll_top(scroll as i32);
            }
        }
        let window = Window::new(scroll, height, self.count());
        if self.thumbnail_window.get_untracked() != window {
            self.thumbnail_window.set(window);
        }
    }
    fn load_thumbnail(self, page: usize, window: Window) {
        let mut ticket = None;
        self.thumbnail_cache
            .update(|cache| ticket = cache.begin(page, window));
        let Some(ticket) = ticket else {
            return;
        };
        let scale = self.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(page))
                .and_then(|p| {
                    thumbnail_scale(
                        p.width,
                        p.height,
                        p.rotation,
                        web_sys::window()
                            .map(|w| w.device_pixel_ratio() as f32)
                            .unwrap_or(1.),
                    )
                })
        });
        spawn_local(async move {
            let result = if let Some(scale) = scale {
                bridge::call_bounded::<String>("render_page", &json!({"page":page,"scale":scale}))
                    .await
            } else {
                Err("此页的尺寸或旋转暂不支持缩略图。".into())
            };
            let window = self.thumbnail_window.get_untracked();
            self.thumbnail_cache.update(|cache| {
                // A timeout abandons observation, not native computation.
                // Never enqueue replacement work behind an unobserved RPC.
                if result
                    .as_ref()
                    .is_err_and(|e| e.starts_with("原生响应超时；"))
                {
                    cache.suspend(ticket);
                } else {
                    cache.finish(ticket, window, result);
                }
            });
        });
    }
}

#[component]
pub(super) fn Sidebar(model: Model) -> impl IntoView {
    let m = model;
    let resize =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::resize, move |_| {
            m.thumbnail_viewport(false);
        });
    on_cleanup(move || resize.remove());
    Effect::new(move |_| {
        let _ = m.page.get();
        let _ = m.document.get();
        if m.chrome.get().sidebar
            && m.sidebar_mode.get() == Mode::Thumbnails
            && m.route.get() == Route::Document
        {
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                m.thumbnail_viewport(true)
            });
        }
    });
    Effect::new(move |_| {
        let window = m.thumbnail_window.get();
        if !m.busy.get()
            && m.loaded()
            && m.chrome.get().sidebar
            && m.sidebar_mode.get() == Mode::Thumbnails
            && m.route.get() == Route::Document
            && let Some(page) = m.thumbnail_cache.with(|cache| cache.next(window))
        {
            m.load_thumbnail(page, window);
        }
    });
    Effect::new(move |_| {
        let ready = m.outline_cache.with(|cache| cache.can_begin());
        if ready
            && !m.busy.get()
            && m.loaded()
            && m.chrome.get().sidebar
            && m.sidebar_mode.get() == Mode::Outline
            && m.route.get() == Route::Document
        {
            m.load_outline();
        }
    });
    Effect::new(move |_| {
        let selected = m.search_selection.get();
        if m.sidebar_mode.get() == Mode::Search
            && m.chrome.get().sidebar
            && m.route.get() == Route::Document
            && let Some(index) = selected
        {
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                if m.sidebar_mode.get_untracked() == Mode::Search
                    && m.search_selection.get_untracked() == Some(index)
                {
                    super::reveal_search_hit(index);
                }
            });
        }
    });
    view! {
        <aside class="pages thumbnail-sidebar" id="pdf-reader-sidebar" aria-label="阅读器侧栏" hidden=move||!m.chrome.get().sidebar>
            <div class="sidebar-modes" role="group" aria-label="阅读器侧栏模式">
                <button aria-label="缩略图" title="缩略图" class:active=move||m.sidebar_mode.get()==Mode::Thumbnails aria-pressed=move||(m.sidebar_mode.get()==Mode::Thumbnails).to_string() disabled=move||m.blocked() on:click=move|_|m.select_sidebar_mode(Mode::Thumbnails)><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 3h5v5H3z M12 3h5v5h-5z M3 12h5v5H3z M12 12h5v5h-5z"/></svg></button>
                <button aria-label="大纲" title="大纲" class:active=move||m.sidebar_mode.get()==Mode::Outline aria-pressed=move||(m.sidebar_mode.get()==Mode::Outline).to_string() disabled=move||m.blocked() on:click=move|_|m.select_sidebar_mode(Mode::Outline)><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 5h2 M8 5h9 M3 10h2 M8 10h9 M3 15h2 M8 15h9"/></svg></button>
                <button aria-label="侧栏搜索" title="搜索" class:active=move||m.sidebar_mode.get()==Mode::Search aria-pressed=move||(m.sidebar_mode.get()==Mode::Search).to_string() disabled=move||m.search_input_blocked() on:click=move|_|m.show_search_sidebar()><svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="8" cy="8" r="5"/><path d="m12 12 5 5"/></svg></button>
                <button aria-label="侧栏批注" title="批注" class:active=move||m.sidebar_mode.get()==Mode::Annotations aria-pressed=move||(m.sidebar_mode.get()==Mode::Annotations).to_string() disabled=move||m.blocked() on:click=move|_|m.select_sidebar_mode(Mode::Annotations)>"A"</button>
            </div>
            <div class="section-label" hidden=move||m.sidebar_mode.get()!=Mode::Thumbnails>"页面"<span>{move||m.count()}</span></div>
            <Show when=move||m.sidebar_mode.get()==Mode::Thumbnails&&m.thumbnail_cache.with(|cache|cache.suspended())><p class="thumbnail-paused" role="status">"缩略图加载已暂停；请先保存文档，再正常重启候选。"</p></Show>
            <div id="pdf-thumbnails" class="thumbnail-scroll" hidden=move||m.sidebar_mode.get()!=Mode::Thumbnails on:scroll=move|e|{
                if e.target().and_then(|t|t.dyn_into::<web_sys::HtmlElement>().ok()).is_some(){m.thumbnail_viewport(false);}
            }>
                {move||{
                    let window=m.thumbnail_window.get();
                    view!{
                        <div aria-hidden="true" style=format!("height:{}px",window.before())></div>
                        <For each={move||window.rows().collect::<Vec<_>>()} key=|page|*page children=move|page|view!{
                            <div class="thumbnail-row">
                                <button class="thumbnail-select" aria-label=format!("第 {} 页",page+1) aria-current=move||if m.page.get()==page{"page"}else{"false"}
                                    disabled=move||m.blocked() on:click=move|_|m.navigate(page)>
                                    <span class="thumbnail-paper" class:selected=move||m.page.get()==page>
                                        {move||m.thumbnail_cache.with(|cache|match cache.image(page){
                                            Some(Image::Ready(source))=>view!{<img src=source.clone() alt=format!("第 {} 页缩略图",page+1) draggable="false"/>}.into_any(),
                                            Some(Image::Failed(_))=>view!{<span class="thumbnail-placeholder">"无法预览"</span>}.into_any(),
                                            None=>view!{<span class="thumbnail-placeholder">"加载中"</span>}.into_any(),
                                        })}
                                    </span>
                                    <span class="thumbnail-number">{page+1}</span>
                                </button>
                                {move||m.thumbnail_cache.with(|cache|match cache.image(page){
                                    Some(Image::Failed(message))=>{
                                        let message=message.clone();
                                        view!{<button class="thumbnail-retry" title=message aria-label=format!("重试第 {} 页缩略图",page+1)
                                            disabled=move||m.busy.get()||m.thumbnail_cache.with(|cache|cache.suspended()) on:click=move|_|m.thumbnail_cache.update(|cache|{cache.retry(page);})>"重试"</button>}.into_any()
                                    },
                                    _=>().into_any(),
                                })}
                            </div>
                        }/>
                        <div aria-hidden="true" style=format!("height:{}px",window.after())></div>
                    }
                }}
            </div>
            <section class="outline-panel" aria-label="文档大纲" hidden=move||m.sidebar_mode.get()!=Mode::Outline>
                <p class="outline-status" role="status">{move||m.outline_cache.with(|cache| {
                    if cache.suspended() { return "大纲读取已暂停；请先保存文档，再正常重启候选。".into(); }
                    match cache.result() {
                        None => "正在读取大纲…".into(),
                        Some(Err(error)) => format!("无法读取大纲：{error}"),
                        Some(Ok(rows)) if rows.is_empty() => "此文档没有大纲".into(),
                        Some(Ok(rows)) => format!("{} 项 · 外部目标不会打开", rows.len()),
                    }
                })}</p>
                <div class="outline-scroll">
                    <For each=move||m.outline_cache.with(|cache|cache.result().and_then(|result|result.as_ref().ok()).cloned().unwrap_or_default().into_iter().enumerate().collect::<Vec<_>>()) key=|(index,row)|(*index,row.path.clone(),row.title.clone(),row.page_index) children=move|(index,row)|{
                        let title=format!("{}{}{}", "  ".repeat(row.depth.min(12)), row.title, row.page_index.map(|page|format!("   · {}", page+1)).unwrap_or_default());
                        let tooltip=row.title.clone();
                        view!{<button class="outline-row" class:selected=move||m.outline_selection.get()==Some(index) aria-current=move||if m.outline_selection.get()==Some(index){"true"}else{"false"}
                            disabled=move||m.outline_cache.with(|cache|cache.page(index,m.count(),m.blocked()).is_none()) on:click=move|_|m.select_outline_row(index) title=tooltip>
                            <span class="outline-title">{title}</span>
                        </button>}
                    }/>
                </div>
            </section>
            <SearchSidebar model=m/>
            <AnnotationSidebar model=m/>
        </aside>
    }
}

#[component]
fn SearchSidebar(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <section class="sidebar-search-panel" aria-label="文档搜索" hidden=move||m.sidebar_mode.get()!=Mode::Search>
            <input class="sidebar-search-input" aria-label="搜索文档文字" placeholder="搜索文档文字" disabled=move||m.search_input_blocked()
                prop:value=move||m.search_query.get() on:input=move|e|{m.search_query.set(event_target_value(&e));m.queue_live_search();}
                on:keydown=move|e|{if e.key()=="Enter"{m.search();}else if e.key()=="Escape"{m.cancel_search();}}/>
            <div class="sidebar-search-navigation" aria-label="搜索结果导航">
                <button disabled=move||m.blocked()||!m.search_result.with(|r|r.as_ref().is_some_and(|r|!r.hits.is_empty())) on:click=move|_|m.step_search_hit(true)>"上一处"</button>
                <button disabled=move||m.blocked()||!m.search_result.with(|r|r.as_ref().is_some_and(|r|!r.hits.is_empty())) on:click=move|_|m.step_search_hit(false)>"下一处"</button>
                <button disabled=move||m.search_input_blocked() on:click=move|_|m.cancel_search()>"取消"</button>
            </div>
            <p class="outline-status" role="status" aria-label="当前搜索结果">{move||m.search_result.with(|r|match r {
                Some(r) if !r.hits.is_empty()=>format!("{} / {}{}",m.search_selection.get().map_or(0,|index|index+1),r.hits.len(),if r.truncated{" · 已达上限，请细化关键词"}else{""}),
                Some(r)=>pdfmaster_ui::search_status_message(&r.query,0,r.truncated),
                None if m.search_flow.with(|flow|flow.suspended())=>"搜索已暂停；请先保存文档，再正常重启候选。".into(),
                None=>m.search_status.get(),
            })}</p>
            <div class="search-results sidebar-search-results" id="search-results" aria-label="大纲或搜索结果列表">
                <For each=move||m.search_result.with(|r|r.as_ref().map(|r|r.hits.clone().into_iter().enumerate().collect::<Vec<_>>()).unwrap_or_default()) key=|(index,hit)|(*index,hit.page_index,hit.match_start) children=move|(index,hit)|{
                    let label=format!("第{}页\n{}",hit.page_index+1,hit.snippet);
                    let detail=format!("{}；字符 {}–{} · 片段内 {}–{}",label,hit.match_start,hit.match_end,hit.snippet_match_start,hit.snippet_match_end);
                    view!{<button class="sidebar-search-row search-hit" id=format!("search-hit-{index}") class:selected=move||m.search_selection.get()==Some(index) aria-current=move||if m.search_selection.get()==Some(index){"true"}else{"false"} title=detail disabled=move||m.blocked() on:click=move|_|m.select_search_hit(index)><span>{label}</span></button>}
                }/>
            </div>
            <details class="sidebar-search-options"><summary>"搜索说明与选项"</summary>
                <label class="search-case"><input type="checkbox" disabled=move||m.search_input_blocked() prop:checked=move||m.search_case_sensitive.get() on:change=move|e|{m.search_case_sensitive.set(event_target_checked(&e));m.queue_live_search();}/><span>"区分大小写"</span></label>
                <p>"搜索当前文档；结果暂不含PDF页标签或页面高亮。"</p>
                {move||m.search_result.with(|r|r.as_ref().map(|r|view!{<p>{format!("已扫描 {} 页 · {} 页含文字；{}",r.pages_scanned,r.searchable_pages,r.limitation)}</p>}))}
            </details>
        </section>
    }
}
