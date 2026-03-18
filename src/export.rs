//! CSV export and browser file download for batch results.

use wasm_bindgen::JsCast;

use crate::job::{Job, JobStatus};

/// Build a CSV string from completed jobs.
///
/// Columns: filename, major_length_mm, minor_length_mm, volume_mm3,
///          a_eq_mm, b_eq_mm, surface_area_mm2, n_total
pub(crate) fn jobs_to_csv(jobs: &[Job]) -> String {
    let mut csv =
        String::from("filename,major_length_mm,minor_length_mm,volume_mm3,a_eq_mm,b_eq_mm,surface_area_mm2,n_total\n");

    for job in jobs {
        if job.status != JobStatus::Done {
            continue;
        }
        if let Some(m) = &job.metrics {
            csv.push_str(&format!(
                "{},{:.2},{:.2},{:.2},{},{},{},{}\n",
                job.filename,
                m.major_length,
                m.minor_length,
                m.volume,
                m.a_eq.map_or("-".to_string(), |v| format!("{v:.2}")),
                m.b_eq.map_or("-".to_string(), |v| format!("{v:.2}")),
                m.surface_area.map_or("-".to_string(), |v| format!("{v:.2}")),
                m.n_total.map_or("-".to_string(), |v| format!("{v}")),
            ));
        }
    }

    csv
}

/// Trigger a browser file download of the given text content.
///
/// Uses pure `web-sys` + `js-sys` to create a Blob, generate an object URL,
/// and click a hidden `<a>` element to trigger the download.
pub(crate) fn trigger_download(content: &str, filename: &str) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            log::error!("trigger_download: no window");
            return;
        }
    };
    let document = match window.document() {
        Some(d) => d,
        None => {
            log::error!("trigger_download: no document");
            return;
        }
    };

    // Create a JS Blob from the CSV string
    let array = js_sys::Array::new();
    array.push(&wasm_bindgen::JsValue::from_str(content));

    let blob_opts = web_sys::BlobPropertyBag::new();
    blob_opts.set_type("text/csv;charset=utf-8");

    let blob = match web_sys::Blob::new_with_str_sequence_and_options(&array, &blob_opts) {
        Ok(b) => b,
        Err(e) => {
            log::error!("trigger_download: failed to create Blob: {e:?}");
            return;
        }
    };

    // Create an object URL for the blob
    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(e) => {
            log::error!("trigger_download: failed to create object URL: {e:?}");
            return;
        }
    };

    // Create a temporary <a> element, click it, then clean up
    if let Ok(elem) = document.create_element("a") {
        let _ = elem.set_attribute("href", &url);
        let _ = elem.set_attribute("download", filename);
        let _ = elem.set_attribute("style", "display:none");

        if let Some(body) = document.body() {
            let _ = body.append_child(&elem);
            if let Some(html_elem) = elem.dyn_ref::<web_sys::HtmlElement>() {
                html_elem.click();
            }
            let _ = body.remove_child(&elem);
        }
    }

    // Clean up the object URL
    let _ = web_sys::Url::revoke_object_url(&url);
}
