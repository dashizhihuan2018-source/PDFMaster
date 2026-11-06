use super::{Document, bridge};
use pdfmaster_ui::image_import::{Layout, ProgressNotice};
use serde::{Deserialize, Serialize};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window","__TAURI__","core"],js_name=Channel)]
    type NativeChannel;
    #[wasm_bindgen(constructor,catch,js_namespace = ["window","__TAURI__","core"],js_class=Channel)]
    fn new(callback: &js_sys::Function) -> Result<NativeChannel, JsValue>;
    #[wasm_bindgen(catch,method,setter=onmessage)]
    fn set_onmessage(this: &NativeChannel, callback: &js_sys::Function) -> Result<(), JsValue>;
    #[wasm_bindgen(catch,js_namespace = ["window","__TAURI__","core"],js_name=invoke)]
    async fn invoke(command: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

struct Observation {
    channel: NativeChannel,
    active: Rc<Cell<bool>>,
}

impl Observation {
    fn new(mut observe: impl FnMut(JsValue) + 'static) -> Result<Self, ()> {
        let active = Rc::new(Cell::new(true));
        let callback_active = active.clone();
        let callback = Closure::<dyn FnMut(JsValue)>::new(move |payload| {
            if callback_active.get() {
                observe(payload);
            }
        })
        .into_js_value();
        let channel = NativeChannel::new(callback.unchecked_ref()).map_err(|_| {
            active.set(false);
        })?;
        Ok(Self { channel, active })
    }

    fn attach(self, args: &JsValue, key: &str) -> Result<Self, ()> {
        if matches!(
            js_sys::Reflect::set(args, &JsValue::from_str(key), &self.channel),
            Ok(true)
        ) {
            Ok(self)
        } else {
            Err(())
        }
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.active.set(false);
        let noop = Closure::<dyn FnMut(JsValue)>::new(|_| {}).into_js_value();
        let _ = self.channel.set_onmessage(noop.unchecked_ref());
    }
}

#[derive(Serialize)]
struct Arguments {
    id: u32,
    order: Vec<usize>,
    layout: Layout,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelNotice {
    id: u32,
}

fn cancel_notice(payload: JsValue) -> Option<CancelNotice> {
    let object = payload.dyn_ref::<js_sys::Object>()?;
    let keys = js_sys::Object::keys(object);
    if keys.length() != 1 || keys.get(0).as_string().as_deref() != Some("id") {
        return None;
    }
    serde_wasm_bindgen::from_value(payload).ok()
}

fn js_error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

pub(super) async fn convert(
    id: u32,
    order: Vec<usize>,
    layout: Layout,
    mut observe: impl FnMut(ProgressNotice) + 'static,
    mut cancel: impl FnMut(u32) + 'static,
) -> Result<Document, String> {
    let arguments = Arguments { id, order, layout };
    let args = arguments
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| error.to_string())?;
    let Ok(progress_observation) = Observation::new(move |payload| {
        if let Ok(notice) = serde_wasm_bindgen::from_value::<ProgressNotice>(payload) {
            observe(notice);
        }
    }) else {
        return bridge::call("convert_image_import", &arguments).await;
    };
    let Ok(progress_observation) = progress_observation.attach(&args, "on_progress") else {
        return bridge::call("convert_image_import", &arguments).await;
    };

    let cancel_observation = Observation::new(move |payload| {
        if let Some(notice) = cancel_notice(payload)
            && notice.id == id
        {
            cancel(id);
        }
    })
    .and_then(|observation| observation.attach(&args, "on_cancel"))
    .ok();

    let reply = invoke("convert_image_import", args).await;
    drop(cancel_observation);
    drop(progress_observation);
    let reply = reply.map_err(js_error)?;
    serde_wasm_bindgen::from_value(reply).map_err(|error| format!("响应格式错误：{error}"))
}

#[cfg(test)]
mod tests {
    use super::CancelNotice;

    #[test]
    fn cancel_notice_accepts_only_an_exact_numeric_id() {
        let notice: CancelNotice = serde_json::from_value(serde_json::json!({"id": 7})).unwrap();
        assert_eq!(notice.id, 7);
        for invalid in [
            serde_json::json!("bad"),
            serde_json::json!({"id": "7"}),
            serde_json::json!({"id": 7, "extra": true}),
            serde_json::json!({}),
        ] {
            assert!(serde_json::from_value::<CancelNotice>(invalid).is_err());
        }
    }
}
