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

// ──────────────────────────────  Camera Mode Persistence  ──────────────────────────────

const CAMERA_MODE_KEY: &str = "pineapplehub_camera_mode";
const CAMERA_APPEND_SESSION_KEY: &str = "pineapplehub_camera_append_session";

/// Save the selected camera session mode to localStorage.
/// `mode` should be one of: `"new"`, `"append"`, `"standalone"`.
pub(crate) fn save_camera_mode(mode: &str) {
    let Some(window) = web_sys::window() else { return };
    let Ok(Some(storage)) = window.local_storage() else { return };
    let _ = storage.set_item(CAMERA_MODE_KEY, mode);
}

/// Load the previously saved camera session mode from localStorage.
/// Returns `None` if nothing was saved yet.
pub(crate) fn load_camera_mode() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item(CAMERA_MODE_KEY).ok()?
}

/// Save the session_id for the "append to existing" camera mode.
pub(crate) fn save_camera_append_session(session_id: &str) {
    let Some(window) = web_sys::window() else { return };
    let Ok(Some(storage)) = window.local_storage() else { return };
    let _ = storage.set_item(CAMERA_APPEND_SESSION_KEY, session_id);
}

/// Load the previously saved append session_id from localStorage.
pub(crate) fn load_camera_append_session() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item(CAMERA_APPEND_SESSION_KEY).ok()?
}

// ──────────────────────────────  Camera Capture  ──────────────────────────────

/// Capture a photo using the device camera via an HTML file input.
///
/// Creates a hidden `<input type="file" accept="image/*" capture="environment">`
/// element and programmatically clicks it, triggering the **native system camera
/// app** on mobile devices. When the user takes (or selects) a photo the input's
/// `change` event fires and the selected file is returned as a [`FileEntry`].
///
/// This approach is preferred over `getUserMedia` + canvas because:
/// - It invokes the proper system camera UI (viewfinder, shutter, etc.)
/// - It works over plain HTTP (no HTTPS requirement)
/// - It degrades gracefully on desktop (opens the file picker instead)
///
/// Returns `Err` if the user cancels or the selected file cannot be read.
pub(crate) async fn capture_photo() -> Result<FileEntry, Error> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().ok_or_else(|| Error::General("No window".into()))?;
    let document = window
        .document()
        .ok_or_else(|| Error::General("No document".into()))?;

    // Create a hidden <input type="file" accept="image/*" capture="environment">
    let input: web_sys::HtmlInputElement = document
        .create_element("input")
        .map_err(|e| Error::General(format!("create input failed: {e:?}")))?
        .dyn_into()
        .map_err(|_| Error::General("cast to HtmlInputElement failed".into()))?;
    input.set_attribute("type", "file").ok();
    input.set_attribute("accept", "image/*").ok();
    // capture="environment" → rear camera; gracefully ignored on desktop
    input.set_attribute("capture", "environment").ok();
    input.style().set_property("display", "none").ok();

    // Append to body so the input is reachable by the browser
    document
        .body()
        .ok_or_else(|| Error::General("No body".into()))?
        .append_child(&input)
        .map_err(|e| Error::General(format!("append failed: {e:?}")))?;

    // Channel: JS change event → Rust async
    let (file_tx, file_rx) = futures::channel::oneshot::channel::<Option<web_sys::File>>();
    let file_tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(file_tx)));

    let input_clone = input.clone();
    let change_closure = wasm_bindgen::closure::Closure::once(move |_: web_sys::Event| {
        let file = input_clone
            .files()
            .and_then(|list| list.get(0));
        if let Some(tx) = file_tx_cell.borrow_mut().take() {
            let _ = tx.send(file);
        }
    });
    input
        .add_event_listener_with_callback("change", change_closure.as_ref().unchecked_ref())
        .ok();
    change_closure.forget();

    // Programmatically open the camera / file picker
    input.click();

    // Await user action
    let maybe_file = file_rx
        .await
        .map_err(|_| Error::General("File channel dropped".into()))?;

    // Clean up DOM
    input.remove();

    let file = maybe_file.ok_or_else(|| {
        Error::General("No photo selected (cancelled).".into())
    })?;

    // Build file name
    let raw_name = file.name();
    let ts = js_sys::Date::now() as u64;
    let name = if raw_name.is_empty() {
        format!("capture_{ts}.jpg")
    } else {
        raw_name
    };

    // Downscale in the browser to avoid WASM OOM on large camera photos
    let data = resize_image_blob(&file, 2048).await?;

    Ok(FileEntry { name, data })
}

/// Maximum pixel dimension for images before they enter WASM.
///
/// Camera phones produce 12MP+ photos (4032×3024).  Decoding one in WASM
/// costs ~65 MB of heap — enough to OOM on mobile browsers.  By resizing
/// to `max_dim` in the **browser's native image pipeline** (zero WASM
/// heap cost) and re-encoding as JPEG, we cut WASM peak memory by >50%.
///
/// The pipeline's `prepare_image` further downscales to 1024px for
/// calibration, so 3072px provides plenty of headroom for the high-res
/// fruitlet-counting path without risking OOM.
async fn resize_image_blob(file: &web_sys::File, max_dim: u32) -> Result<Vec<u8>, Error> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().ok_or_else(|| Error::General("No window".into()))?;
    let document = window.document().ok_or_else(|| Error::General("No doc".into()))?;

    // 1. Create a Blob URL and load it into an <img> to get dimensions
    let blob: &web_sys::Blob = file.as_ref(); // File extends Blob
    let url = web_sys::Url::create_object_url_with_blob(blob)
        .map_err(|e| Error::General(format!("createObjectURL: {e:?}")))?;

    let img: web_sys::HtmlImageElement = document
        .create_element("img")
        .map_err(|e| Error::General(format!("create img: {e:?}")))?
        .dyn_into()
        .map_err(|_| Error::General("cast to HtmlImageElement".into()))?;

    // Wait for the image to load
    let (load_tx, load_rx) = futures::channel::oneshot::channel::<Result<(), Error>>();
    let load_tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(load_tx)));

    let err_cell = load_tx_cell.clone();
    let onerror = wasm_bindgen::closure::Closure::once(move |_: web_sys::Event| {
        if let Some(tx) = err_cell.borrow_mut().take() {
            let _ = tx.send(Err(Error::General("Image load failed".into())));
        }
    });
    let onload = wasm_bindgen::closure::Closure::once(move |_: web_sys::Event| {
        if let Some(tx) = load_tx_cell.borrow_mut().take() {
            let _ = tx.send(Ok(()));
        }
    });
    img.set_onload(Some(onload.as_ref().unchecked_ref()));
    img.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onload.forget();
    onerror.forget();

    img.set_src(&url);
    load_rx.await.map_err(|_| Error::General("Load channel dropped".into()))??;

    let orig_w = img.natural_width();
    let orig_h = img.natural_height();

    // 2. Compute target dimensions (preserve aspect ratio)
    let (draw_w, draw_h) = if orig_w <= max_dim && orig_h <= max_dim {
        // Already small enough — skip resize, just read raw bytes
        web_sys::Url::revoke_object_url(&url).ok();
        return read_file_bytes(file).await;
    } else {
        let scale = max_dim as f64 / orig_w.max(orig_h) as f64;
        (
            (orig_w as f64 * scale).round() as u32,
            (orig_h as f64 * scale).round() as u32,
        )
    };

    // 3. Draw onto a canvas at the target size
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|e| Error::General(format!("create canvas: {e:?}")))?
        .dyn_into()
        .map_err(|_| Error::General("cast to canvas".into()))?;
    canvas.set_width(draw_w);
    canvas.set_height(draw_h);

    let ctx: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|e| Error::General(format!("getContext: {e:?}")))?
        .ok_or_else(|| Error::General("no 2d context".into()))?
        .dyn_into()
        .map_err(|_| Error::General("cast to 2d ctx".into()))?;

    ctx.draw_image_with_html_image_element_and_dw_and_dh(
        &img, 0.0, 0.0, draw_w as f64, draw_h as f64,
    )
    .map_err(|e| Error::General(format!("drawImage: {e:?}")))?;

    web_sys::Url::revoke_object_url(&url).ok();

    // 4. Export as JPEG blob → bytes
    let (blob_tx, blob_rx) = futures::channel::oneshot::channel::<Result<JsValue, Error>>();
    let blob_tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(blob_tx)));

    let cb = wasm_bindgen::closure::Closure::once(move |blob: JsValue| {
        let result = if blob.is_null() || blob.is_undefined() {
            Err(Error::General("toBlob returned null".into()))
        } else {
            // Read the blob via Response.arrayBuffer() — simpler than FileReader
            Ok(blob)
        };
        if let Some(tx) = blob_tx_cell.borrow_mut().take() {
            let _ = tx.send(result);
        }
    });

    canvas
        .to_blob_with_type_and_encoder_options(
            cb.as_ref().unchecked_ref(),
            "image/jpeg",
            &JsValue::from_f64(0.92),
        )
        .map_err(|e| Error::General(format!("toBlob: {e:?}")))?;
    cb.forget();

    let blob_js = blob_rx
        .await
        .map_err(|_| Error::General("Blob channel dropped".into()))??;

    // Convert blob to ArrayBuffer via Response API (clean one-liner)
    let resp = web_sys::Response::new_with_opt_blob(Some(
        &blob_js.dyn_into::<web_sys::Blob>()
            .map_err(|_| Error::General("cast to Blob".into()))?,
    ))
    .map_err(|e| Error::General(format!("Response::new: {e:?}")))?;

    let ab_promise = resp
        .array_buffer()
        .map_err(|e| Error::General(format!("arrayBuffer(): {e:?}")))?;
    let ab = JsFuture::from(ab_promise)
        .await
        .map_err(|e| Error::General(format!("await arrayBuffer: {e:?}")))?;

    let bytes = js_sys::Uint8Array::new(&ab).to_vec();

    log::info!(
        "resize_image_blob: {}×{} → {}×{}, {} KB JPEG",
        orig_w, orig_h, draw_w, draw_h, bytes.len() / 1024
    );

    Ok(bytes)
}

/// Read a File's raw bytes via FileReader (no resize).
async fn read_file_bytes(file: &web_sys::File) -> Result<Vec<u8>, Error> {
    use wasm_bindgen::JsCast;

    let (buf_tx, buf_rx) = futures::channel::oneshot::channel::<Result<Vec<u8>, Error>>();
    let buf_tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(buf_tx)));

    let reader = web_sys::FileReader::new()
        .map_err(|e| Error::General(format!("FileReader::new() failed: {e:?}")))?;

    let reader_clone = reader.clone();
    let load_closure = wasm_bindgen::closure::Closure::once(move |_: web_sys::Event| {
        let bytes = reader_clone
            .result()
            .ok()
            .map(|v| js_sys::Uint8Array::new(&v).to_vec());
        if let Some(tx) = buf_tx_cell.borrow_mut().take() {
            match bytes {
                Some(d) => { let _ = tx.send(Ok(d)); }
                None    => { let _ = tx.send(Err(Error::General("FileReader result empty".into()))); }
            }
        }
    });
    reader
        .add_event_listener_with_callback("load", load_closure.as_ref().unchecked_ref())
        .ok();
    load_closure.forget();

    reader
        .read_as_array_buffer(file)
        .map_err(|e| Error::General(format!("readAsArrayBuffer failed: {e:?}")))?;

    buf_rx
        .await
        .map_err(|_| Error::General("Buffer channel dropped".into()))?
}

