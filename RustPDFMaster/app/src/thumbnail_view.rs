use super::{Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    shell::Route,
    thumbnails::{Image, ROW_HEIGHT, Window, thumbnail_scale},
};
use serde_json::json;
use wasm_bindgen::JsCast;

impl Model {
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
        if m.chrome.get().sidebar && m.route.get() == Route::Document {
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
            && m.route.get() == Route::Document
            && let Some(page) = m.thumbnail_cache.with(|cache| cache.next(window))
        {
            m.load_thumbnail(page, window);
        }
    });
    view! {
        <aside class="pages thumbnail-sidebar" aria-label="页面缩略图" hidden=move||!m.chrome.get().sidebar>
            <div class="section-label">"页面"<span>{move||m.count()}</span></div>
            <Show when=move||m.thumbnail_cache.with(|cache|cache.suspended())><p class="thumbnail-paused" role="status">"缩略图加载已暂停；请先保存文档，再正常重启候选。"</p></Show>
            <div id="pdf-thumbnails" class="thumbnail-scroll" on:scroll=move|e|{
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
        </aside>
    }
}
