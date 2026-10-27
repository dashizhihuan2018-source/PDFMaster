use super::{Document, Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    image_import::{Configuration, Paper, Selection},
    shell::Route,
};
use serde_json::json;
use wasm_bindgen::JsCast;

impl Model {
    pub(super) fn prepare_images(self) {
        if self.busy.get_untracked()
            || self.image_import.get_untracked().is_some()
            || self.has_note_draft()
            || self.has_ink()
        {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Option<Selection>>("prepare_image_import", &json!({})).await {
                Ok(Some(selection)) => {
                    let id = selection.id;
                    match Configuration::new(selection) {
                        Ok(c) => {
                            self.image_import.set(Some(c));
                            leptos::leptos_dom::helpers::request_animation_frame(|| {
                                focus("image-paper")
                            });
                        }
                        Err(e) => {
                            self.message.set(e);
                            let _ = bridge::call::<bool>("cancel_image_import", &json!({"id":id}))
                                .await;
                        }
                    }
                }
                Ok(None) => self
                    .message
                    .set("已取消选择图片；现有文档保持不变。".into()),
                Err(e) => self.message.set(e),
            }
            self.busy.set(false);
        });
    }
    fn image_current(self, id: u32) -> bool {
        self.image_import
            .with_untracked(|c| c.as_ref().is_some_and(|c| c.selection.id == id))
    }
    fn change_images(self, change: impl FnOnce(&mut Configuration)) {
        self.image_import.update(|c| {
            if let Some(c) = c
                && !c.running
                && !c.cancelling
            {
                change(c);
                c.error.clear();
            }
        });
    }
    fn convert_images(self) {
        let Some(c) = self.image_import.get_untracked() else {
            return;
        };
        if c.running || c.cancelling {
            return;
        }
        let layout = match c.layout() {
            Ok(l) => l,
            Err(e) => {
                self.image_import.update(|c| {
                    if let Some(c) = c {
                        c.error = e;
                    }
                });
                return;
            }
        };
        let id = c.selection.id;
        self.remember_tab();
        self.image_import.update(|c| {
            if let Some(c) = c {
                c.running = true;
                c.error.clear();
            }
        });
        self.busy.set(true);
        spawn_local(async move {
            let result = bridge::call::<Document>(
                "convert_image_import",
                &json!({"id":id,"order":c.order,"layout":layout}),
            )
            .await;
            if !self.image_current(id) {
                return;
            }
            match result {
                Ok(doc) => {
                    // Publication won the native lock before a possible cancel.
                    // Render that authoritative document, never hide a new tab.
                    self.image_import.set(None);
                    self.restore_tab(&doc);
                    self.page.set(0);
                    self.page_jump.set("1".into());
                    self.search_result.set(None);
                    self.chrome
                        .update(|c| *c = c.document_opened(false, false, true));
                    self.route.set(Route::Document);
                    self.replace_document_view(Some(doc));
                    let mut message =
                        "已创建独立 PDF；来源图片和已有文档保持不变，请导出副本保存。".to_string();
                    if !self.render().await {
                        message.push_str(&format!(" 预览失败：{}", self.message.get_untracked()));
                    }
                    self.message.set(message);
                }
                Err(e) => self.image_import.update(|c| {
                    if let Some(c) = c {
                        c.running = false;
                        c.error = e;
                    }
                }),
            }
            self.busy.set(false);
        });
    }
    fn cancel_images(self) {
        let Some(c) = self.image_import.get_untracked() else {
            return;
        };
        if c.cancelling {
            return;
        }
        let id = c.selection.id;
        self.image_import.update(|c| {
            if let Some(c) = c {
                c.cancelling = true;
            }
        });
        spawn_local(async move {
            let result = bridge::call::<bool>("cancel_image_import", &json!({"id":id})).await;
            if !self.image_current(id) {
                return;
            }
            match result {
                Ok(true) => {
                    self.image_import.set(None);
                    self.busy.set(false);
                    self.message.set("已取消图片转换；未创建文档。".into());
                    focus("image-tool-entry");
                }
                Ok(false) => self.image_import.update(|c| {
                    if let Some(c) = c {
                        c.cancelling = false;
                        c.error = "取消未确认；请等待当前任务结果。".into();
                    }
                }),
                Err(e) => self.image_import.update(|c| {
                    if let Some(c) = c {
                        c.cancelling = false;
                        c.error = format!("取消未确认：{e}；请重试取消或等待结果。");
                    }
                }),
            }
        });
    }
}
fn focus(id: &str) {
    if let Some(e) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = e.focus();
    }
}
fn trap_tab(e: &web_sys::KeyboardEvent) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let ids = [
        "image-paper",
        "image-orientation",
        "image-margin",
        "image-list",
        "image-up",
        "image-down",
        "image-remove",
        "image-cancel",
        "image-create",
    ];
    let enabled: Vec<_> = ids
        .into_iter()
        .filter(|id| {
            doc.get_element_by_id(id)
                .is_some_and(|e| !e.has_attribute("disabled"))
        })
        .collect();
    if enabled.is_empty() {
        return;
    }
    let current = doc.active_element().map(|e| e.id()).unwrap_or_default();
    let next = match enabled.iter().position(|id| *id == current) {
        Some(i) => {
            if e.shift_key() {
                (i + enabled.len() - 1) % enabled.len()
            } else {
                (i + 1) % enabled.len()
            }
        }
        None => 0,
    };
    e.prevent_default();
    focus(enabled[next]);
}
#[component]
pub(super) fn Dialog(model: Model) -> impl IntoView {
    let disabled = move || {
        model
            .image_import
            .with(|c| c.as_ref().is_none_or(|c| c.running || c.cancelling))
    };
    let pixels = move || {
        model
            .image_import
            .with(|c| c.as_ref().is_some_and(|c| c.paper == Paper::Pixels))
    };
    let no_selection = move || {
        model
            .image_import
            .with(|c| c.as_ref().is_none_or(|c| c.selected.is_none()))
    };
    view! {
        <Show when=move||model.image_import.with(|c|c.is_some())>
            <div class="image-import-backdrop">
                <section class="image-import-dialog" role="dialog" aria-modal="true" aria-labelledby="image-import-title" on:keydown=move|e|{
                    if e.key()=="Escape" {e.prevent_default();model.cancel_images();}
                    else if e.key()=="Tab" {trap_tab(&e);}
                    else if e.key()=="Enter" && e.target().and_then(|t|t.dyn_into::<web_sys::Element>().ok()).is_some_and(|t|t.tag_name()!="BUTTON") {e.prevent_default();model.convert_images();}
                }>
                    <h2 id="image-import-title">"图片转 PDF · 本地候选功能"</h2>
                    <p class="image-format-notice">"PNG、JPEG、支持的 TIFF（全部帧）；最多 32 个文件、500 页。暂不支持带色彩配置的图片、GIF、HEIC、SVG 或 OCR。图片像素尺寸：1 像素 = 1 pt，不推测 DPI；不修改来源文件。"</p>
                    <div class="image-import-options">
                        <label>"纸型"<select id="image-paper" aria-label="纸型" disabled=disabled prop:value=move||model.image_import.with(|c|c.as_ref().map(|c|match c.paper{Paper::A4=>"a4",Paper::Letter=>"letter",Paper::Pixels=>"pixels"}).unwrap_or("a4")) on:change=move|e|{let v=event_target_value(&e);model.change_images(|c|c.paper=match v.as_str(){"letter"=>Paper::Letter,"pixels"=>Paper::Pixels,_=>Paper::A4});}><option value="a4">"A4"</option><option value="letter">"Letter"</option><option value="pixels">"图片像素尺寸"</option></select></label>
                        <label>"方向"<select id="image-orientation" aria-label="方向" disabled=move||disabled()||pixels() prop:value=move||if model.image_import.with(|c|c.as_ref().is_some_and(|c|c.landscape)){"landscape"}else{"portrait"} on:change=move|e|{let v=event_target_value(&e);model.change_images(|c|c.landscape=v=="landscape");}><option value="portrait">"纵向"</option><option value="landscape">"横向"</option></select></label>
                        <label>"边距（pt）"<input id="image-margin" aria-label="边距（pt）" inputmode="decimal" disabled=move||disabled()||pixels() prop:value=move||model.image_import.with(|c|c.as_ref().map(|c|c.margin.clone()).unwrap_or_default()) on:input=move|e|{let v=event_target_value(&e);model.change_images(|c|c.margin=v);}/></label>
                    </div>
                    <div class="image-import-table"><div class="image-import-header">"转换顺序 / 来源图片"</div>
                        <div id="image-list" class="image-import-list" role="listbox" aria-label="转换顺序 / 来源图片" tabindex="0" on:keydown=move|e|{
                            if ["ArrowUp","ArrowDown"].contains(&e.key().as_str()) {e.prevent_default();model.change_images(|c|{if !c.order.is_empty(){let i=c.selected.unwrap_or(0);c.selected=Some(if e.key()=="ArrowUp"{i.saturating_sub(1)}else{(i+1).min(c.order.len()-1)});}});}
                        }>
                            <For each=move||model.image_import.with(|c|c.as_ref().map(|c|c.order.clone()).unwrap_or_default()) key=|i|*i children=move|native_index|{
                                let label=model.image_import.with_untracked(|c|c.as_ref().and_then(|c|c.selection.names.get(native_index).cloned()).unwrap_or_default());
                                view!{<button type="button" role="option" tabindex="-1" title=label.clone() aria-selected=move||model.image_import.with(|c|c.as_ref().is_some_and(|c|c.selected.and_then(|i|c.order.get(i)).copied()==Some(native_index))).to_string() disabled=disabled on:click=move|_|model.change_images(|c|c.selected=c.order.iter().position(|i|*i==native_index))>{label.clone()}</button>}
                            }/>
                        </div>
                    </div>
                    <div class="image-import-order"><button id="image-up" disabled=move||disabled()||model.image_import.with(|c|c.as_ref().is_none_or(|c|c.selected.is_none_or(|i|i==0))) on:click=move|_|model.change_images(|c|c.shift(-1))>"上移"</button><button id="image-down" disabled=move||disabled()||model.image_import.with(|c|c.as_ref().is_none_or(|c|c.selected.is_none_or(|i|i+1>=c.order.len()))) on:click=move|_|model.change_images(|c|c.shift(1))>"下移"</button><button id="image-remove" disabled=move||disabled()||no_selection() on:click=move|_|model.change_images(|c|c.remove())>"移除"</button></div>
                    <p class="image-import-status" role="status" aria-live="polite">{move||model.image_import.with(|c|c.as_ref().map(|c|if !c.error.is_empty(){c.error.clone()}else if c.running{"正在转换图片 · 不覆盖来源文件或当前文档".into()}else{format!("{} 个文件；将创建独立 PDF 标签。",c.order.len())}).unwrap_or_default())}</p>
                    <footer><Show when=move||model.image_import.with(|c|c.as_ref().is_some_and(|c|c.running))><span class="home-import-spinner" aria-label="正在转换"></span></Show><button id="image-cancel" disabled=move||model.image_import.with(|c|c.as_ref().is_some_and(|c|c.cancelling)) on:click=move|_|model.cancel_images()>"取消"</button><button id="image-create" class="primary" disabled=move||disabled()||model.image_import.with(|c|c.as_ref().is_none_or(|c|c.layout().is_err())) on:click=move|_|model.convert_images()>"创建新 PDF"</button></footer>
                </section>
            </div>
        </Show>
    }
}
