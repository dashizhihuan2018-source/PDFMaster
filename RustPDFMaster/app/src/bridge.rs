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
