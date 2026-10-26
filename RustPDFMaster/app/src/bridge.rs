use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    async fn tauri_invoke(command: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

pub async fn call<T: DeserializeOwned>(command: &str, args: &impl Serialize) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let value = tauri_invoke(command, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("响应格式错误：{e}"))
}

/// Timeout abandons observation, not the native mutation. Ink callers retain
/// their request identity and reconcile/cancel it on the native side before
/// admitting another stroke. The losing Promise never updates UI state.
pub async fn call_bounded<T: DeserializeOwned>(
    command: &str,
    args: &impl Serialize,
) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let command = command.to_owned();
    let mut reject_timeout = None;
    let timeout = js_sys::Promise::new(&mut |_, reject| {
        reject_timeout = Some(reject);
    });
    let reject = reject_timeout.ok_or("无法建立响应超时保护。")?;
    let callback = Closure::once(move || {
        let _ = reject.call1(
            &JsValue::NULL,
            &JsValue::from_str("原生响应超时；结果未知，请重试或取消／同步手绘。"),
        );
    });
    let window = web_sys::window().ok_or("窗口不可用。")?;
    let timer = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            15_000,
        )
        .map_err(|_| "无法建立响应超时保护。")?;
    let rpc =
        wasm_bindgen_futures::future_to_promise(async move { tauri_invoke(&command, args).await });
    let promises = js_sys::Array::new();
    promises.push(&rpc);
    promises.push(&timeout);
    let result = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::race(&promises)).await;
    window.clear_timeout_with_handle(timer);
    drop(callback);
    let value = result.map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("响应格式错误：{e}"))
}
