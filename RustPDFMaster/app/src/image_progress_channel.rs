use super::{Document, bridge};
use pdfmaster_ui::image_import::{Layout, ProgressNotice};
use serde::Serialize;
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

fn js_error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

pub(super) async fn convert(
    id: u32,
    order: Vec<usize>,
    layout: Layout,
    mut observe: impl FnMut(ProgressNotice) + 'static,
) -> Result<Document, String> {
    let arguments = Arguments { id, order, layout };
    let args = arguments
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| error.to_string())?;
    let active = Rc::new(Cell::new(true));
    let callback_active = active.clone();
    let callback = Closure::<dyn FnMut(JsValue)>::new(move |payload| {
        if callback_active.get()
            && let Ok(notice) = serde_wasm_bindgen::from_value::<ProgressNotice>(payload)
        {
            observe(notice);
        }
    })
    .into_js_value();
    let Ok(channel) = NativeChannel::new(callback.unchecked_ref()) else {
        active.set(false);
        return bridge::call("convert_image_import", &arguments).await;
    };
    let observation = Observation { channel, active };
    if !matches!(
        js_sys::Reflect::set(
            &args,
            &JsValue::from_str("on_progress"),
            &observation.channel,
        ),
        Ok(true)
    ) {
        drop(observation);
        return bridge::call("convert_image_import", &arguments).await;
    }

    let reply = invoke("convert_image_import", args).await;
    drop(observation);
    let reply = reply.map_err(js_error)?;
    serde_wasm_bindgen::from_value(reply).map_err(|error| format!("响应格式错误：{error}"))
}
