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
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let chunk = js_file
                    .slice_with_i32_and_i32(start as i32, end as i32)
                    .expect("File.slice failed");
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

            if total_size == 0 {
                return Err(Error::General("Selected file is empty".into()));
            }

            // Single decode: avoids redundant ImageReader parse for dimensions + full decode
            let original_high_res = ImageReader::new(Cursor::new(buffer))
                .with_guessed_format()
                .expect("Image format detection failed")
                .decode()?;

            let image = original_high_res.resize(1024, 1024, imageops::Lanczos3);

            let preview = Preview::ready(image, Instant::now());

            Ok(Box::new(Some(Intermediate {
                current_step: Step::Original,
                preview,
                pixels_per_mm: None,
                binary_image: None,
                fused_image: None,
                contours: None,
                context_image: None,
                roi_image: None,
                original_high_res: Some(Arc::new(original_high_res)),
                transform: None,
                metrics: None,
            })))
        } else {
            Ok(Box::new(None))
        }
    })
}
