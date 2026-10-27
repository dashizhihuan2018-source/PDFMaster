use super::{Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::annotations::{Identity, Row, Sort, copy_status_is_current, sort_rows, summary};
use pdfmaster_ui::{
    outline::Mode,
    shell::Route,
    workbench::{Mode as ToolMode, Panel},
};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue};

fn modified_date(row: &Row) -> Option<js_sys::Date> {
    let date = js_sys::Date::new(&JsValue::from_str(row.modified.as_deref()?));
    date.get_time().is_finite().then_some(date)
}

fn time_label(row: &Row) -> String {
    modified_date(row)
        .map(|d| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                d.get_full_year(),
                d.get_month() + 1,
                d.get_date(),
                d.get_hours(),
                d.get_minutes()
            )
        })
        .unwrap_or_else(|| "时间未记录".into())
}

fn close_options() {
    if let Some(menu) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("annotation-options"))
    {
        let _ = menu.remove_attribute("open");
    }
}

async fn copy_text(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("窗口不可用")?;
    let navigator = window.navigator();
    let clipboard = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("clipboard"))
        .map_err(|_| "剪贴板不可用")?;
    if clipboard.is_null() || clipboard.is_undefined() {
        return Err("当前环境不允许访问剪贴板。".into());
    }
    let clipboard = clipboard
        .dyn_into::<web_sys::Clipboard>()
        .map_err(|_| "剪贴板接口不可用")?;
    let mut reject_fn = None;
    let timeout = js_sys::Promise::new(&mut |_, reject| reject_fn = Some(reject));
    let reject = reject_fn.ok_or("无法建立复制响应保护")?;
    let timer = leptos::leptos_dom::helpers::set_timeout_with_handle(
        move || {
            let _ = reject.call1(
                &JsValue::NULL,
                &JsValue::from_str("复制响应超时，剪贴板结果未知。"),
            );
        },
        std::time::Duration::from_secs(5),
    )
    .map_err(|_| "无法建立复制响应保护")?;
    let promises = js_sys::Array::new();
    promises.push(&clipboard.write_text(text));
    promises.push(&timeout);
    let result = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::race(&promises)).await;
    timer.clear();
    result.map(|_| ()).map_err(|e| {
        e.as_string()
            .unwrap_or_else(|| "剪贴板拒绝了复制请求。".into())
    })
}

impl Model {
    pub(super) fn annotation_rows(self) -> Vec<Row> {
        let mut rows = self.annotation_cache.with(|cache| {
            if cache.suspended() {
                return Vec::new();
            }
            cache
                .result()
                .and_then(|r| r.as_ref().ok())
                .cloned()
                .unwrap_or_default()
        });
        sort_rows(&mut rows, self.annotation_sort.get());
        rows
    }
    fn load_annotations(self) {
        let mut ticket = None;
        self.annotation_cache.update(|cache| ticket = cache.begin());
        let Some(ticket) = ticket else {
            return;
        };
        spawn_local(async move {
            let result = bridge::call_bounded::<Vec<Row>>("annotations_document", &json!({}))
                .await
                .map(|mut rows| {
                    for row in &mut rows {
                        row.modified_millis = modified_date(row).map(|d| d.get_time() as i64);
                    }
                    rows
                });
            self.annotation_cache.update(|cache| {
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
    fn select_annotation(self, identity: Identity) {
        if self.blocked() {
            return;
        }
        let Some(row) = self
            .annotation_rows()
            .into_iter()
            .find(|r| r.identity() == identity && r.page_index < self.count())
        else {
            return;
        };
        self.annotation_selection.set(Some(identity));
        self.navigate(row.page_index);
    }
    fn sort_annotations(self, sort: Sort) {
        if self.blocked() {
            return;
        }
        self.annotation_sort.set(sort);
        close_options();
    }
    fn copy_annotations(self) {
        if self.blocked() || self.annotation_copying.get_untracked() {
            return;
        }
        let rows = self.annotation_rows();
        if rows.is_empty() {
            return;
        }
        let text = summary(&rows);
        let generation = self.annotation_cache.with_untracked(|c| c.generation());
        let pending_message = "正在复制点击时的批注摘要…";
        close_options();
        self.annotation_copying.set(true);
        self.message.set(pending_message.into());
        spawn_local(async move {
            let result = copy_text(&text).await;
            self.annotation_copying.set(false);
            if copy_status_is_current(
                generation,
                self.annotation_cache.with_untracked(|c| c.generation()),
                pending_message,
                &self.message.get_untracked(),
            ) {
                self.message.set(match result {
                    Ok(()) => "已复制点击时的批注摘要。".into(),
                    Err(error) => format!("未能确认复制批注摘要：{error}"),
                });
            }
        });
    }
    fn annotation_tool(self, panel: Panel) {
        self.select_mode(if panel == Panel::Ink {
            ToolMode::Drawing
        } else {
            ToolMode::Annotation
        });
        self.select_panel(Some(panel));
    }
}

#[component]
pub(super) fn AnnotationSidebar(model: Model) -> impl IntoView {
    let m = model;
    Effect::new(move |_| {
        let ready = m.annotation_cache.with(|c| c.can_begin());
        if ready
            && !m.blocked()
            && m.loaded()
            && m.chrome.get().sidebar
            && m.sidebar_mode.get() == Mode::Annotations
            && m.route.get() == Route::Document
        {
            m.load_annotations();
        }
    });
    view! {
        <section class="annotation-panel" aria-label="文档批注" hidden=move||m.sidebar_mode.get()!=Mode::Annotations>
            <div class="annotation-header"><strong>"批注"</strong>
                <button class="annotation-delete" aria-label="删除所选批注" title="统一批注删除尚未迁移；便笺和形状仍可在各自编辑面板删除" disabled><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 5h14 M7 5V3h6v2 M5 5l1 12h8l1-12 M8 8v6 M12 8v6"/></svg></button>
                <details id="annotation-options" class="annotation-options"><summary aria-label="批注选项" title="批注排序与复制摘要">"•••"</summary>
                    <div class="annotation-menu" role="menu">
                        <button role="menuitemradio" aria-checked=move||(m.annotation_sort.get()==Sort::Page).to_string() disabled=move||m.blocked() on:click=move|_|m.sort_annotations(Sort::Page)>"按页码排序"</button>
                        <button role="menuitemradio" aria-checked=move||(m.annotation_sort.get()==Sort::Newest).to_string() disabled=move||m.blocked() on:click=move|_|m.sort_annotations(Sort::Newest)>"最新修改在前"</button>
                        <button role="menuitem" disabled=move||m.blocked()||m.annotation_copying.get()||m.annotation_rows().is_empty() on:click=move|_|m.copy_annotations()>"复制批注摘要"</button>
                    </div>
                </details>
            </div>
            <Show when=move||m.annotation_cache.with(|c|c.suspended() || !matches!(c.result(),Some(Ok(_))))>
                <p class="annotation-state" role="status">{move||m.annotation_cache.with(|c|if c.suspended(){"批注读取已暂停；请先保存文档，再正常重启候选。".into()}else{match c.result(){Some(Err(e))=>format!("无法读取批注：{e}"),_=>"正在读取批注…".into()}})}</p>
            </Show>
            <Show when=move||m.annotation_cache.with(|c|matches!(c.result(),Some(Ok(rows)) if rows.is_empty()) && !c.suspended())>
                <div class="annotation-empty">
                    <svg class="annotation-empty-glyph" viewBox="0 0 98 98" aria-hidden="true"><g transform="translate(0 98) scale(1 -1)" fill="none" stroke="#737373" stroke-width="2"><rect x="22" y="16" width="52" height="66" rx="5"/><path d="M33 62h27 M33 51h27 M33 40h27"/><rect x="5" y="65" width="29" height="20" rx="5" fill="#333"/><path d="M58 17 81 48 87 43 64 12z" fill="#333"/></g></svg>
                    <p>"使用批注和绘画工具标记PDF"</p>
                    <div class="annotation-empty-tools" role="group" aria-label="批注工具">
                        <button aria-label="批注工具 · 高亮" title="高亮尚未迁移" disabled>"▰"</button>
                        <button aria-label="批注工具 · 画笔" title="画笔" disabled=move||m.blocked() on:click=move|_|m.annotation_tool(Panel::Ink)>"✎"</button>
                        <button aria-label="批注工具 · 文字" title="文字" disabled=move||m.blocked() on:click=move|_|m.annotation_tool(Panel::Text)>"T"</button>
                        <button aria-label="批注工具 · 便笺" title="便笺" disabled=move||m.blocked() on:click=move|_|m.annotation_tool(Panel::Notes)>"▱"</button>
                    </div>
                </div>
            </Show>
            <div class="annotation-scroll" role="list" aria-label="批注列表">
                <For each=move||m.annotation_rows() key=|row|row.identity() children=move|row|{
                    let identity=row.identity();let heading=row.heading();let detail=row.detail();let time=time_label(&row);
                    let tooltip=format!("{heading}\n{detail}\n{time}");let color=row.color_css();let height=row.height();
                    view! {<button class="annotation-row" role="listitem" style=format!("height:{height}px") title=tooltip
                        class:selected=move||m.annotation_selection.get()==Some(identity) aria-current=move||(m.annotation_selection.get()==Some(identity)).to_string()
                        disabled=move||{m.blocked()||row.page_index>=m.count()} on:click=move|_|m.select_annotation(identity)>
                        <span class="annotation-dot" style=format!("background:{color}") aria-hidden="true"></span>
                        <span class="annotation-heading">{heading}</span>
                        {(!detail.is_empty()).then(||view!{<span class="annotation-detail">{detail}</span>})}
                        <span class="annotation-time">{time}</span>
                    </button>}
                }/>
            </div>
            <details class="annotation-limitations"><summary>"当前限制"</summary><p>"页内精确定位、选中文字摘录和统一批注删除尚未迁移；当前列表只跳转到所在页。设备颜色未做ICC色彩管理。"</p></details>
        </section>
    }
}
