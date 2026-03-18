mod correction;
mod error;
mod export;
mod history;
mod icons;
mod job;
mod js_interop;
mod pipeline;
mod theme;
mod ui;
mod upload;
mod utils;

// Re-export init_thread_pool so wasm-bindgen exposes `initThreadPool()` in JS.
pub use wasm_bindgen_rayon::init_thread_pool;

use std::collections::HashSet;

use crate::{
    error::Error,
    export::{jobs_to_csv, trigger_download},
    history::{
        model::{AnalysisRecord, SessionMeta, SessionSummary, StoredMetrics},
        store::{self, CacheWarningLevel},
    },
    job::{Job, JobStatus},
    js_interop::FileEntry,
    pipeline::{EncodedImage, FruitletMetrics, Intermediate, Step},
    ui::{
        history_view::{self, HistoryPanel, Page},
        preview::Preview,
        viewer::Viewer,
    },
    upload::{State, Update, Upload, decode_to_intermediate},
    utils::dynamic_image_to_handle,
};

use iced::{
    Color, Element, Function, Length, Subscription, Task,
    time::Instant,
    widget::{
        button, column, container, grid, image, row, scrollable, space, stack, text,
        toggler, tooltip,
    },
    window,
};

/// Noto Sans SC font bytes — subset for CJK display. See `assets/README.md` for regeneration.
const NOTO_SANS_SC_BYTES: &[u8] = include_bytes!("../assets/NotoSansSC-Regular.ttf");

// ────────────────────────  Messages  ────────────────────────

#[non_exhaustive]
#[derive(Debug, Clone)]
#[allow(dead_code)]
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

    // ── Navigation ──
    NavigateTo(Page),
    OpenGitHub,

    // ── History page ──
    HistoryLoaded(Vec<SessionSummary>),
    HistorySetPanel(HistoryPanel),
    ToggleSidebar,
    ToggleSessionSelected(String, bool),
    ToggleAllSessions(bool),
    ToggleSessionStar(String, bool),
    LoadSelectedRecords,
    SessionRecordsLoaded(Vec<AnalysisRecord>),
    ToggleSuspect(String, bool),
    OpenNoteEditor(String),
    NoteInputChanged(String, String),
    SaveNote(String, String),
    SubmitCurrentNote,
    DeleteCurrentNote,
    OpenMetricEditor(String),
    /// Metric editor: user changed a field's raw text.
    /// (field_index, raw_text) — field 0=H, 1=D, 2=a, 3=b
    MetricInputChanged(usize, String),
    SaveEditedMetric(String, StoredMetrics),
    SubmitCurrentMetric,
    ResetCurrentMetric,
    CancelEdit,
    StartRenameSession(String),
    RenameSessionInput(String),
    SubmitSessionRename,
    DeleteSelectedSessions,
    ConfirmDelete,
    CancelDelete,
    SearchQueryChanged(String),
    ClearAllHistory,
    ConfirmClearAll,
    CancelClearAll,
    DeleteExportedSessions,
    DismissExportPrompt,
    PaneResized(iced::widget::pane_grid::ResizeEvent),
    SortBy(SortColumn),
    ExportSelectedSessions,
    QuickCleanup,
    CleanupDone(store::CleanupResult),
    CacheStatus(CacheWarningLevel),
    DismissCacheWarning,
    BatchSaved,
    /// Undo a recent deletion (soft-deleted data).
    UndoDelete,
    /// Undo timer expired — commit the deletion.
    UndoExpired,
    UndoToastMessage(String),
    /// Undo countdown tick (every 1s).
    UndoTick,

    /// Jump to a specific record from the chart click.
    JumpToRecord(String),
    /// Tick for highlight flash animation.
    HighlightTick,

    /// Toggle the D2R-style help overlay (F1 / Help button).
    ToggleHelp,
    /// No-op (used for smoke tests)
    Noop,
    /// Quick filter toggle for Records panel
    ToggleRecordFilter(RecordFilter),
}

// ────────────────────────  History Pane  ────────────────────────

#[derive(Clone, Debug)]
enum HistoryPane {
    Sidebar,
    MainPanel,
}

/// Data cached during soft-delete, restorable via Undo.
struct PendingDelete {
    sessions: Vec<SessionSummary>,
    records: Vec<AnalysisRecord>,
    sids: Vec<String>,
}

/// Quick filter for Records panel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum RecordFilter {
    #[default]
    All,
    SuspectsOnly,
    NormalOnly,
    HasNote,
}

/// Column by which records can be sorted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SortColumn {
    Filename,
    Height,
    Width,
    Volume,
    Aeq,
    Beq,
    SurfaceArea,
    NTotal,
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
    /// Whether the Pipeline Details column is visible.
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

    // ── History / Navigation state ──
    /// Current page (Analysis or History).
    page: Page,

    /// Session summaries loaded from IndexedDB.
    sessions: Vec<SessionSummary>,
    /// Sessions selected via checkbox.
    selected_sessions: HashSet<String>,
    /// Records loaded for selected sessions.
    current_records: Vec<AnalysisRecord>,

    /// Current note editing state: (record_id, current_text).
    editing_note: Option<(String, String)>,
    /// Current metric editing state: (record_id, current_metrics).
    editing_metric: Option<(String, StoredMetrics)>,
    /// Raw text buffers for the 4 editable metric fields [H, D, a, b].
    editing_metric_text: [String; 4],

    /// Quick filter for Records panel.
    record_filter: RecordFilter,

    /// Current session rename editing state: (session_id, current_name).
    editing_session_name: Option<(String, String)>,

    /// Current cache warning level.
    cache_warning: Option<CacheWarningLevel>,

    /// Undo toast message text.
    undo_toast: Option<String>,

    /// Countdown seconds remaining for undo (None = no active undo).
    undo_countdown: Option<u8>,

    /// Soft-deleted data awaiting commit or undo.
    pending_delete: Option<PendingDelete>,

    /// Delete confirmation state: session IDs pending deletion.
    delete_confirm: Option<(Vec<String>, u32)>,

    /// Search query for filtering records by filename.
    search_query: String,

    /// Current sort column and direction for records table.
    sort_column: Option<SortColumn>,
    sort_ascending: bool,

    /// Clear-all confirmation state.
    clear_all_confirm: bool,

    /// Export-then-delete prompt.
    export_delete_prompt: bool,
    /// Session IDs that were last exported.
    exported_session_ids: Vec<String>,

    /// Outlier cells: record_id → set of outlier columns.
    outlier_cells: std::collections::HashMap<String, std::collections::HashSet<history::stats::MetricColumn>>,
    /// Descriptive statistics per column.
    column_stats: std::collections::HashMap<history::stats::MetricColumn, history::stats::ColumnStats>,

    /// Precomputed parallel coordinates chart data.
    parallel_coords_chart: ui::parallel_coords::ParallelCoordsChart,

    /// Record ID currently being flash-highlighted (after JumpToRecord).
    highlight_record_id: Option<String>,
    /// Remaining flash ticks (counts down to 0; 6 ticks = 3 blinks × on/off).
    highlight_ticks: u8,

    /// Session ID generated for the current batch (None for single-image mode).
    current_session_id: Option<String>,

    /// Pane grid state for the history sidebar/main panel split.
    history_panes: iced::widget::pane_grid::State<HistoryPane>,

    /// D2R-style help overlay visible.
    show_help: bool,
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
            page: Page::default(),
            sessions: Vec::new(),
            selected_sessions: HashSet::new(),
            current_records: Vec::new(),
            editing_note: None,
            editing_metric: None,
            editing_metric_text: Default::default(),
            record_filter: RecordFilter::default(),
            editing_session_name: None,
            cache_warning: None,
            undo_toast: None,
            undo_countdown: None,
            pending_delete: None,
            delete_confirm: None,
            search_query: String::new(),
            sort_column: None,
            sort_ascending: true,
            clear_all_confirm: false,
            export_delete_prompt: false,
            exported_session_ids: Vec::new(),
            outlier_cells: std::collections::HashMap::new(),
            column_stats: std::collections::HashMap::new(),
            parallel_coords_chart: ui::parallel_coords::ParallelCoordsChart::default(),
            highlight_record_id: None,
            highlight_ticks: 0,
            current_session_id: None,
            history_panes: iced::widget::pane_grid::State::with_configuration(
                iced::widget::pane_grid::Configuration::Split {
                    axis: iced::widget::pane_grid::Axis::Vertical,
                    ratio: 0.18,
                    a: Box::new(iced::widget::pane_grid::Configuration::Pane(HistoryPane::Sidebar)),
                    b: Box::new(iced::widget::pane_grid::Configuration::Pane(HistoryPane::MainPanel)),
                },
            ),
            show_help: false,
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

        let mut subs = Vec::new();

        if is_animating {
            subs.push(window::frames().map(|_| Message::Animate));
        }

        // Undo countdown tick (1 per second)
        if self.undo_countdown.is_some() {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(1))
                    .map(|_| Message::UndoTick),
            );
        }

        // Highlight flash animation (300ms per tick, 6 ticks = 1.8s total)
        if self.highlight_ticks > 0 {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(300))
                    .map(|_| Message::HighlightTick),
            );
        }

        // ESC key to cancel editing (note / metric / session rename editors)
        if self.editing_note.is_some() || self.editing_metric.is_some() || self.editing_session_name.is_some() {
            subs.push(
                iced::event::listen_with(|event, _status, _window| {
                    use iced::keyboard;
                    if let iced::Event::Keyboard(keyboard::Event::KeyPressed {
                        key: keyboard::Key::Named(keyboard::key::Named::Escape),
                        ..
                    }) = event
                    {
                        Some(Message::CancelEdit)
                    } else {
                        None
                    }
                }),
            );
        }

        // F1 toggles help overlay (non-capturing closure required by listen_with)
        subs.push(
            iced::event::listen_with(|event, _status, _window| {
                use iced::keyboard;
                if let iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::F1),
                    ..
                }) = event
                {
                    Some(Message::ToggleHelp)
                } else {
                    None
                }
            }),
        );

        // ESC closes help overlay when open
        if self.show_help {
            subs.push(
                iced::event::listen_with(|event, _status, _window| {
                    use iced::keyboard;
                    if let iced::Event::Keyboard(keyboard::Event::KeyPressed {
                        key: keyboard::Key::Named(keyboard::key::Named::Escape),
                        ..
                    }) = event
                    {
                        Some(Message::ToggleHelp)
                    } else {
                        None
                    }
                }),
            );
        }

        Subscription::batch(subs)
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

                // Update job metrics if final step — BUT only when doing real
                // analysis (single-file mode or batch), NOT when doing pipeline
                // detail preview in batch mode.
                if inter.current_step == Step::FruitletCounting {
                    let is_preview_only = self.show_pipeline && self.jobs.len() > 1;
                    if !is_preview_only {
                        if let Some(job) = self.jobs.first_mut() {
                            log::info!(
                                "[main] Setting job.metrics: a_eq={:?}, b_eq={:?}, surface={:?}, n_total={:?}",
                                inter.metrics.as_ref().and_then(|m| m.a_eq),
                                inter.metrics.as_ref().and_then(|m| m.b_eq),
                                inter.metrics.as_ref().and_then(|m| m.surface_area),
                                inter.metrics.as_ref().and_then(|m| m.n_total),
                            );
                            job.metrics = inter.metrics.clone();
                            job.status = JobStatus::Done;
                        }
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
                if let State::Finished(_entries) = &self.upload.state {
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

                    // Generate session ID for batch mode (≥2 files)
                    if num_jobs >= 2 {
                        self.current_session_id = Some(store::generate_id());
                    } else {
                        self.current_session_id = None;
                    }

                    // Mark all jobs as Processing and show decoding overlay.
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
                    let _num_jobs = self.jobs.len();
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

                // Check if all batch jobs are done — auto-save to history
                let all_done = self.jobs.iter().all(|j| {
                    matches!(j.status, JobStatus::Done | JobStatus::Error(_))
                });
                if all_done {
                    if let Some(session_id) = self.current_session_id.take() {
                        let total_count = self.jobs.len() as u32;
                        let failed_count = self.jobs.iter()
                            .filter(|j| matches!(j.status, JobStatus::Error(_)))
                            .count() as u32;

                        let timestamp = js_sys::Date::now();
                        let meta = SessionMeta {
                            session_id: session_id.clone(),
                            timestamp,
                            total_count,
                            success_count: total_count - failed_count,
                            failed_count,
                            starred: false,
                            name: None,
                        };

                        let mut records: Vec<AnalysisRecord> = self.jobs.iter()
                            .filter_map(|job| {
                                let metrics = job.metrics.as_ref()?;
                                Some(AnalysisRecord {
                                    id: store::generate_id(),
                                    session_id: session_id.clone(),
                                    timestamp,
                                    filename: job.filename.clone(),
                                    metrics: history::model::StoredMetrics::from(metrics),
                                    suspect: false,
                                    note: String::new(),
                                })
                            })
                            .collect();

                        // Compute IQR outliers and mark suspects before persisting
                        {
                            let refs: Vec<&AnalysisRecord> = records.iter().collect();
                            let stats = history::stats::compute_all_stats_from_refs(&refs);
                            let outliers = history::stats::detect_outliers_from_refs(&refs, &stats);
                            for record in &mut records {
                                if outliers.contains_key(&record.id) {
                                    record.suspect = true;
                                }
                            }
                        }

                        return Task::perform(
                            async move {
                                match store::open_db().await {
                                    Ok(db) => match store::save_session(&db, &meta, &records).await {
                                        Ok(()) => log::info!("Batch saved successfully"),
                                        Err(e) => log::error!("Failed to save batch: {e:?}"),
                                    },
                                    Err(e) => log::error!("Failed to open DB for batch save: {e:?}"),
                                }
                            },
                            |()| Message::BatchSaved,
                        );
                    }
                }

                Task::none()
            }
            Message::JumpToRecord(record_id) => {
                // Switch to Records panel + start flash animation
                if let Page::History { panel, .. } = &mut self.page {
                    *panel = HistoryPanel::Records;
                }
                self.highlight_record_id = Some(record_id);
                self.highlight_ticks = 6; // 3 blinks × (on + off)
                Task::none()
            }
            Message::HighlightTick => {
                if self.highlight_ticks > 0 {
                    self.highlight_ticks -= 1;
                }
                if self.highlight_ticks == 0 {
                    self.highlight_record_id = None;
                }
                Task::none()
            }
            Message::ToggleHelp => {
                self.show_help = !self.show_help;
                Task::none()
            }
            Message::Noop => Task::none(),
            Message::ToggleRecordFilter(f) => {
                // Toggle off if the same filter is pressed again
                self.record_filter = if self.record_filter == f { RecordFilter::All } else { f };
                Task::none()
            }

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

            // ── Navigation ──
            Message::NavigateTo(page) => {
                self.editing_note = None;
                self.editing_metric = None;
                if matches!(page, Page::History { .. }) {
                    // Load sessions when navigating to History
                    self.page = page;
                    return Task::perform(
                        async {
                            let db = store::open_db().await.ok()?;
                            store::load_session_summaries(&db).await.ok()
                        },
                        |opt| Message::HistoryLoaded(opt.unwrap_or_default()),
                    );
                }
                self.page = page;
                Task::none()
            }
            Message::OpenGitHub => {
                // Open GitHub repository in new tab
                if let Some(window) = web_sys::window() {
                    let _ = window.open_with_url_and_target(
                        "https://github.com/TT-Industry/pineapplehub",
                        "_blank",
                    );
                }
                Task::none()
            }

            // ── History page ──
            Message::HistoryLoaded(summaries) => {
                self.sessions = summaries;
                // Also load cache status
                Task::perform(
                    async {
                        let db = store::open_db().await.ok()?;
                        store::check_cache_status(&db).await.ok()
                    },
                    |opt| Message::CacheStatus(opt.unwrap_or(CacheWarningLevel::Ok)),
                )
            }
            Message::HistorySetPanel(panel) => {
                if let Page::History {
                    panel: current_panel,
                    ..
                } = &mut self.page
                {
                    *current_panel = panel;
                }
                Task::none()
            }
            Message::ToggleSidebar => {
                if let Page::History { sidebar_open, .. } = &mut self.page {
                    *sidebar_open = !*sidebar_open;
                }
                Task::none()
            }
            Message::ToggleSessionSelected(sid, checked) => {
                log::info!("ToggleSessionSelected: sid={sid}, checked={checked}");
                if checked {
                    self.selected_sessions.insert(sid);
                } else {
                    self.selected_sessions.remove(&sid);
                }
                // Auto-load records for selected sessions
                let sids: Vec<String> = self.selected_sessions.iter().cloned().collect();
                log::info!("Loading records for {} session(s)", sids.len());
                Task::perform(
                    async move {
                        let db = match store::open_db().await {
                            Ok(db) => db,
                            Err(e) => {
                                log::error!("Failed to open DB: {e:?}");
                                return Vec::new();
                            }
                        };
                        match store::load_records_for_sessions(&db, &sids).await {
                            Ok(records) => {
                                log::info!("Loaded {} records", records.len());
                                records
                            }
                            Err(e) => {
                                log::error!("Failed to load records: {e:?}");
                                Vec::new()
                            }
                        }
                    },
                    Message::SessionRecordsLoaded,
                )
            }
            Message::ToggleAllSessions(checked) => {
                if checked {
                    self.selected_sessions = self
                        .sessions
                        .iter()
                        .map(|s| s.session_id.clone())
                        .collect();
                } else {
                    self.selected_sessions.clear();
                }
                let sids: Vec<String> = self.selected_sessions.iter().cloned().collect();
                Task::perform(
                    async move {
                        let db = match store::open_db().await {
                            Ok(db) => db,
                            Err(e) => {
                                log::error!("Failed to open DB: {e:?}");
                                return Vec::new();
                            }
                        };
                        store::load_records_for_sessions(&db, &sids).await.unwrap_or_default()
                    },
                    Message::SessionRecordsLoaded,
                )
            }
            Message::ToggleSessionStar(sid, starred) => {
                // Update in-memory
                if let Some(session) = self.sessions.iter_mut().find(|s| s.session_id == sid) {
                    session.starred = starred;
                }
                // Persist to IndexedDB
                Task::perform(
                    async move {
                        if let Ok(db) = store::open_db().await {
                            let _ = store::toggle_session_star(&db, &sid, starred).await;
                        }
                    },
                    |()| Message::Noop,
                )
            }
            Message::LoadSelectedRecords => {
                let sids: Vec<String> = self.selected_sessions.iter().cloned().collect();
                Task::perform(
                    async move {
                        let db = match store::open_db().await {
                            Ok(db) => db,
                            Err(e) => {
                                log::error!("Failed to open DB: {e:?}");
                                return Vec::new();
                            }
                        };
                        store::load_records_for_sessions(&db, &sids).await.unwrap_or_default()
                    },
                    Message::SessionRecordsLoaded,
                )
            }
            Message::SessionRecordsLoaded(records) => {
                log::info!("SessionRecordsLoaded: {} records", records.len());
                self.current_records = records;

                // Per-session IQR outlier detection (for display highlighting only).
                // The suspect flag is already persisted in IndexedDB from batch save time;
                // we only recompute outlier_cells here for cell-level highlighting.
                self.outlier_cells.clear();
                self.column_stats.clear();

                // Group record indices by session
                let mut session_groups: std::collections::HashMap<String, Vec<usize>> =
                    std::collections::HashMap::new();
                for (i, r) in self.current_records.iter().enumerate() {
                    session_groups
                        .entry(r.session_id.clone())
                        .or_default()
                        .push(i);
                }

                for (_sid, indices) in &session_groups {
                    let session_records: Vec<&AnalysisRecord> =
                        indices.iter().map(|&i| &self.current_records[i]).collect();
                    let stats = history::stats::compute_all_stats_from_refs(&session_records);
                    let outliers = history::stats::detect_outliers_from_refs(&session_records, &stats);
                    self.outlier_cells.extend(outliers);
                    self.column_stats.extend(stats);
                }

                // Recalculate parallel coords chart
                self.parallel_coords_chart = ui::parallel_coords::ParallelCoordsChart::new(
                    &self.current_records, &self.outlier_cells,
                );

                Task::none()
            }
            Message::ToggleSuspect(record_id, suspect) => {
                if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                    record.suspect = suspect;
                    let record = record.clone();

                    // Sync suspect_count in session summaries
                    let session_id = record.session_id.clone();
                    if let Some(session) = self.sessions.iter_mut().find(|s| s.session_id == session_id) {
                        session.suspect_count = self
                            .current_records
                            .iter()
                            .filter(|r| r.session_id == session_id && r.suspect)
                            .count() as u32;
                    }

                    // Recalculate chart after suspect change
                    self.parallel_coords_chart = ui::parallel_coords::ParallelCoordsChart::new(
                        &self.current_records, &self.outlier_cells,
                    );

                    return Task::perform(
                        async move {
                            if let Ok(db) = store::open_db().await {
                                match store::update_record(&db, &record).await {
                                    Ok(()) => log::info!(
                                        "ToggleSuspect: persisted record {} suspect={}",
                                        record.id, record.suspect
                                    ),
                                    Err(e) => log::error!(
                                        "ToggleSuspect: FAILED to persist record {}: {:?}",
                                        record.id, e
                                    ),
                                }
                            } else {
                                log::error!("ToggleSuspect: failed to open DB");
                            }
                        },
                        |()| Message::Noop,
                    );
                }
                Task::none()
            }
            Message::OpenNoteEditor(record_id) => {
                // Toggle: close if already editing the same record
                if let Some((ref current_id, _)) = self.editing_note {
                    if *current_id == record_id {
                        self.editing_note = None;
                        return Task::none();
                    }
                }
                let note = self
                    .current_records
                    .iter()
                    .find(|r| r.id == record_id)
                    .map(|r| r.note.clone())
                    .unwrap_or_default();
                self.editing_note = Some((record_id, note));
                self.editing_metric = None;
                Task::none()
            }
            Message::NoteInputChanged(record_id, val) => {
                self.editing_note = Some((record_id, val));
                Task::none()
            }
            Message::SaveNote(record_id, note) => {
                if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                    record.note = note;
                    let record = record.clone();
                    self.editing_note = None;
                    return Task::perform(
                        async move {
                            if let Ok(db) = store::open_db().await {
                                let _ = store::update_record(&db, &record).await;
                            }
                        },
                        |()| Message::Noop,
                    );
                }
                Task::none()
            }
            Message::SubmitCurrentNote => {
                if let Some((record_id, note)) = self.editing_note.take() {
                    if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                        record.note = note;
                        let record = record.clone();
                        return Task::perform(
                            async move {
                                if let Ok(db) = store::open_db().await {
                                    let _ = store::update_record(&db, &record).await;
                                }
                            },
                            |()| Message::Noop,
                        );
                    }
                }
                Task::none()
            }
            Message::DeleteCurrentNote => {
                if let Some((record_id, _)) = self.editing_note.take() {
                    if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                        record.note = String::new();
                        let record = record.clone();
                        return Task::perform(
                            async move {
                                if let Ok(db) = store::open_db().await {
                                    let _ = store::update_record(&db, &record).await;
                                }
                            },
                            |()| Message::Noop,
                        );
                    }
                }
                Task::none()
            }
            Message::OpenMetricEditor(record_id) => {
                // Toggle: close if already editing the same record
                if let Some((ref current_id, _)) = self.editing_metric {
                    if *current_id == record_id {
                        self.editing_metric = None;
                        return Task::none();
                    }
                }
                let metrics = self
                    .current_records
                    .iter()
                    .find(|r| r.id == record_id)
                    .map(|r| r.metrics.clone());
                if let Some(m) = metrics {
                    // Populate raw text buffers from current values
                    self.editing_metric_text = [
                        format!("{}", m.major_length),
                        format!("{}", m.minor_length),
                        m.a_eq.map_or(String::new(), |v| format!("{v}")),
                        m.b_eq.map_or(String::new(), |v| format!("{v}")),
                    ];
                    self.editing_metric = Some((record_id, m));
                    self.editing_note = None;
                }
                Task::none()
            }
            Message::MetricInputChanged(field_idx, raw_text) => {
                // Just update the text buffer; do NOT parse yet.
                if field_idx < 4 {
                    self.editing_metric_text[field_idx] = raw_text;
                }
                Task::none()
            }
            Message::SaveEditedMetric(record_id, mut metrics) => {
                metrics.manually_edited = true;
                if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                    record.metrics = metrics;
                    // Auto-clear suspect on edit
                    record.suspect = false;
                    let record = record.clone();
                    self.editing_metric = None;
                    return Task::perform(
                        async move {
                            if let Ok(db) = store::open_db().await {
                                let _ = store::update_record(&db, &record).await;
                            }
                        },
                        |()| Message::Noop,
                    );
                }
                Task::none()
            }
            Message::CancelEdit => {
                self.editing_note = None;
                self.editing_metric = None;
                self.editing_session_name = None;
                Task::none()
            }
            Message::StartRenameSession(sid) => {
                // Pre-fill with existing name, or empty for timestamp-named sessions
                let current_name = self.sessions.iter()
                    .find(|s| s.session_id == sid)
                    .and_then(|s| s.name.clone())
                    .unwrap_or_default();
                self.editing_session_name = Some((sid, current_name));
                Task::none()
            }
            Message::RenameSessionInput(val) => {
                if let Some((_, ref mut name)) = self.editing_session_name {
                    *name = val;
                }
                Task::none()
            }
            Message::SubmitSessionRename => {
                if let Some((sid, name)) = self.editing_session_name.take() {
                    let trimmed = name.trim().to_string();
                    let new_name = if trimmed.is_empty() { None } else { Some(trimmed) };
                    // Update in-memory session
                    if let Some(session) = self.sessions.iter_mut().find(|s| s.session_id == sid) {
                        session.name = new_name.clone();
                    }
                    // Persist to IndexedDB
                    let sid_owned = sid.clone();
                    return Task::perform(
                        async move {
                            if let Ok(db) = store::open_db().await {
                                let _ = store::rename_session(&db, &sid_owned, new_name).await;
                            }
                        },
                        |()| Message::Noop,
                    );
                }
                Task::none()
            }
            Message::SubmitCurrentMetric => {
                if let Some((record_id, mut metrics)) = self.editing_metric.take() {
                    // Parse raw text into metrics
                    let txt = &self.editing_metric_text;
                    if let Ok(v) = txt[0].parse::<f32>() { metrics.major_length = v; }
                    if let Ok(v) = txt[1].parse::<f32>() { metrics.minor_length = v; }
                    metrics.a_eq = txt[2].parse::<f32>().ok().or(metrics.a_eq);
                    metrics.b_eq = txt[3].parse::<f32>().ok().or(metrics.b_eq);

                    // On first manual edit, snapshot the original computed values
                    if metrics.original.is_none() {
                        if let Some(record) = self.current_records.iter().find(|r| r.id == record_id) {
                            metrics.original = Some(history::model::OriginalMetrics {
                                major_length: record.metrics.major_length,
                                minor_length: record.metrics.minor_length,
                                a_eq: record.metrics.a_eq,
                                b_eq: record.metrics.b_eq,
                            });
                        }
                    }
                    metrics.manually_edited = true;
                    if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                        record.metrics = metrics;
                        record.suspect = false;
                        let record = record.clone();
                        return Task::perform(
                            async move {
                                if let Ok(db) = store::open_db().await {
                                    let _ = store::update_record(&db, &record).await;
                                }
                            },
                            |()| Message::Noop,
                        );
                    }
                }
                Task::none()
            }
            Message::ResetCurrentMetric => {
                // Restore original program-computed values
                if let Some((record_id, _)) = self.editing_metric.take() {
                    if let Some(record) = self.current_records.iter_mut().find(|r| r.id == record_id) {
                        if let Some(orig) = record.metrics.original.take() {
                            record.metrics.major_length = orig.major_length;
                            record.metrics.minor_length = orig.minor_length;
                            record.metrics.a_eq = orig.a_eq;
                            record.metrics.b_eq = orig.b_eq;
                            record.metrics.manually_edited = false;
                            // Keep editor open with restored values
                            let m = &record.metrics;
                            self.editing_metric_text = [
                                format!("{}", m.major_length),
                                format!("{}", m.minor_length),
                                m.a_eq.map_or(String::new(), |v| format!("{v}")),
                                m.b_eq.map_or(String::new(), |v| format!("{v}")),
                            ];
                            self.editing_metric = Some((record_id.clone(), record.metrics.clone()));
                            let record = record.clone();
                            return Task::perform(
                                async move {
                                    if let Ok(db) = store::open_db().await {
                                        let _ = store::update_record(&db, &record).await;
                                    }
                                },
                                |()| Message::Noop,
                            );
                        }
                    }
                }
                Task::none()
            }
            Message::DeleteSelectedSessions => {
                // Show confirmation instead of deleting immediately
                let sids: Vec<String> = self.selected_sessions.iter().cloned().collect();
                if sids.is_empty() {
                    return Task::none();
                }
                let record_count = self
                    .current_records
                    .iter()
                    .filter(|r| sids.contains(&r.session_id))
                    .count() as u32;
                self.delete_confirm = Some((sids, record_count));
                Task::none()
            }
            Message::ConfirmDelete => {
                if let Some((sids, _)) = self.delete_confirm.take() {
                    let count = sids.len();
                    // Soft-delete: remove from view, cache for undo
                    let deleted_sessions: Vec<_> = self.sessions
                        .iter()
                        .filter(|s| sids.contains(&s.session_id))
                        .cloned()
                        .collect();
                    let deleted_records: Vec<_> = self.current_records
                        .iter()
                        .filter(|r| sids.contains(&r.session_id))
                        .cloned()
                        .collect();
                    let record_count = deleted_records.len();

                    self.selected_sessions.clear();
                    self.current_records.retain(|r| !sids.contains(&r.session_id));
                    self.sessions.retain(|s| !sids.contains(&s.session_id));

                    self.pending_delete = Some(PendingDelete {
                        sessions: deleted_sessions,
                        records: deleted_records,
                        sids: sids.clone(),
                    });
                    self.undo_toast = Some(format!(
                        "Deleted {count} session(s) ({record_count} records)"
                    ));
                    self.undo_countdown = Some(5);

                    // Countdown is driven by subscription tick, no gloo timer needed
                }
                Task::none()
            }
            Message::CancelDelete => {
                self.delete_confirm = None;
                Task::none()
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Task::none()
            }
            Message::SortBy(col) => {
                if self.sort_column == Some(col) {
                    self.sort_ascending = !self.sort_ascending;
                } else {
                    self.sort_column = Some(col);
                    self.sort_ascending = true;
                }
                Task::none()
            }
            Message::ClearAllHistory => {
                self.clear_all_confirm = true;
                Task::none()
            }
            Message::ConfirmClearAll => {
                self.clear_all_confirm = false;
                let sids: Vec<String> = self.sessions.iter().map(|s| s.session_id.clone()).collect();
                if sids.is_empty() {
                    return Task::none();
                }
                let session_count = sids.len();

                // Soft-delete: cache everything for undo
                let deleted_sessions = self.sessions.clone();
                let deleted_records = self.current_records.clone();
                let record_count = deleted_records.len();

                self.sessions.clear();
                self.selected_sessions.clear();
                self.current_records.clear();
                self.search_query.clear();

                self.pending_delete = Some(PendingDelete {
                    sessions: deleted_sessions,
                    records: deleted_records,
                    sids,
                });
                self.undo_toast = Some(format!(
                    "Cleared all history: {session_count} session(s) ({record_count} records)"
                ));
                self.undo_countdown = Some(5);

                Task::none()
            }
            Message::CancelClearAll => {
                self.clear_all_confirm = false;
                Task::none()
            }
            Message::ExportSelectedSessions => {
                // Export selected sessions' records as CSV
                let records = &self.current_records;
                if !records.is_empty() {
                    let mut csv = String::from("filename,height_mm,width_mm,volume_mm3,a_eq,b_eq,surface_area,n_total\n");
                    for r in records {
                        let m = &r.metrics;
                        csv += &format!(
                            "{},{:.2},{:.2},{:.0},{},{},{},{}\n",
                            r.filename,
                            m.major_length,
                            m.minor_length,
                            m.volume,
                            m.a_eq.map_or(String::new(), |v| format!("{v:.2}")),
                            m.b_eq.map_or(String::new(), |v| format!("{v:.2}")),
                            m.surface_area.map_or(String::new(), |v| format!("{v:.0}")),
                            m.n_total.map_or(String::new(), |v| format!("{v}")),
                        );
                    }
                    trigger_download(&csv, "pineapple_history.csv");
                    // Remember which sessions were exported and show prompt
                    self.exported_session_ids = self.selected_sessions.iter().cloned().collect();
                    self.export_delete_prompt = true;
                }
                Task::none()
            }
            Message::DeleteExportedSessions => {
                self.export_delete_prompt = false;
                let sids = std::mem::take(&mut self.exported_session_ids);
                if sids.is_empty() {
                    return Task::none();
                }
                let session_count = sids.len();

                // Soft-delete: cache for undo
                let deleted_sessions: Vec<_> = self.sessions
                    .iter()
                    .filter(|s| sids.contains(&s.session_id))
                    .cloned()
                    .collect();
                let deleted_records: Vec<_> = self.current_records
                    .iter()
                    .filter(|r| sids.contains(&r.session_id))
                    .cloned()
                    .collect();
                let record_count = deleted_records.len();

                self.selected_sessions.clear();
                self.current_records.retain(|r| !sids.contains(&r.session_id));
                self.sessions.retain(|s| !sids.contains(&s.session_id));

                self.pending_delete = Some(PendingDelete {
                    sessions: deleted_sessions,
                    records: deleted_records,
                    sids,
                });
                self.undo_toast = Some(format!(
                    "Deleted {session_count} exported session(s) ({record_count} records)"
                ));
                self.undo_countdown = Some(5);

                Task::none()
            }
            Message::DismissExportPrompt => {
                self.export_delete_prompt = false;
                self.exported_session_ids.clear();
                Task::none()
            }
            Message::PaneResized(iced::widget::pane_grid::ResizeEvent { split, ratio }) => {
                self.history_panes.resize(split, ratio);
                Task::none()
            }
            Message::QuickCleanup => Task::perform(
                async {
                    let db = store::open_db().await.ok()?;
                    store::cleanup_oldest_unstarred(&db).await.ok()
                },
                |opt| {
                    if let Some(result) = opt {
                        Message::CleanupDone(result)
                    } else {
                        Message::Noop
                    }
                },
            ),
            Message::CleanupDone(result) => {
                self.undo_toast = Some(format!(
                    "Cleaned up {} session(s) ({} records)",
                    result.sessions_deleted, result.records_deleted
                ));
                // Reload sessions
                Task::perform(
                    async {
                        let db = store::open_db().await.ok()?;
                        store::load_session_summaries(&db).await.ok()
                    },
                    |opt| Message::HistoryLoaded(opt.unwrap_or_default()),
                )
            }
            Message::CacheStatus(level) => {
                self.cache_warning = if level == CacheWarningLevel::Ok {
                    None
                } else {
                    Some(level)
                };
                Task::none()
            }
            Message::DismissCacheWarning => {
                self.cache_warning = None;
                Task::none()
            }
            Message::BatchSaved => {
                // Check cache status after saving
                Task::perform(
                    async {
                        let db = store::open_db().await.ok()?;
                        store::check_cache_status(&db).await.ok()
                    },
                    |opt| Message::CacheStatus(opt.unwrap_or(CacheWarningLevel::Ok)),
                )
            }
            Message::UndoDelete => {
                // Restore soft-deleted sessions/records
                if let Some(pending) = self.pending_delete.take() {
                    self.sessions.extend(pending.sessions);
                    self.current_records.extend(pending.records);
                    self.sessions.sort_by(|a, b| b.timestamp.partial_cmp(&a.timestamp).unwrap_or(std::cmp::Ordering::Equal));
                }
                self.undo_toast = None;
                self.undo_countdown = None;
                Task::none()
            }
            Message::UndoExpired => {
                // Commit the deletion to IndexedDB
                self.undo_toast = None;
                self.undo_countdown = None;
                if let Some(pending) = self.pending_delete.take() {
                    let sids = pending.sids;
                    return Task::perform(
                        async move {
                            if let Ok(db) = store::open_db().await {
                                let _ = store::delete_sessions(&db, &sids).await;
                            }
                        },
                        |()| Message::Noop,
                    );
                }
                Task::none()
            }
            Message::UndoTick => {
                if let Some(ref mut count) = self.undo_countdown {
                    if *count <= 1 {
                        // Time's up — trigger commit
                        return self.update(Message::UndoExpired, self.now);
                    }
                    *count -= 1;
                }
                Task::none()
            }
            Message::UndoToastMessage(msg) => {
                if !msg.is_empty() {
                    self.undo_toast = Some(msg);
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // ── Title bar ──
        let favicon_handle = image::Handle::from_bytes(
            include_bytes!("../assets/favicon.png").as_slice(),
        );
        // PornHub-style logo: "Pineapple" white + "Hub" black-on-orange badge
        let hub_badge = container(text("Hub").size(18).color(Color::BLACK))
            .padding([2, 6])
            .style(theme::hub_badge_style);
        let logo = button(
            row![
                image(favicon_handle).width(26).height(26),
                text("Pineapple").size(20).color(theme::TEXT_PRIMARY),
                hub_badge,
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        )
        .on_press(Message::NavigateTo(Page::Analysis))
        .style(theme::text_button_style)
        .padding([4, 8]);

        let title_bar = container(
            row![
                logo,
                space::horizontal().width(Length::Fill),
                tooltip(
                    button(text(icons::ICON_HELP).font(icons::ICON_FONT).size(20))
                        .on_press(Message::ToggleHelp)
                        .style(theme::text_button_style)
                        .padding(6),
                    "Help (F1)",
                    tooltip::Position::Bottom,
                ).style(theme::tooltip_style),
                tooltip(
                    button(text(icons::ICON_HISTORY).font(icons::ICON_FONT).size(20))
                        .on_press(Message::NavigateTo(Page::History {
                            panel: HistoryPanel::Records,
                            sidebar_open: true,
                        }))
                        .style(theme::text_button_style)
                        .padding(6),
                    "History",
                    tooltip::Position::Bottom,
                ).style(theme::tooltip_style),
                tooltip(
                    button(
                        image(image::Handle::from_bytes(
                            include_bytes!("../assets/github-mark-white.png").as_slice(),
                        ))
                        .width(20)
                        .height(20),
                    )
                    .on_press(Message::OpenGitHub)
                    .style(theme::text_button_style)
                    .padding(6),
                    "GitHub Repository",
                    tooltip::Position::Bottom,
                ).style(theme::tooltip_style),
            ]
            .spacing(8)
            .padding([14, 20])
            .align_y(iced::Alignment::Center),
        )
        .style(theme::title_bar_style)
        .width(Length::Fill);

        // ── Page content ──
        let page_content: Element<'_, Message> = match &self.page {
            Page::Analysis => self.view_analysis(),
            Page::History { panel, sidebar_open } => self.view_history(panel, *sidebar_open),
        };

        let mut main_layout = column![
            title_bar,
            container(space::horizontal().width(0))
                .width(Length::Fill)
                .height(2)
                .style(theme::accent_separator),
        ].spacing(0).height(Length::Fill);

        // ── Undo toast (placed before page content so it's visible) ──
        if let (Some(msg), Some(cd)) = (&self.undo_toast, self.undo_countdown) {
            main_layout = main_layout.push(history_view::view_undo_toast(msg, cd));
        }

        main_layout = main_layout.push(page_content);

        let viewer = self.viewer.view(self.now);
        let mut layers = stack![main_layout, viewer];

        // ── Export-then-delete dialog (centered overlay) ──
        if self.export_delete_prompt {
            layers = layers.push(history_view::view_export_delete_prompt());
        }

        // Full-screen decoding overlay — blocks interaction visually
        if self.decoding {
            let (current, total) = self.decode_progress;
            let progress_text = if total > 0 {
                format!("Decoding images ({}/{})", current + 1, total)
            } else {
                "Decoding images...".to_string()
            };
            let overlay = container(
                row![
                    text(icons::ICON_HOURGLASS_TOP).font(icons::ICON_FONT).size(24),
                    text(progress_text).size(24),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .style(theme::overlay_style)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill);
            layers = layers.push(overlay);
        }

        // D2R-style help overlay
        if self.show_help {
            let help_layer = match &self.page {
                Page::Analysis => ui::help_overlay::view_analysis_help(),
                Page::History { .. } => ui::help_overlay::view_history_help(),
            };
            layers = layers.push(help_layer);
        }

        layers.into()
    }

    /// Analysis page — the original 3-column layout.
    fn view_analysis(&self) -> Element<'_, Message> {
        // ── Left column: file input + file list ──
        let mut left_col = column![].spacing(12).padding(12).width(Length::FillPortion(3));

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
                    JobStatus::Queued => icons::ICON_HOURGLASS,
                    JobStatus::Processing => icons::ICON_SYNC,
                    JobStatus::Done => icons::ICON_CHECK_CIRCLE,
                    JobStatus::Error(_) => icons::ICON_ERROR,
                };
                let is_selected = self.selected_job == Some(job.id);
                let row_content: Element<'_, Message> = row![
                    text(icon).font(icons::ICON_FONT).size(14),
                    text(&job.filename).width(Length::Fill),
                ]
                .spacing(8)
                .into();

                if is_selected {
                    button(container(row_content).style(theme::selected_job_style))
                        .padding([8, 12])
                        .style(theme::text_button_style)
                        .on_press(Message::SelectJob(job.id))
                        .into()
                } else {
                    button(row_content)
                        .padding([8, 12])
                        .style(theme::text_button_style)
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
                    .label("Pipeline Details")
                    .on_toggle(Message::TogglePipeline),
            ]
            .spacing(10)
            .width(Length::FillPortion(5));

            if self.selected_job.is_none() {
                col = col.push(
                    container(text("← Select a file to view details"))
                        .center(Length::Fill)
                        .padding(40),
                );
            } else if !self.intermediates.is_empty() {
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
                    info = info.push(text(format!("H: {:.2} mm", m.major_length)).size(14));
                    info = info.push(text(format!("D: {:.2} mm", m.minor_length)).size(14));
                    info = info.push(text(format!("V: {:.0} mm³", m.volume)).size(14));
                    if let Some(v) = m.a_eq {
                        info = info.push(text(format!("a: {v:.2} mm")).size(14));
                    }
                    if let Some(v) = m.b_eq {
                        info = info.push(text(format!("b: {v:.2} mm")).size(14));
                    }
                    if let Some(v) = m.surface_area {
                        info = info.push(text(format!("S: {v:.0} mm²")).size(14));
                    }
                    if let Some(v) = m.n_total {
                        info = info.push(text(format!("Nf: {v}")).size(14));
                    }
                }

                col = col.push(container(info));
            }

            col.into()
        } else if !self.jobs.is_empty() {
            column![
                toggler(self.show_pipeline)
                    .label("Pipeline Details")
                    .on_toggle(Message::TogglePipeline),
            ]
            .width(Length::Shrink)
            .into()
        } else {
            space::horizontal().into()
        };

        // ── Right column: results table ──
        let mut right_col = column![text("Results").size(22)].spacing(8).padding(12).width(Length::FillPortion(4));

        use history::stats::MetricColumn;
        let tip_hdr = |label: &'static str, tip: &'static str, portion: u16| -> Element<'_, Message> {
            tooltip(
                text(label).width(Length::FillPortion(portion)),
                tip,
                tooltip::Position::Bottom,
            )
            .style(theme::tooltip_style)
            .into()
        };
        let header = row![
            tip_hdr("File", "Source image filename", 3),
            tip_hdr("H", MetricColumn::Height.description(), 1),
            tip_hdr("D", MetricColumn::Width.description(), 1),
            tip_hdr("V", MetricColumn::Volume.description(), 1),
            tip_hdr("a", MetricColumn::Aeq.description(), 1),
            tip_hdr("b", MetricColumn::Beq.description(), 1),
            tip_hdr("S", MetricColumn::SurfaceArea.description(), 1),
            tip_hdr("Nf", MetricColumn::NTotal.description(), 1),
        ]
        .spacing(4);
        right_col = right_col.push(container(header).style(theme::table_header_style).padding([8, 6]));

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
            let rows = column(completed_jobs.iter().enumerate().map(|(idx, job)| {
                let m = job.metrics.as_ref().unwrap();
                let row_bg = theme::table_row_bg(idx, false, false);
                container(
                    row![
                        text(&job.filename).size(13).width(Length::FillPortion(3)),
                        text(format!("{:.1}", m.major_length)).size(13).width(Length::FillPortion(1)),
                        text(format!("{:.1}", m.minor_length)).size(13).width(Length::FillPortion(1)),
                        text(format!("{:.0}", m.volume)).size(13).width(Length::FillPortion(1)),
                        text(m.a_eq.map_or("-".into(), |v| format!("{v:.1}"))).size(13).width(Length::FillPortion(1)),
                        text(m.b_eq.map_or("-".into(), |v| format!("{v:.1}"))).size(13).width(Length::FillPortion(1)),
                        text(m.surface_area.map_or("-".into(), |v| format!("{v:.0}"))).size(13).width(Length::FillPortion(1)),
                        text(m.n_total.map_or("-".into(), |v| format!("{v}"))).size(13).width(Length::FillPortion(1)),
                    ]
                    .spacing(4)
                    .align_y(iced::Alignment::Center),
                )
                .style(row_bg)
                .padding([6, 6])
                .into()
            }))
            .spacing(1);
            right_col = right_col.push(scrollable(rows));
        }

        if !completed_jobs.is_empty() {
            right_col = right_col.push(button("Export CSV").on_press(Message::ExportCsv));
        }

        container(
            row![
                scrollable(left_col),
                mid_col,
                scrollable(right_col),
            ]
            .spacing(16)
            .padding(12),
        )
        .height(Length::Fill)
        .into()
    }

    /// History page — Sessions Sidebar + Tab Bar + Main Panel.
    fn view_history(&self, panel: &HistoryPanel, sidebar_open: bool) -> Element<'_, Message> {
        use iced::widget::pane_grid;

        let mut row_layout = row![].spacing(0).height(Length::Fill);

        if sidebar_open {
            // Use pane_grid for resizable sidebar/main panel
            let sessions = &self.sessions;
            let selected_sessions = &self.selected_sessions;
            let cache_warning = &self.cache_warning;
            let delete_confirm = &self.delete_confirm;
            let clear_all_confirm = self.clear_all_confirm;
            let current_records = &self.current_records;
            let editing_note = &self.editing_note;
            let editing_metric = &self.editing_metric;
            let editing_session_name = &self.editing_session_name;
            let search_query = &self.search_query;
            let sort_column = self.sort_column;
            let sort_ascending = self.sort_ascending;
            let outlier_cells = &self.outlier_cells;
            let column_stats = &self.column_stats;

            let pg = pane_grid(&self.history_panes, move |_pane, state, _is_maximized| {
                match state {
                    HistoryPane::Sidebar => {
                        pane_grid::Content::new(
                            history_view::view_sessions_sidebar(
                                sessions,
                                selected_sessions,
                                cache_warning,
                                delete_confirm,
                                clear_all_confirm,
                                editing_session_name,
                            ),
                        )
                        .style(theme::sidebar_style)
                    }
                    HistoryPane::MainPanel => {
                        pane_grid::Content::new(history_view::view_main_content(
                            panel,
                            current_records,
                            selected_sessions.len(),
                            editing_note,
                            editing_metric,
                            &self.editing_metric_text,
                            self.record_filter,
                            search_query,
                            sort_column,
                            sort_ascending,
                            outlier_cells,
                            column_stats,
                            &self.parallel_coords_chart,
                            &self.highlight_record_id,
                            self.highlight_ticks,
                            true,
                        ))
                    }
                }
            })
            .on_resize(6, Message::PaneResized)
            .height(Length::Fill);

            row_layout = row_layout.push(pg);
        } else {
            // Sidebar collapsed — show only main panel with expand button
            let main_content = history_view::view_main_content(
                panel,
                &self.current_records,
                self.selected_sessions.len(),
                &self.editing_note,
                &self.editing_metric,
                &self.editing_metric_text,
                self.record_filter,
                &self.search_query,
                self.sort_column,
                self.sort_ascending,
                &self.outlier_cells,
                &self.column_stats,
                &self.parallel_coords_chart,
                &self.highlight_record_id,
                self.highlight_ticks,
                false,
            );

            row_layout = row_layout.push(
                container(main_content)
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        }

        row_layout.into()
    }
}

/// Fast-mode pipeline: processes a file from bytes to metrics without generating
/// any intermediate images or previews.
///
/// TODO Phase 2: This will be called from rayon worker threads.
// run_pipeline_fast has been moved to pipeline/fast.rs
// for Web Worker thread safety (no iced types / browser APIs).

fn pineapple_app_theme(_app: &App) -> iced::Theme {
    theme::pineapple_theme()
}

fn main() -> iced::Result {
    console_log::init().expect("Initialize logger");
    console_error_panic_hook::set_once();

    iced::application::timed(App::new, App::update, App::subscription, App::view)
        .centered()
        .theme(pineapple_app_theme)
        .font(NOTO_SANS_SC_BYTES)
        .font(icons::ICON_FONT_BYTES)
        .run()
}
