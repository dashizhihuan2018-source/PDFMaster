use super::{Document, Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::shell::Route;
use serde::Deserialize;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window","__TAURI__","core"],js_name=Channel)]
    type NativeChannel;
    #[wasm_bindgen(constructor,catch,js_namespace = ["window","__TAURI__","core"],js_class=Channel)]
    fn new(callback: &js_sys::Function) -> Result<NativeChannel, JsValue>;
    #[wasm_bindgen(method,setter=onmessage)]
    fn set_onmessage(this: &NativeChannel, callback: &js_sys::Function);
    #[wasm_bindgen(catch,js_namespace = ["window","__TAURI__","core"],js_name=invoke)]
    async fn invoke(command: &str, args: JsValue) -> Result<JsValue, JsValue>;
}
#[derive(Deserialize)]
struct Notice {
    kind: String,
    id: Option<u32>,
    x: f64,
    y: f64,
    error: Option<String>,
}
fn detach(channel: &NativeChannel) {
    // Transfer the harmless replacement callback to JS ownership; no eval and
    // no dropped Rust callback can be invoked while unregister is in flight.
    let noop = Closure::<dyn FnMut(JsValue)>::new(|_| {}).into_js_value();
    channel.set_onmessage(noop.unchecked_ref());
}
fn unregister(id: u32) {
    spawn_local(async move {
        let _ = bridge::call::<()>("unregister_file_drop_channel", &json!({"id":id})).await;
    });
}
pub(super) fn listen(model: Model) {
    let disposed = Rc::new(Cell::new(false));
    let listener = Rc::new(RefCell::new(None::<(NativeChannel, u32)>));
    let cleanup = StoredValue::new_local((disposed.clone(), listener.clone()));
    on_cleanup(move || {
        cleanup.with_value(|(disposed, listener)| {
            disposed.set(true);
            if let Some((channel, id)) = listener.borrow_mut().take() {
                detach(&channel);
                unregister(id);
            }
        })
    });
    spawn_local(async move {
        let active = disposed.clone();
        let callback = Closure::<dyn FnMut(JsValue)>::new(move |payload| {
            if !active.get()
                && let Ok(notice) = serde_wasm_bindgen::from_value::<Notice>(payload)
            {
                model.file_drop_notice(notice);
            }
        })
        .into_js_value();
        let Ok(channel) = NativeChannel::new(callback.unchecked_ref()) else {
            return;
        };
        let args = js_sys::Object::new();
        if js_sys::Reflect::set(&args, &JsValue::from_str("channel"), &channel).is_err() {
            detach(&channel);
            return;
        }
        match invoke("register_file_drop_channel", args.into())
            .await
            .and_then(|v| {
                v.as_f64()
                    .filter(|n| *n >= 1. && *n <= f64::from(u32::MAX) && n.fract() == 0.)
                    .map(|n| n as u32)
                    .ok_or(JsValue::NULL)
            }) {
            Ok(id) if disposed.get() => {
                detach(&channel);
                unregister(id);
            }
            Ok(id) => {
                *listener.borrow_mut() = Some((channel, id));
            }
            Err(_) => detach(&channel),
        }
    });
}
fn discard(id: u32) {
    spawn_local(async move {
        let _ = bridge::call::<bool>("cancel_file_drop", &json!({"id":id})).await;
    });
}
impl Model {
    fn file_drop_index(self, x: f64, y: f64) -> Option<usize> {
        if self.route.get_untracked() != Route::Document
            || self.blocked()
            || !x.is_finite()
            || !y.is_finite()
            || self.overview_drag.with_untracked(|d| d.is_some())
        {
            return None;
        }
        let document = web_sys::window()?.document()?;
        let target = document.element_from_point(x as f32, y as f32)?;
        if self.overview_visible.get_untracked() {
            let grid = target.closest("#pdf-overview").ok()??;
            let rect = grid.get_bounding_client_rect();
            self.overview_layout.with_untracked(|l| {
                l.insertion(
                    x - rect.left() - f64::from(grid.client_left()) + f64::from(grid.scroll_left()),
                    y - rect.top() - f64::from(grid.client_top()) + f64::from(grid.scroll_top()),
                )
            })
        } else {
            target.closest("#pdf-reader").ok()??;
            Some(
                self.page
                    .get_untracked()
                    .saturating_add(1)
                    .min(self.count()),
            )
        }
    }
    fn file_drop_notice(self, notice: Notice) {
        if notice.kind == "leave" {
            if self.overview_drag.with_untracked(|d| d.is_none()) {
                self.overview_marker.set(None);
            }
            return;
        }
        let index = self.file_drop_index(notice.x, notice.y);
        if notice.kind == "over" {
            self.overview_marker
                .set(if self.overview_visible.get_untracked() {
                    index
                } else {
                    None
                });
            return;
        }
        if notice.kind != "drop" {
            return;
        }
        self.overview_marker.set(None);
        if let Some(error) = notice.error {
            self.message.set(error);
            return;
        }
        let Some(id) = notice.id else {
            return;
        };
        let Some(index) = index else {
            discard(id);
            return;
        };
        let Some(document) = self.document.with_untracked(|d| d.as_ref().map(|d| d.id)) else {
            discard(id);
            return;
        };
        let overview = self.overview_visible.get_untracked();
        let old_count = self.count();
        self.remember_tab();
        self.file_drop_request.set(Some(id));
        self.busy.set(true);
        self.message
            .set("正在导入拖入的 PDF；整批完成后才插入页面。".into());
        spawn_local(async move {
            let result = bridge::call::<Option<Document>>(
                "import_file_drop",
                &json!({"id":id,"document":document,"index":index}),
            )
            .await;
            // A confirmed cancellation already released native ownership and
            // may have admitted a newer operation. Never observe the old read.
            if self.file_drop_request.get_untracked() != Some(id) {
                return;
            }
            match result {
                Ok(Some(doc)) => {
                    let added = doc.info.page_count.saturating_sub(old_count);
                    self.page
                        .set(index.min(doc.info.page_count.saturating_sub(1)));
                    self.page_jump
                        .set((self.page.get_untracked() + 1).to_string());
                    self.search_result.set(None);
                    self.replace_document_view(Some(doc));
                    if overview {
                        self.overview_selection.update(|s| {
                            s.restore(&(index..index + added).collect::<Vec<_>>(), self.count())
                        });
                    }
                    let mut status = format!("已导入 {} 页，可撤销；请导出完整副本保存。", added);
                    if !self.render().await {
                        status.push_str(&format!(" 预览失败：{}", self.message.get_untracked()));
                    }
                    self.message.set(status);
                    leptos::leptos_dom::helpers::request_animation_frame(move || {
                        self.measure_overview(true)
                    });
                }
                Ok(None) => self.message.set("已取消文件导入，文档未修改。".into()),
                Err(error) => self.message.set(error),
            }
            self.file_drop_request.set(None);
            self.busy.set(false);
        });
    }
    pub(super) fn cancel_file_import(self) {
        let Some(id) = self.file_drop_request.get_untracked() else {
            return;
        };
        spawn_local(async move {
            let result = bridge::call::<bool>("cancel_file_drop", &json!({"id":id})).await;
            if self.file_drop_request.get_untracked() != Some(id) {
                return;
            }
            match result {
                Ok(true) => {
                    self.file_drop_request.set(None);
                    self.busy.set(false);
                    self.message
                        .set("已取消文件导入，未插入页面。后台读取即使结束也不会提交。".into());
                }
                Ok(false) => self
                    .message
                    .set("导入已结束，正在同步实际结果；未确认取消。".into()),
                Err(error) => self.message.set(error),
            }
        });
    }
}
