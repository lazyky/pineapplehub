//! JavaScript interop for browser APIs not available through `rfd`.
//!
//! Provides:
//! - `showDirectoryPicker()` wrapper with file-extension filtering
//! - Mobile UA detection (for hiding unsupported UI elements)

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::error::Error;

/// A decoded file entry ready for pipeline processing.
#[derive(Clone, Debug)]
pub(crate) struct FileEntry {
    pub name: String,
    pub data: Vec<u8>,
}

const ALLOWED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "tiff", "tif"];

/// Check whether the current browser is on a mobile device.
///
/// Uses `navigator.userAgent` heuristic — sufficient for hiding
/// the "Choose Folder" button on mobile where `showDirectoryPicker` is unsupported.
pub(crate) fn is_mobile() -> bool {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    let navigator = window.navigator();
    let ua = navigator.user_agent().unwrap_or_default();
    let ua_lower = ua.to_lowercase();
    ua_lower.contains("android")
        || ua_lower.contains("iphone")
        || ua_lower.contains("ipad")
        || ua_lower.contains("mobile")
}

/// Returns `true` if `showDirectoryPicker` is available in the current browser.
pub(crate) fn has_directory_picker() -> bool {
    if is_mobile() {
        return false;
    }
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    js_sys::Reflect::has(&window, &JsValue::from_str("showDirectoryPicker")).unwrap_or(false)
}

fn has_allowed_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    ALLOWED_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// Pick a directory via the File System Access API and return all image files
/// within it (non-recursively for simplicity, then recursively via stack),
/// filtered by allowed extensions.
///
/// Returns `Ok(vec![])` if the user cancels the picker.
pub(crate) async fn pick_directory_files() -> Result<Vec<FileEntry>, Error> {
    let window = web_sys::window().ok_or_else(|| Error::General("No window object".into()))?;

    // Call window.showDirectoryPicker()
    let show_fn = js_sys::Reflect::get(&window, &JsValue::from_str("showDirectoryPicker"))
        .map_err(|_| Error::General("showDirectoryPicker not available".into()))?;

    let promise = js_sys::Reflect::apply(
        &js_sys::Function::from(show_fn),
        &window,
        &js_sys::Array::new(),
    );

    let dir_handle = match promise {
        Ok(p) => match JsFuture::from(js_sys::Promise::from(p)).await {
            Ok(h) => h,
            Err(_) => return Ok(vec![]), // User cancelled
        },
        Err(_) => return Ok(vec![]),
    };

    // Use an iterative stack instead of async recursion to avoid extra dependency
    let mut entries = Vec::new();
    let mut dir_stack = vec![dir_handle];

    while let Some(current_dir) = dir_stack.pop() {
        let iterator = call_method(&current_dir, "entries", &js_sys::Array::new())?;

        loop {
            let next_promise = call_method(&iterator, "next", &js_sys::Array::new())?;
            let result = JsFuture::from(js_sys::Promise::from(next_promise))
                .await
                .map_err(|e| Error::General(format!("Iterator next failed: {e:?}")))?;

            let done = js_sys::Reflect::get(&result, &JsValue::from_str("done"))
                .unwrap_or(JsValue::TRUE);
            if done.is_truthy() {
                break;
            }

            let value = js_sys::Reflect::get(&result, &JsValue::from_str("value"))
                .map_err(|e| Error::General(format!("No value: {e:?}")))?;

            let array = js_sys::Array::from(&value);
            let handle = array.get(1);

            let kind = js_sys::Reflect::get(&handle, &JsValue::from_str("kind"))
                .unwrap_or(JsValue::UNDEFINED)
                .as_string()
                .unwrap_or_default();

            if kind == "directory" {
                dir_stack.push(handle);
            } else if kind == "file" {
                let name = js_sys::Reflect::get(&handle, &JsValue::from_str("name"))
                    .unwrap_or(JsValue::UNDEFINED)
                    .as_string()
                    .unwrap_or_default();

                if !has_allowed_extension(&name) {
                    continue;
                }

                let file_promise = call_method(&handle, "getFile", &js_sys::Array::new())?;
                let file = JsFuture::from(js_sys::Promise::from(file_promise))
                    .await
                    .map_err(|e| Error::General(format!("getFile() failed: {e:?}")))?;

                let ab_promise =
                    call_method(&file, "arrayBuffer", &js_sys::Array::new())?;
                let array_buffer = JsFuture::from(js_sys::Promise::from(ab_promise))
                    .await
                    .map_err(|e| Error::General(format!("arrayBuffer() failed: {e:?}")))?;

                let data = js_sys::Uint8Array::new(&array_buffer).to_vec();
                entries.push(FileEntry { name, data });
            }
        }
    }

    Ok(entries)
}

/// Helper: call a method on a JS object by name.
fn call_method(obj: &JsValue, method: &str, args: &js_sys::Array) -> Result<JsValue, Error> {
    let func = js_sys::Reflect::get(obj, &JsValue::from_str(method))
        .map_err(|e| Error::General(format!("No {method}() method: {e:?}")))?;
    js_sys::Reflect::apply(&js_sys::Function::from(func), obj, args)
        .map_err(|e| Error::General(format!("{method}() call failed: {e:?}")))
}
