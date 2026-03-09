mod correction;
mod error;
mod export;
mod job;
mod js_interop;
mod pipeline;
mod ui;
mod upload;
mod utils;

// Re-export init_thread_pool so wasm-bindgen exposes `initThreadPool()` in JS.
pub use wasm_bindgen_rayon::init_thread_pool;

use crate::{
    error::Error,
    export::{jobs_to_csv, trigger_download},
    job::{Job, JobStatus},
    js_interop::FileEntry,
    pipeline::{EncodedImage, FruitletMetrics, Intermediate, Step},
    ui::{preview::Preview, viewer::Viewer},
    upload::{State, Update, Upload, decode_to_intermediate},
    utils::dynamic_image_to_handle,
};

use iced::{
    Element, Function, Length, Subscription, Task,
    time::Instant,
    widget::{
        button, column, container, grid, row, scrollable, space, stack, text,
        toggler,
    },
    window,
};

/// Noto Emoji font bytes (monochrome, variable weight).
const NOTO_EMOJI_BYTES: &[u8] = include_bytes!("../assets/NotoEmoji-Regular.ttf");
/// Noto Sans SC font bytes (CJK Simplified Chinese) for Chinese filename display.
const NOTO_SANS_SC_BYTES: &[u8] = include_bytes!("../assets/NotoSansSC-Regular.ttf");

// ────────────────────────  Messages  ────────────────────────

#[non_exhaustive]
#[derive(Debug, Clone)]
enum Message {
    // ── File input ──
    PickFiles,
    PickDirectory,
    UploadUpdated(Update),

    // ── Processing ──
    BatchStart,
    /// Deferred start: gives iced a render frame to show the decoding overlay
    /// before the synchronous Phase 1 decode blocks the main thread.
    StartDecoding,
    /// Phase 1 decode finished — dismiss the overlay.
    DecodingDone,
    /// Progress update during Phase 1 decode (current, total).
    DecodingProgress(usize, usize),
    /// Single-image debug-mode step-by-step processing
    Process(Result<Intermediate, Error>),
    BlurhashDecoded(Intermediate, EncodedImage),
    /// A single batch job finished; triggers ProcessNext for the next image.
    JobDone(usize, Result<FruitletMetrics, Error>),

    // ── UI interaction ──
    SelectJob(usize),
    TogglePipeline(bool),
    ThumbnailHovered(Step, bool),
    Open(Step),
    Close,
    Animate,
    ExportCsv,
    /// No-op (used for smoke tests)
    Noop,
}

// ────────────────────────  App State  ────────────────────────

struct App {
    upload: Upload,
    now: Instant,
    viewer: Viewer,

    /// All jobs (one per selected file).
    jobs: Vec<Job>,
    /// Index of the currently selected job for pipeline preview.
    selected_job: Option<usize>,
    /// Whether the Pipeline Preview column is visible.
    show_pipeline: bool,

    /// Single-image debug-mode intermediate steps (reused from old architecture).
    /// Only populated when `jobs.len() == 1` and `show_pipeline` is true.
    intermediates: Vec<Intermediate>,

    /// Cached `has_directory_picker` result.
    can_pick_directory: bool,

    /// True while Phase 1 (sequential image decode) is running.
    /// Used to show a banner before the main thread blocks.
    decoding: bool,

    /// Current decode progress (current, total) for the overlay.
    decode_progress: (usize, usize),
}

impl App {
    fn new() -> Self {
        Self {
            upload: Upload::new(),
            now: Instant::now(),
            viewer: Viewer::new(),
            jobs: Vec::new(),
            selected_job: None,
            show_pipeline: false,
            intermediates: Vec::new(),
            can_pick_directory: js_interop::has_directory_picker(),
            decoding: false,
            decode_progress: (0, 0),
        }
    }

    /// Start the step-by-step debug pipeline for a specific job.
    ///
    /// Decodes the job's file to an `Intermediate` and begins the pipeline,
    /// populating `self.intermediates` so the middle column can show preview cards.
    fn start_debug_pipeline_for(&mut self, job_id: usize) -> Task<Message> {
        // Get the file data for this job
        let entry = if let State::Finished(entries) = &self.upload.state {
            entries.get(job_id).cloned()
        } else {
            None
        };

        let Some(entry) = entry else {
            return Task::none();
        };

        // Decode file to intermediate
        match decode_to_intermediate(&entry) {
            Ok(inter) => {
                self.intermediates.clear();
                self.intermediates.push(inter.clone());

                // Start the step-by-step pipeline
                Task::sip(
                    inter.clone().process(),
                    Message::BlurhashDecoded.with(inter),
                    Message::Process,
                )
            }
            Err(e) => {
                log::error!("Failed to decode for preview: {e:?}");
                self.intermediates.clear();
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let is_animating = self
            .intermediates
            .iter()
            .any(|i| i.preview.is_animating(self.now))
            || self.viewer.is_animating(self.now)
            || self.upload.is_animating(self.now);

        if is_animating {
            window::frames().map(|_| Message::Animate)
        } else {
            Subscription::none()
        }
    }

    fn update(&mut self, message: Message, now: Instant) -> Task<Message> {
        self.now = now;

        match message {
            // ── File picking ──
            Message::PickFiles => {
                let task = self.upload.start_files();
                task.map(Message::UploadUpdated)
            }
            Message::PickDirectory => {
                let task = self.upload.start_directory();
                task.map(Message::UploadUpdated)
            }
            Message::UploadUpdated(update) => {
                self.upload.update(update);

                // When upload finishes, create jobs
                if let State::Finished(entries) = &self.upload.state {
                    self.jobs = entries
                        .iter()
                        .enumerate()
                        .map(|(id, entry)| Job {
                            id,
                            filename: entry.name.clone(),
                            status: JobStatus::Queued,
                            intermediates: Vec::new(),
                            metrics: None,
                        })
                        .collect();

                    // Auto-set mode: single → debug (pipeline ON), multi → fast (pipeline OFF)
                    self.show_pipeline = self.jobs.len() == 1;
                    self.selected_job = if self.jobs.len() == 1 {
                        Some(0)
                    } else {
                        None
                    };
                    self.intermediates.clear();

                    // For single-image debug mode, auto-decode to intermediate
                    // Keep status as Queued — user must click "Process" to start
                    if self.jobs.len() == 1 {
                        if let Ok(inter) = decode_to_intermediate(&entries[0]) {
                            self.intermediates.push(inter.clone());
                        }
                    }
                }

                Task::none()
            }

            // ── Single-image debug processing (step-by-step) ──
            Message::Process(Ok(inter)) => {
                if let Some(last) = self.intermediates.last_mut() {
                    *last = inter.clone();
                }

                // Update job metrics if final step
                if inter.current_step == Step::FruitletCounting {
                    if let Some(job) = self.jobs.first_mut() {
                        job.metrics = inter.metrics.clone();
                        job.status = JobStatus::Done;
                    }
                    return Task::none();
                }

                Task::sip(
                    inter.clone().process(),
                    Message::BlurhashDecoded.with(inter),
                    Message::Process,
                )
            }
            Message::BlurhashDecoded(mut inter, blurhash) => {
                inter.preview = Preview::loading(blurhash, self.now);
                self.intermediates.push(inter);
                Task::none()
            }
            Message::Process(Err(e)) => {
                log::error!("Processing failed: {e:?}");
                if let Some(job) = self.jobs.first_mut() {
                    job.status = JobStatus::Error(format!("{e}"));
                }
                Task::none()
            }

            // ── Batch processing ──
            Message::BatchStart => {
                // TODO Phase 2: rayon parallel processing
                // For now, process images sequentially
                if let State::Finished(entries) = &self.upload.state {
                    let entries_clone: Vec<FileEntry> = entries.clone();
                    let num_jobs = self.jobs.len();

                    // For single image in debug mode, start pipeline step-by-step
                    if num_jobs == 1 && self.show_pipeline {
                        if let Some(inter) = self.intermediates.last().cloned() {
                            self.jobs[0].status = JobStatus::Processing;
                            return Task::sip(
                                inter.clone().process(),
                                Message::BlurhashDecoded.with(inter),
                                Message::Process,
                            );
                        }
                    }

                    // Mark all jobs as Processing and show decoding overlay.
                    // Return Task::done(StartDecoding) so iced renders the
                    // overlay BEFORE the synchronous Phase 1 decode blocks.
                    for job in &mut self.jobs {
                        job.status = JobStatus::Processing;
                    }
                    self.decoding = true;
                    return Task::done(Message::StartDecoding);
                }
                Task::none()
            }
            Message::StartDecoding => {
                // Now the overlay is visible. Start the streaming pipeline.
                if let State::Finished(entries) = &self.upload.state {
                    let entries_clone: Vec<FileEntry> = entries.clone();
                    let num_jobs = self.jobs.len();
                    return Task::run(iced::stream::channel(1, move |mut output: futures::channel::mpsc::Sender<Message>| async move {
                        use futures::SinkExt;
                        let total = entries_clone.len();

                        // Phase 1: sequential decode on main thread
                        // After sending progress, yield to browser's MACROTASK queue
                        // via setTimeout(0). Microtasks alone don't trigger paint.
                        let mut prepared = Vec::with_capacity(total);
                        for (id, entry) in entries_clone.iter().enumerate() {
                            let _ = output.send(Message::DecodingProgress(id, total)).await;
                            // Force browser paint before blocking on the next decode
                            gloo_timers::future::TimeoutFuture::new(0).await;
                            match pipeline::fast::prepare_image(entry) {
                                Ok(prep) => prepared.push((id, prep)),
                                Err(e) => {
                                    let _ = output.send(Message::JobDone(id, Err(e))).await;
                                }
                            }
                        }

                        // Phase 1 done — dismiss overlay immediately
                        let _ = output.send(Message::DecodingDone).await;

                        // Phase 2: spawn rayon tasks, collect results via channel
                        let (tx, mut rx) = futures::channel::mpsc::unbounded();
                        let count = prepared.len();

                        for (id, prep) in prepared {
                            let tx = tx.clone();
                            rayon::spawn(move || {
                                let result = pipeline::fast::process_prepared(&prep);
                                let _ = tx.unbounded_send(Message::JobDone(id, result));
                            });
                        }
                        drop(tx); // close sender so rx ends when all workers finish

                        // Forward results to iced as each worker completes
                        use futures::StreamExt;
                        let mut received = 0;
                        while let Some(msg) = rx.next().await {
                            let _ = output.send(msg).await;
                            received += 1;
                            if received >= count {
                                break;
                            }
                        }
                    }), |msg| msg);
                }
                Task::none()
            }
            Message::DecodingDone => {
                self.decoding = false;
                self.decode_progress = (0, 0);
                Task::none()
            }
            Message::DecodingProgress(current, total) => {
                self.decode_progress = (current, total);
                Task::none()
            }
            Message::JobDone(id, result) => {
                if let Some(job) = self.jobs.get_mut(id) {
                    match result {
                        Ok(metrics) => {
                            job.metrics = Some(metrics);
                            job.status = JobStatus::Done;
                        }
                        Err(e) => {
                            job.status = JobStatus::Error(format!("{e}"));
                        }
                    }
                }
                Task::none()
            }
            Message::Noop => Task::none(),

            // ── UI interaction ──
            Message::SelectJob(id) => {
                self.selected_job = Some(id);
                // If pipeline preview is ON, start debug pipeline for this job
                if self.show_pipeline {
                    return self.start_debug_pipeline_for(id);
                }
                Task::none()
            }
            Message::TogglePipeline(on) => {
                self.show_pipeline = on;
                if on {
                    // If a job is already selected, start debug pipeline for it
                    if let Some(id) = self.selected_job {
                        return self.start_debug_pipeline_for(id);
                    }
                } else {
                    // Clear intermediates when toggling OFF
                    self.intermediates.clear();
                }
                Task::none()
            }
            Message::ThumbnailHovered(step, is_hovered) => {
                if let Some(i) = self
                    .intermediates
                    .iter_mut()
                    .find(|i| i.current_step == step)
                {
                    i.preview.toggle_zoom(is_hovered, self.now);
                }
                Task::none()
            }
            Message::Open(step) => {
                if let Some(intermediate) = self
                    .intermediates
                    .iter()
                    .find(|i| i.current_step == step)
                    .cloned()
                {
                    self.viewer.show(
                        dynamic_image_to_handle(&intermediate.preview.into()),
                        self.now,
                    );
                }
                Task::none()
            }
            Message::Close => {
                self.viewer.close(self.now);
                Task::none()
            }
            Message::Animate => Task::none(),
            Message::ExportCsv => {
                let csv = jobs_to_csv(&self.jobs);
                trigger_download(&csv, "pineapple_results.csv");
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // ── Left column: file input + file list ──
        let mut left_col = column![].spacing(10).width(Length::FillPortion(2));

        // Buttons
        let mut btn_row = row![button("Choose Files").on_press(Message::PickFiles)].spacing(8);
        if self.can_pick_directory {
            btn_row = btn_row.push(button("Choose Folder").on_press(Message::PickDirectory));
        }
        left_col = left_col.push(btn_row);

        // Upload progress
        left_col = left_col.push(self.upload.view());

        // "Start" button
        if !self.jobs.is_empty()
            && self.jobs.iter().all(|j| j.status == JobStatus::Queued)
        {
            let label = if self.jobs.len() == 1 {
                "Process"
            } else {
                "Batch Process"
            };
            left_col = left_col.push(button(label).on_press(Message::BatchStart));
        }

        // File list
        if self.jobs.len() == 1 {
            // Single file: show preview if available
            if let Some(inter) = self.intermediates.first() {
                left_col = left_col.push(inter.card(self.now));
            }
            if let Some(job) = self.jobs.first() {
                left_col = left_col.push(text(&job.filename));
            }
        } else {
            // Multiple files: name + status
            let file_list = column(self.jobs.iter().map(|job| {
                let icon = match &job.status {
                    JobStatus::Queued => "\u{23f3}",
                    JobStatus::Processing => "\u{1f504}",
                    JobStatus::Done => "\u{2705}",
                    JobStatus::Error(_) => "\u{274c}",
                };
                let is_selected = self.selected_job == Some(job.id);
                let row_content: Element<'_, Message> = row![
                    text(icon),
                    text(&job.filename).width(Length::Fill),
                ]
                .spacing(8)
                .into();

                if is_selected {
                    button(container(row_content).style(container::dark))
                        .padding(4)
                        .style(button::text)
                        .on_press(Message::SelectJob(job.id))
                        .into()
                } else {
                    button(row_content)
                        .padding(4)
                        .style(button::text)
                        .on_press(Message::SelectJob(job.id))
                        .into()
                }
            }))
            .spacing(2);

            left_col = left_col.push(scrollable(file_list));
        }

        // ── Middle column: pipeline preview ──
        let mid_col: Element<'_, Message> = if self.show_pipeline {
            let mut col = column![
                toggler(self.show_pipeline)
                    .label("Pipeline Preview")
                    .on_toggle(Message::TogglePipeline),
            ]
            .spacing(10)
            .width(Length::FillPortion(3));

            if self.selected_job.is_none() {
                col = col.push(
                    container(text("← Select a file to view details"))
                        .center(Length::Fill)
                        .padding(40),
                );
            } else if !self.intermediates.is_empty() {
                // Show step-by-step pipeline cards (skip Original — shown in left column)
                let cards: Vec<_> = self
                    .intermediates
                    .iter()
                    .filter(|i| i.current_step != Step::Original)
                    .map(|i| i.card(self.now))
                    .collect();
                col = col.push(
                    scrollable(
                        grid(cards)
                            .columns(2)
                            .spacing(5),
                    ),
                );
            } else if let Some(job) = self.selected_job.and_then(|id| self.jobs.get(id)) {
                // Batch mode: show selected job status summary
                let status_text = match &job.status {
                    JobStatus::Queued => "Queued".to_string(),
                    JobStatus::Processing => "Processing...".to_string(),
                    JobStatus::Done => "Done".to_string(),
                    JobStatus::Error(e) => format!("Error: {e}"),
                };
                let mut info = column![
                    text(&job.filename).size(18),
                    text(status_text).size(14),
                ]
                .spacing(8)
                .padding(20);

                if let Some(m) = &job.metrics {
                    info = info.push(text(format!("Height: {:.2} mm", m.major_length)).size(14));
                    info = info.push(text(format!("Width: {:.2} mm", m.minor_length)).size(14));
                    info = info.push(text(format!("Volume: {:.0} mm3", m.volume)).size(14));
                    if let Some(v) = m.a_eq {
                        info = info.push(text(format!("a_eq: {v:.2} mm")).size(14));
                    }
                    if let Some(v) = m.b_eq {
                        info = info.push(text(format!("b_eq: {v:.2} mm")).size(14));
                    }
                    if let Some(v) = m.surface_area {
                        info = info.push(text(format!("Surface area: {v:.0} mm2")).size(14));
                    }
                    if let Some(v) = m.n_total {
                        info = info.push(text(format!("N_total: {v}")).size(14));
                    }
                }

                col = col.push(container(info));
            }

            col.into()
        } else if !self.jobs.is_empty() {
            column![
                toggler(self.show_pipeline)
                    .label("Pipeline Preview")
                    .on_toggle(Message::TogglePipeline),
            ]
            .width(Length::Shrink)
            .into()
        } else {
            space::horizontal().into()
        };

        // ── Right column: results table ──
        let mut right_col = column![text("Results").size(24)].spacing(8).width(Length::FillPortion(5));

        // Table header
        let header = row![
            text("File").width(Length::FillPortion(3)),
            text("Height").width(Length::FillPortion(2)),
            text("Width").width(Length::FillPortion(2)),
            text("Volume").width(Length::FillPortion(2)),
            text("a_eq").width(Length::FillPortion(2)),
            text("b_eq").width(Length::FillPortion(2)),
            text("S. Area").width(Length::FillPortion(2)),
            text("N_total").width(Length::FillPortion(2)),
        ]
        .spacing(4);
        right_col = right_col.push(header);

        // Table rows
        let completed_jobs: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Done)
            .collect();

        if completed_jobs.is_empty() {
            right_col = right_col.push(
                container(text("No results yet"))
                    .center(Length::Fill)
                    .padding(20),
            );
        } else {
            let rows = column(completed_jobs.iter().map(|job| {
                let m = job.metrics.as_ref().unwrap();
                row![
                    text(&job.filename).size(12).width(Length::FillPortion(3)),
                    text(format!("{:.1}", m.major_length)).size(12).width(Length::FillPortion(2)),
                    text(format!("{:.1}", m.minor_length)).size(12).width(Length::FillPortion(2)),
                    text(format!("{:.0}", m.volume)).size(12).width(Length::FillPortion(2)),
                    text(m.a_eq.map_or("-".into(), |v| format!("{v:.1}"))).size(12).width(Length::FillPortion(2)),
                    text(m.b_eq.map_or("-".into(), |v| format!("{v:.1}"))).size(12).width(Length::FillPortion(2)),
                    text(m.surface_area.map_or("-".into(), |v| format!("{v:.0}"))).size(12).width(Length::FillPortion(2)),
                    text(m.n_total.map_or("-".into(), |v| format!("{v}"))).size(12).width(Length::FillPortion(2)),
                ]
                .spacing(4)
                .into()
            }))
            .spacing(2);
            right_col = right_col.push(scrollable(rows));
        }

        // Export button
        if !completed_jobs.is_empty() {
            right_col = right_col.push(button("Export CSV").on_press(Message::ExportCsv));
        }

        // ── Assemble layout ──
        let content = container(
            row![
                scrollable(left_col),
                mid_col,
                scrollable(right_col),
            ]
            .spacing(10)
            .padding(10),
        );

        let viewer = self.viewer.view(self.now);
        let mut layers = stack![content, viewer];

        // Full-screen decoding overlay — blocks interaction visually
        if self.decoding {
            let (current, total) = self.decode_progress;
            let progress_text = if total > 0 {
                format!("⏳ Decoding images… ({}/{})", current + 1, total)
            } else {
                "⏳ Decoding images…".to_string()
            };
            let overlay = container(
                text(progress_text).size(24)
            )
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6))),
                text_color: Some(iced::Color::WHITE),
                ..Default::default()
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill);
            layers = layers.push(overlay);
        }

        layers.into()
    }
}

/// Fast-mode pipeline: processes a file from bytes to metrics without generating
/// any intermediate images or previews.
///
/// TODO Phase 2: This will be called from rayon worker threads.
// run_pipeline_fast has been moved to pipeline/fast.rs
// for Web Worker thread safety (no iced types / browser APIs).

fn main() -> iced::Result {
    console_log::init().expect("Initialize logger");
    console_error_panic_hook::set_once();

    iced::application::timed(App::new, App::update, App::subscription, App::view)
        .centered()
        .font(NOTO_EMOJI_BYTES)
        .font(NOTO_SANS_SC_BYTES)
        .run()
}
