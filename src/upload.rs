use crate::{Intermediate, Message, Preview, Step, error::Error, js_interop::FileEntry};
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
    Finished(Result<Box<Vec<FileEntry>>, Error>),
}

pub(crate) enum State {
    Idle,
    Uploading {
        progress: Progress,
        _task: task::Handle,
    },
    Finished(Vec<FileEntry>),
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

    pub(crate) fn is_animating(&self, _now: Instant) -> bool {
        false
    }

    pub(crate) fn start_files(&mut self) -> Task<Update> {
        match self.state {
            State::Idle | State::Finished(..) | State::Errored => {
                let (task, handle) =
                    Task::sip(upload_files(), Update::Uploading, Update::Finished).abortable();
                self.state = State::Uploading {
                    progress: Progress::Progress(0.0),
                    _task: handle.abort_on_drop(),
                };
                task
            }
            State::Uploading { .. } => Task::none(),
        }
    }

    pub(crate) fn start_directory(&mut self) -> Task<Update> {
        match self.state {
            State::Idle | State::Finished(..) | State::Errored => {
                let (task, handle) =
                    Task::sip(upload_directory(), Update::Uploading, Update::Finished).abortable();
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
                    self.state = if let Ok(boxed_entries) = result {
                        let entries = *boxed_entries;
                        if entries.is_empty() {
                            State::Idle
                        } else {
                            State::Finished(entries)
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
                    column![progress_bar(0.0..=100.0, 100.0), text!("Loading...")]
                }
            },
            State::Finished(entries) => column![
                progress_bar(0.0..=100.0, 100.0),
                text!("{} file(s) loaded.", entries.len())
            ],
            State::Errored => column![
                progress_bar(0.0..=100.0, 0.0),
                text!("Something went wrong.")
            ],
        }
        .into()
    }
}

/// Upload multiple files via the browser file picker.
pub(crate) fn upload_files() -> impl Straw<Box<Vec<FileEntry>>, Progress, Error> {
    sipper(async move |mut progress| {
        if let Some(files) = AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "tiff", "tif"])
            .pick_files()
            .await
        {
            let total = files.len();
            let mut entries = Vec::with_capacity(total);

            for (i, file) in files.into_iter().enumerate() {
                let name = file.file_name();
                let data = file.read().await;

                entries.push(FileEntry { name, data });

                #[allow(clippy::cast_precision_loss)]
                let pct = ((i + 1) as f32 / total as f32) * 100.0;
                let () = progress.send(Progress::Progress(pct)).await;
            }

            Ok(Box::new(entries))
        } else {
            Ok(Box::new(vec![]))
        }
    })
}

/// Upload files from a directory via `showDirectoryPicker()`.
pub(crate) fn upload_directory() -> impl Straw<Box<Vec<FileEntry>>, Progress, Error> {
    sipper(async move |mut progress| {
        let () = progress.send(Progress::Resizing).await;

        let entries = crate::js_interop::pick_directory_files().await?;

        Ok(Box::new(entries))
    })
}

/// Decode a `FileEntry` into a resized `Intermediate` for single-image debug mode.
///
/// This replicates the old single-file upload behavior where an image is decoded,
/// resized and wrapped in an `Intermediate` ready for pipeline stepping.
pub(crate) fn decode_to_intermediate(entry: &FileEntry) -> Result<Intermediate, Error> {
    let original_high_res = ImageReader::new(Cursor::new(&entry.data))
        .with_guessed_format()
        .expect("Image format detection failed")
        .decode()?;

    let image = original_high_res.resize(1024, 1024, imageops::Lanczos3);
    let preview = Preview::ready(image, Instant::now());

    Ok(Intermediate {
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
        horiz_contour: None,
        horiz_rect_metrics: None,
        scale_factor: None,
    })
}
