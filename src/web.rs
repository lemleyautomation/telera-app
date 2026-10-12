//! Browser-only glue for the wasm32 build: console logging and the fallback
//! font download. Everything else web-specific lives behind
//! `cfg(target_arch = "wasm32")` next to its native counterpart in `lib.rs`.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use winit::event_loop::EventLoopProxy;

use crate::InternalEvents;

/// `console.error(message)`. Backs the `log_error!` macro in a browser, where
/// stderr goes nowhere.
pub(crate) fn console_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

/// Downloads `url` in the background and hands the bytes back to the event
/// loop as `InternalEvents::FallbackFontLoaded`. A failed download is logged
/// to the console and otherwise ignored: the app keeps running, just without
/// visible text.
pub(crate) fn spawn_font_fetch(url: String, app_events: EventLoopProxy<InternalEvents>) {
    wasm_bindgen_futures::spawn_local(async move {
        match fetch_bytes(&url).await {
            Ok(bytes) => {
                let _ = app_events.send_event(InternalEvents::FallbackFontLoaded(bytes));
            }
            Err(error) => console_error(&format!("failed to download fallback font {url}: {error}")),
        }
    });
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no global `window`")?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(js_error)?
        .dyn_into()
        .map_err(js_error)?;
    if !response.ok() {
        return Err(format!("HTTP {} {}", response.status(), response.status_text()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

fn js_error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
