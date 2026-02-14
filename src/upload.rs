use crate::{Intermediate, Message, Preview, Step, error::Error};
use gloo_timers::future::TimeoutFuture;
use iced::{
    Element, Task, task,
    time::Instant,
    widget::{column, progress_bar, text},
};
use image::{ImageReader, imageops};
use rfd::AsyncFileDialog;
use sipper::{Straw, sipper};

use std::{io::Cursor, sync::Arc};

#[derive(Debug, Clone)]
pub(crate) enum Update {
    Uploading(Progress),
    Finished(Result<Box<Option<Intermediate>>, Error>),
}

pub(crate) enum State {
    Idle,
    Uploading {
        progress: Progress,
        _task: task::Handle,
    },
    Finished(Box<Intermediate>),
    Errored,
}

pub(crate) struct Upload {
    pub(crate) state: State,
}

#[derive(Debug, Clone)]
pub(crate) enum Progress {
    Progress(f32),
    Resizing,
}

impl Upload {
    pub(crate) fn new() -> Self {
        Self { state: State::Idle }
    }

    pub(crate) fn is_animating(&self, now: Instant) -> bool {
        match &self.state {
            State::Finished(i) => i.preview.is_animating(now),
            _ => false,
        }
    }

    pub(crate) fn start(&mut self) -> Task<Update> {
        match self.state {
            State::Idle | State::Finished(..) | State::Errored => {
                let (task, handle) =
                    Task::sip(upload(), Update::Uploading, Update::Finished).abortable();
                self.state = State::Uploading {
                    progress: Progress::Progress(0.0),
                    _task: handle.abort_on_drop(),
                };
                task
            }
            State::Uploading { .. } => Task::none(),
        }
    }

    pub(crate) fn update(&mut self, update: Update) {
        if let State::Uploading { progress, .. } = &mut self.state {
            match update {
                Update::Uploading(new_progress) => {
                    *progress = new_progress;
                }
                Update::Finished(result) => {
                    self.state = if let Ok(boxed_opt) = result {
                        match *boxed_opt {
                            Some(i) => State::Finished(Box::new(i)),
                            None => State::Idle,
                        }
                    } else {
                        State::Errored
                    };
                }
            }
        }
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        match &self.state {
            State::Idle => column![progress_bar(0.0..=100.0, 0.0)],
            State::Uploading { progress, .. } => match progress {
                Progress::Progress(p) => {
                    column![progress_bar(0.0..=100.0, *p), text!("Uploading: {p}%")]
                }
                Progress::Resizing => {
                    column![progress_bar(0.0..=100.0, 100.0), text!("Resizing...")]
                }
            },

            State::Finished(..) => column![progress_bar(0.0..=100.0, 100.0), text!("Done.")],
            State::Errored => column![
                progress_bar(0.0..=100.0, 0.0),
                text!("Something went wrong.")
            ],
        }
        .into()
    }
}

/// Upload and resizing the image.
/// Returns an `Option<Intermediate>` which is `None` if the upload was cancelled.
///
/// Typically, this should be split into two functions to avoid [long method](https://refactoring.guru/smells/long-method),
/// but for iced, there's no proper [`Task`](https://docs.iced.rs/iced/struct.Task.html#implementations) type for such two consecutive operations.
pub(crate) fn upload() -> impl Straw<Box<Option<Intermediate>>, Progress, Error> {
    sipper(async move |mut progress| {
        if let Some(file) = AsyncFileDialog::new().pick_file().await {
            let js_file = file.inner();

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let total_size = js_file.size() as usize;
            if total_size == 0 {
                todo!()
            }
            let () = progress.send(Progress::Progress(0.0)).await;

            let mut loaded = 0;
            let mut buffer = Vec::with_capacity(total_size);

            let chunk_size = match total_size {
                0..=500_000 => 16 * 1024,         // Small:   16KB
                500_001..=5_000_000 => 64 * 1024, // Medium:  64KB
                _ => 128 * 1024,                  // Large:   256KB
            };
            let mut start = 0;

            while start < total_size {
                let end = (start + chunk_size).min(total_size);
                let chunk = unsafe {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    js_file
                        .slice_with_i32_and_i32(start as i32, end as i32)
                        .unwrap_unchecked()
                };
                let array_buffer = wasm_bindgen_futures::JsFuture::from(chunk.array_buffer())
                    .await
                    .map_err(|js_value| {
                        let error_str = js_value
                            .as_string()
                            .unwrap_or_else(|| format!("Unrecognized JS error: {js_value:?}"));
                        Error::Read(error_str)
                    })?;
                let chunk_data = js_sys::Uint8Array::new(&array_buffer).to_vec();

                buffer.extend_from_slice(&chunk_data);
                loaded += chunk_data.len();
                start = end;

                if loaded % chunk_size == 0 || loaded == total_size {
                    let () = progress
                        .send(Progress::Progress(
                            #[allow(clippy::cast_precision_loss)]
                            {
                                loaded as f32 / total_size as f32 * 100.0
                            },
                        ))
                        .await;
                }
            }

            // Here, the rendering will be blocked since there's a heavy calculation though the signal has been sent.
            // See https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Execution_model
            // The ugly solution maybe can be solved by [`wasm_bindgen_spawn`](https://docs.rs/wasm-bindgen-spawn/latest/wasm_bindgen_spawn)
            TimeoutFuture::new(100).await;
            let () = progress.send(Progress::Resizing).await;
            TimeoutFuture::new(200).await;

            let mut focal_length_px = None;

            // EXIF Parsing
            let exif_reader = exif::Reader::new();
            if let Ok(exif_data) = exif_reader.read_from_container(&mut Cursor::new(&buffer)) {
                use exif::{In, Tag};

                // Get Focal Length
                let fl_val = exif_data
                    .get_field(Tag::FocalLength, In::PRIMARY)
                    .and_then(|f| f.value.get_uint(0));

                // Get Resolution (PlaneXResolution or estimate from 35mm equiv)
                // If FocalPlaneXResolution exists (pixels per unit), we need FocalPlaneResolutionUnit (2=inch, 3=cm).
                // Standard is usually inches.

                // Strategy A: FocalPlaneXResolution
                let _resolution_val = exif_data
                    .get_field(Tag::FocalPlaneXResolution, In::PRIMARY)
                    .and_then(|f| f.value.get_uint(0));

                let _res_unit_val = exif_data
                    .get_field(Tag::FocalPlaneResolutionUnit, In::PRIMARY)
                    .and_then(|f| f.value.get_uint(0))
                    .unwrap_or(2); // Default to inches

                if let Some(_fl_mm_u32) = fl_val {
                    // Note: EXIF FocalLength is usually rational, get_uint might fail if it's not integer?
                    // kamadak-exif handles rational to uint/float conversion via helper methods usually,
                    // but `get_uint` returns u32. Let's try to get float to be safe.
                    // Actually `get_uint` works if value is integer. Rationals are common.
                    // Better use match on value.
                }

                // Simplified parsing using display value parsing or direct value access
                // Let's iterate fields to find them more robustly or use robust accessors.

                // Re-implementation with correct kamadak-exif usage
                let focal_len = exif_data
                    .get_field(Tag::FocalLength, In::PRIMARY)
                    .and_then(|f| match f.value {
                        exif::Value::Rational(ref v) if !v.is_empty() => Some(v[0].to_f64()),
                        exif::Value::Float(ref v) if !v.is_empty() => Some(v[0] as f64),
                        _ => None,
                    });

                let plane_x_res = exif_data
                    .get_field(Tag::FocalPlaneXResolution, In::PRIMARY)
                    .and_then(|f| match f.value {
                        exif::Value::Rational(ref v) if !v.is_empty() => Some(v[0].to_f64()),
                        exif::Value::Float(ref v) if !v.is_empty() => Some(v[0] as f64),
                        _ => None,
                    });

                let res_unit = exif_data
                    .get_field(Tag::FocalPlaneResolutionUnit, In::PRIMARY)
                    .and_then(|f| match f.value {
                        exif::Value::Short(ref v) if !v.is_empty() => Some(v[0]),
                        _ => None,
                    })
                    .unwrap_or(2); // 2 = inches, 3 = cm

                if let Some(fl) = focal_len {
                    if let Some(res) = plane_x_res {
                        // Focal Length (mm) * Resolution (px/unit) * UnitConversion
                        // Unit 2 (Inch): res is px/inch.  1 inch = 25.4 mm.
                        // f_px = fl_mm * (res / 25.4)
                        let conversion = match res_unit {
                            2 => 1.0 / 25.4,
                            3 => 1.0 / 10.0,
                            _ => 1.0 / 25.4, // Assume inch if unknown
                        };
                        focal_length_px = Some((fl * res * conversion) as f32);

                        log::info!(
                            "EXIF: FL={}mm, Res={}, Unit={}, f_px={}",
                            fl,
                            res,
                            res_unit,
                            focal_length_px.unwrap()
                        );
                    } else {
                        // Strategy B: 35mm Equivalent
                        // If we know it's 35mm equiv, we need sensor size...
                        // Without sensor size or resolution plane, we can't get accurate pixels.
                        // But maybe we can guess if we assume standard sensor width? No, too risky.
                        // Many phones write FocalPlaneXResolution.

                        // Fallback: If 35mm equiv is known, and we assume standard 36mm width for 35mm film...
                        // fl_35 / 36mm = f_px / image_width_px
                        // => f_px = fl_35 * image_width_px / 36.0
                        let fl_35 = exif_data
                            .get_field(Tag::FocalLengthIn35mmFilm, In::PRIMARY)
                            .and_then(|f| match f.value {
                                exif::Value::Short(ref v) if !v.is_empty() => Some(v[0] as f64),
                                exif::Value::Long(ref v) if !v.is_empty() => Some(v[0] as f64),
                                _ => None,
                            });

                        log::info!("[Upload] Checking 35mm equiv: {:?}", fl_35);

                        if let Some(fl35) = fl_35 {
                            // We need original image width.
                            let w = unsafe {
                                ImageReader::new(Cursor::new(&buffer))
                                    .with_guessed_format()
                                    .unwrap_unchecked()
                                    .into_dimensions()
                            }
                            .ok()
                            .map(|(w, _)| w);

                            if let Some(width) = w {
                                focal_length_px = Some((fl35 * width as f64 / 36.0) as f32);
                                log::info!(
                                    "EXIF (35mm): FL35={}, W={}, f_px={}",
                                    fl35,
                                    width,
                                    focal_length_px.unwrap()
                                );
                            }
                        }
                    }
                } else {
                    log::info!("[Upload] No FocalLength found in EXIF.");
                }
            }

            let original_high_res = unsafe {
                ImageReader::new(Cursor::new(buffer))
                    .with_guessed_format()
                    .unwrap_unchecked()
            }
            .decode()?;

            let image = original_high_res.resize(1024, 1024, imageops::Lanczos3);

            let preview = Preview::ready(image, Instant::now());

            Ok(Box::new(Some(Intermediate {
                current_step: Step::Original,
                preview,
                pixels_per_mm: None,
                context_image: None,
                roi_image: None,
                original_high_res: Some(Arc::new(original_high_res)),
                focal_length_px,
                transform: None,
            })))
        } else {
            Ok(Box::new(None))
        }
    })
}
