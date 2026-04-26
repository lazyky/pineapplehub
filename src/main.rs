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
    theme::ThemeVariant,
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
    /// Cycle to the next theme variant.
    SwitchTheme,

    /// Window was resized — update cached dimensions.
    WindowResized(f32, f32),

    // ── Camera page ──
    /// User changed the session mode in the Camera page.
    CameraSessionModeChanged(CameraSessionMode),
    /// Shutter button pressed — start capture.
    CameraCapture,
    /// A photo has been captured (or failed).
    CameraCaptured(Result<js_interop::FileEntry, crate::error::Error>),
    /// Remove a queued capture by index.
    CameraRemoveCapture(usize),
    /// "Analyze" button pressed — run the pipeline on queued captures.
    CameraStartAnalysis,
    /// Session list loaded lazily for "append to existing" mode.
    CameraSessionsLoaded(Vec<SessionSummary>),
    /// Camera permission was denied by the user.
    CameraPermissionDenied,
}

// ────────────────────────  History Pane  ────────────────────────

#[derive(Clone, Debug)]
enum HistoryPane {
    Sidebar,
    MainPanel,
}

/// Session association mode for camera captures.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum CameraSessionMode {
    /// Create a new session automatically after analysis.
    #[default]
    NewSession,
    /// Append results to an existing session (selected by session_id).
    AppendTo(String),
    /// Standalone — do not persist results.
    Standalone,
}

impl CameraSessionMode {
    /// Serialize to the string stored in localStorage.
    fn as_key(&self) -> &'static str {
        match self {
            Self::NewSession => "new",
            Self::AppendTo(_) => "append",
            Self::Standalone => "standalone",
        }
    }

    /// Deserialize from the string stored in localStorage.
    fn from_key(key: &str) -> Self {
        match key {
            "append" => Self::AppendTo(String::new()), // session_id filled in separately
            "standalone" => Self::Standalone,
            _ => Self::NewSession,
        }
    }
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
    /// Batch completion summary toast: (success_count, failed_count).
    batch_toast: Option<(u32, u32)>,
    /// Countdown (ticks) before undo toast disappears.
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

    /// Currently active theme variant.
    theme_variant: ThemeVariant,

    /// Current window dimensions in logical pixels (updated via subscription).
    /// Used for portrait detection on mobile.
    window_width: f32,
    window_height: f32,

    // ── Camera page state ──
    /// Currently selected session association mode.
    camera_mode: CameraSessionMode,
    /// Photos queued for analysis (captured but not yet processed).
    camera_queue: Vec<js_interop::FileEntry>,
    /// Session list shown in the "append to existing" picker (lazily loaded).
    camera_sessions: Vec<SessionSummary>,
    /// Whether the camera sessions list is currently loading.
    camera_sessions_loading: bool,
    /// Camera capture is in progress.
    camera_capturing: bool,
    /// Last camera error message to display to the user.
    camera_error: Option<String>,
    /// The camera mode that was active when CameraStartAnalysis was triggered.
    /// Cleared after each batch completes. None means this batch came from a
    /// normal file upload (not the camera page).
    camera_batch_mode: Option<CameraSessionMode>,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        // Detect if camera mode was restored as AppendTo so we can
        // pre-load the session list without waiting for user interaction.
        let camera_mode = {
            let base = js_interop::load_camera_mode()
                .map(|k| CameraSessionMode::from_key(&k))
                .unwrap_or_default();
            if let CameraSessionMode::AppendTo(_) = &base {
                let sid = js_interop::load_camera_append_session().unwrap_or_default();
                CameraSessionMode::AppendTo(sid)
            } else {
                base
            }
        };

        let is_append = matches!(camera_mode, CameraSessionMode::AppendTo(_));
        let startup_task: Task<Message> =
            if is_append {
                Task::perform(
                    async {
                        let db = store::open_db().await.ok()?;
                        store::load_session_summaries(&db).await.ok()
                    },
                    |opt| Message::CameraSessionsLoaded(opt.unwrap_or_default()),
                )
            } else {
                Task::none()
            };

        let app = Self {
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
            batch_toast: None,
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
            theme_variant: ThemeVariant::default(),
            window_width: web_sys::window()
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(1024.0) as f32,
            window_height: web_sys::window()
                .and_then(|w| w.inner_height().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(768.0) as f32,
            camera_mode,
            camera_queue: Vec::new(),
            camera_sessions: Vec::new(),
            // Mark loading=true so the picker shows "Loading..." until the
            // startup task resolves (or false if mode is not AppendTo).
            camera_sessions_loading: is_append,
            camera_capturing: false,
            camera_error: None,
            camera_batch_mode: None,
        };
        (app, startup_task)
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

        // Window resize — update portrait detection
        subs.push(
            iced::event::listen_with(|event, _status, _window| {
                if let iced::Event::Window(iced::window::Event::Resized(size)) = event {
                    Some(Message::WindowResized(size.width, size.height))
                } else {
                    None
                }
            }),
        );

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
                log::info!("[batch] BatchStart received; state={}", match &self.upload.state {
                    State::Idle => "Idle",
                    State::Uploading { .. } => "Uploading",
                    State::Finished(_) => "Finished",
                    State::Errored => "Errored",
                });
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

                    // Generate session ID:
                    // - Normal upload: only for ≥2 files (single-image stays in debug mode)
                    // - Camera NewSession: always (even 1 photo needs a session)
                    // - Camera AppendTo: the session already exists — use that ID directly
                    // - Camera Standalone: no persistence, leave None
                    self.current_session_id = match &self.camera_batch_mode {
                        Some(CameraSessionMode::NewSession) => Some(store::generate_id()),
                        Some(CameraSessionMode::AppendTo(sid)) if !sid.is_empty() => {
                            Some(sid.clone())
                        }
                        Some(CameraSessionMode::AppendTo(_)) => None, // no session selected
                        Some(CameraSessionMode::Standalone) => None,
                        None => {
                            // Regular file-upload mode: only batch (≥2) gets a session
                            if num_jobs >= 2 { Some(store::generate_id()) } else { None }
                        }
                    };

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
                log::info!("[batch] StartDecoding; jobs={}", self.jobs.len());
                // Now the overlay is visible. Start the streaming pipeline.
                if let State::Finished(entries) = &self.upload.state {
                    let entries_clone: Vec<FileEntry> = entries.clone();
                    let _num_jobs = self.jobs.len();
                    return Task::run(iced::stream::channel(1, move |mut output: futures::channel::mpsc::Sender<Message>| async move {
                        use futures::SinkExt;
                        let total = entries_clone.len();
                        web_sys::console::log_1(&format!("[stream] Phase 1 begin: {} entries", total).into());

                        // Phase 1: sequential decode on main thread
                        let mut prepared = Vec::with_capacity(total);
                        for (id, entry) in entries_clone.iter().enumerate() {
                            web_sys::console::log_1(&format!("[stream] decoding entry {id}").into());
                            let _ = output.send(Message::DecodingProgress(id, total)).await;
                            gloo_timers::future::TimeoutFuture::new(0).await;
                            match pipeline::fast::prepare_image(entry) {
                                Ok(prep) => {
                                    web_sys::console::log_1(&format!("[stream] entry {id} decoded OK").into());
                                    prepared.push((id, prep));
                                }
                                Err(e) => {
                                    web_sys::console::log_1(&format!("[stream] entry {id} decode ERR: {e}").into());
                                    let _ = output.send(Message::JobDone(id, Err(e))).await;
                                }
                            }
                        }

                        // Phase 1 done — dismiss overlay immediately
                        web_sys::console::log_1(&format!("[stream] Phase 1 done: {} prepared", prepared.len()).into());
                        let _ = output.send(Message::DecodingDone).await;

                        // Phase 2: spawn rayon tasks, collect results via channel
                        let (tx, mut rx) = futures::channel::mpsc::unbounded();
                        let count = prepared.len();
                        web_sys::console::log_1(&format!("[stream] Phase 2 begin: spawning {count} rayon tasks").into());

                        for (id, prep) in prepared {
                            let tx = tx.clone();
                            rayon::spawn(move || {
                                let result = std::panic::catch_unwind(
                                    std::panic::AssertUnwindSafe(|| {
                                        pipeline::fast::process_prepared(&prep)
                                    })
                                );
                                let result = match result {
                                    Ok(r) => r,
                                    Err(panic_info) => {
                                        let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                                            s.clone()
                                        } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                                            s.to_string()
                                        } else {
                                            "Unknown panic in pipeline".to_string()
                                        };
                                        web_sys::console::error_1(
                                            &format!("[stream] worker {id} PANIC caught: {msg}").into()
                                        );
                                        Err(crate::error::Error::General(
                                            format!("Pipeline panic: {msg}")
                                        ))
                                    }
                                };
                                let _ = tx.unbounded_send(Message::JobDone(id, result));
                            });
                        }
                        drop(tx);
                        web_sys::console::log_1(&"[stream] all rayon tasks spawned".into());

                        // Forward results to iced as each worker completes
                        use futures::StreamExt;
                        let mut received = 0;
                        while let Some(msg) = rx.next().await {
                            let _ = output.send(msg).await;
                            received += 1;
                            web_sys::console::log_1(&format!("[stream] result {received}/{count} received").into());
                            if received >= count {
                                break;
                            }
                        }
                        web_sys::console::log_1(&"[stream] stream complete".into());
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
                    // Show batch summary toast
                    let ok = self.jobs.iter().filter(|j| j.status == JobStatus::Done).count() as u32;
                    let fail = self.jobs.iter().filter(|j| matches!(j.status, JobStatus::Error(_))).count() as u32;
                    self.batch_toast = Some((ok, fail));

                    // Take camera mode snapshot (clear so next upload is a fresh slate)
                    let batch_mode = self.camera_batch_mode.take();

                    if let Some(session_id) = self.current_session_id.take() {
                        let total_count = self.jobs.len() as u32;
                        let failed_count = fail;

                        let timestamp = js_sys::Date::now();

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

                        // Route storage: AppendTo → append, NewSession/normal → save_session
                        let is_append = matches!(
                            &batch_mode, Some(CameraSessionMode::AppendTo(_))
                        );

                        return Task::perform(
                            async move {
                                match store::open_db().await {
                                    Ok(db) => {
                                        if is_append {
                                            match store::append_to_session(&db, &session_id, &records).await {
                                                Ok(()) => log::info!("Camera batch appended to session {session_id}"),
                                                Err(e) => log::error!("Failed to append camera batch: {e:?}"),
                                            }
                                        } else {
                                            let meta = SessionMeta {
                                                session_id: session_id.clone(),
                                                timestamp,
                                                total_count,
                                                success_count: total_count - failed_count,
                                                failed_count,
                                                starred: false,
                                                name: {
                                                    // Default session name = local date-time
                                                    // (user can rename later in History)
                                                    use js_sys::Date;
                                                    let d = Date::new_0();
                                                    let y = d.get_full_year();
                                                    let mo = d.get_month() + 1;
                                                    let day = d.get_date();
                                                    let h = d.get_hours();
                                                    let mi = d.get_minutes();
                                                    Some(format!("{y:04}-{mo:02}-{day:02} {h:02}:{mi:02}"))
                                                },
                                            };
                                            match store::save_session(&db, &meta, &records).await {
                                                Ok(()) => log::info!("Batch saved successfully"),
                                                Err(e) => log::error!("Failed to save batch: {e:?}"),
                                            }
                                        }
                                    }
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
            Message::SwitchTheme => {
                self.theme_variant = self.theme_variant.next();
                theme::set_active_palette(self.theme_variant);
                Task::none()
            }
            Message::WindowResized(w, h) => {
                // iced gives us logical pixels (CSS pixels on WASM with DPR=1 scale).
                // Cross-check with window.innerWidth to guard against DPR scaling issues.
                let (css_w, css_h) = {
                    let win = web_sys::window();
                    let cw = win.as_ref()
                        .and_then(|w| w.inner_width().ok())
                        .and_then(|v| v.as_f64())
                        .unwrap_or(w as f64) as f32;
                    let ch = win.as_ref()
                        .and_then(|w| w.inner_height().ok())
                        .and_then(|v| v.as_f64())
                        .unwrap_or(h as f64) as f32;
                    (cw, ch)
                };
                self.window_width = css_w;
                self.window_height = css_h;
                Task::none()
            }

            // ── Camera page ──
            Message::CameraSessionModeChanged(mode) => {
                js_interop::save_camera_mode(mode.as_key());
                // IMPORTANT: set mode first so the UI always reflects the click,
                // even if we also kick off an async session-list load below.
                self.camera_mode = mode.clone();
                if let CameraSessionMode::AppendTo(ref sid) = mode {
                    if !sid.is_empty() {
                        js_interop::save_camera_append_session(sid);
                    } else if !self.camera_sessions_loading {
                        // Lazily load session list on first switch to "append"
                        self.camera_sessions_loading = true;
                        return Task::perform(
                            async {
                                let db = store::open_db().await.ok()?;
                                store::load_session_summaries(&db).await.ok()
                            },
                            |opt| Message::CameraSessionsLoaded(opt.unwrap_or_default()),
                        );
                    }
                }
                Task::none()
            }
            Message::CameraSessionsLoaded(sessions) => {
                self.camera_sessions = sessions;
                self.camera_sessions_loading = false;
                // If AppendTo still has empty sid, default to first session
                if let CameraSessionMode::AppendTo(ref sid) = self.camera_mode {
                    if sid.is_empty() {
                        if let Some(first) = self.camera_sessions.first() {
                            js_interop::save_camera_append_session(&first.session_id);
                            self.camera_mode = CameraSessionMode::AppendTo(first.session_id.clone());
                        }
                    }
                }
                Task::none()
            }
            Message::CameraCapture => {
                if self.camera_capturing { return Task::none(); }
                self.camera_capturing = true;
                self.camera_error = None;
                Task::future(async {
                    Message::CameraCaptured(js_interop::capture_photo().await)
                })
            }
            Message::CameraCaptured(result) => {
                self.camera_capturing = false;
                match result {
                    Ok(entry) => { self.camera_queue.push(entry); }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("denied") || msg.contains("NotAllowed") {
                            self.camera_error = Some("Camera permission denied. Please allow camera access in your browser.".into());
                        } else {
                            self.camera_error = Some(format!("Capture failed: {msg}"));
                        }
                    }
                }
                Task::none()
            }
            Message::CameraRemoveCapture(idx) => {
                if idx < self.camera_queue.len() {
                    self.camera_queue.remove(idx);
                }
                Task::none()
            }
            Message::CameraStartAnalysis => {
                log::info!("[camera] CameraStartAnalysis: queue_len={}", self.camera_queue.len());
                if self.camera_queue.is_empty() { return Task::none(); }
                // Wrap queued photos as Upload entries and run pipeline
                let entries: Vec<js_interop::FileEntry> = self.camera_queue.drain(..).collect();
                log::info!("[camera] entries drained: {}", entries.len());
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
                self.upload.state = State::Finished(entries);
                self.show_pipeline = false;
                self.selected_job = None;
                self.intermediates.clear();
                // Snapshot the camera mode so JobDone can route storage correctly
                self.camera_batch_mode = Some(self.camera_mode.clone());
                // Navigate to Analysis and kick off batch
                self.page = Page::Analysis;
                log::info!("[camera] firing BatchStart, jobs={} state=Finished", self.jobs.len());
                return Task::done(Message::BatchStart);
            }
            Message::CameraPermissionDenied => {
                self.camera_error = Some("Camera permission denied.".into());
                self.camera_capturing = false;
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
        // ── Global portrait guard ──
        // On mobile (width < 600px) in portrait orientation, the multi-column
        // layouts on both Analysis and History pages are unusable. Show a full-
        // screen rotate prompt instead of rendering broken UI.
        // Exception: Camera page is explicitly designed for portrait phone use.
        let is_mobile = js_interop::is_mobile();
        let is_portrait = self.window_width < self.window_height;
        if is_mobile && is_portrait && !matches!(self.page, Page::Camera) {
            return self.view_portrait_prompt();
        }

        // ── Title bar ──
        let favicon_handle = image::Handle::from_bytes(
            include_bytes!("../assets/favicon.png").as_slice(),
        );
        // Logo: "Pineapple" white + "Hub" black-on-accent badge
        let hub_badge = container(text("Hub").size(18).color(Color::BLACK))
            .padding([2, 6])
            .style(theme::hub_badge_style);
        let logo = button(
            row![
                image(favicon_handle).width(26).height(26),
                text("Pineapple").size(20).color(theme::text_primary()),
                hub_badge,
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        )
        .on_press(Message::NavigateTo(Page::Analysis))
        .style(theme::text_button_style)
        .padding([4, 8]);

        let camera_btn: Option<Element<'_, Message>> = if js_interop::is_mobile() {
            Some(
                tooltip(
                    button(text(icons::ICON_PHOTO_CAMERA).font(icons::ICON_FONT).size(20))
                        .on_press(Message::NavigateTo(Page::Camera))
                        .style(theme::text_button_style)
                        .padding(6),
                    "Camera",
                    tooltip::Position::Bottom,
                ).style(theme::tooltip_style).into()
            )
        } else {
            None
        };

        let mut title_row = row![
            logo,
            space::horizontal().width(Length::Fill),
        ].spacing(8).align_y(iced::Alignment::Center);

        if let Some(btn) = camera_btn {
            title_row = title_row.push(btn);
        }

        title_row = title_row
            .push(tooltip(
                button(text(icons::ICON_HELP).font(icons::ICON_FONT).size(20))
                    .on_press(Message::ToggleHelp)
                    .style(theme::text_button_style)
                    .padding(6),
                "Help (F1)",
                tooltip::Position::Bottom,
            ).style(theme::tooltip_style))
            .push(tooltip(
                button(text(icons::ICON_HISTORY).font(icons::ICON_FONT).size(20))
                    .on_press(Message::NavigateTo(Page::History {
                        panel: HistoryPanel::Records,
                        sidebar_open: true,
                    }))
                    .style(theme::text_button_style)
                    .padding(6),
                "History",
                tooltip::Position::Bottom,
            ).style(theme::tooltip_style))
            .push(tooltip(
                button(text(icons::ICON_PALETTE).font(icons::ICON_FONT).size(20))
                    .on_press(Message::SwitchTheme)
                    .style(theme::text_button_style)
                    .padding(6),
                self.theme_variant.label(),
                tooltip::Position::Bottom,
            ).style(theme::tooltip_style))
            .push(tooltip(
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
            ).style(theme::tooltip_style));

        let title_bar = container(
            title_row.padding([14, 20]),
        )
        .style(theme::title_bar_style)
        .width(Length::Fill);

        // ── Page content ──
        let page_content: Element<'_, Message> = match &self.page {
            Page::Analysis => self.view_analysis(),
            Page::History { panel, sidebar_open } => self.view_history(panel, *sidebar_open),
            Page::Camera => self.view_camera(),
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
                Page::History { .. } | Page::Camera => ui::help_overlay::view_history_help(),
            };
            layers = layers.push(help_layer);
        }

        layers.into()
    }

    /// Full-screen portrait lock overlay.
    ///
    /// Shown on mobile devices (width < 600px) when held in portrait mode.
    /// Both the Analysis and History layouts require horizontal space and are
    /// unusable in portrait, so we gate the entire app behind this prompt.
    fn view_portrait_prompt(&self) -> Element<'_, Message> {
        container(
            column![
                text(icons::ICON_SCREEN_ROTATION)
                    .font(icons::ICON_FONT)
                    .size(64),
                text("Please rotate your device")
                    .size(24),
                text("PineappleHub works best in landscape mode")
                    .size(15)
                    .color(Color { a: 0.55, ..theme::text_primary() }),
            ]
            .spacing(16)
            .align_x(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .style(theme::overlay_style)
        .into()
    }

    /// Camera capture page — mobile-only full-screen capture interface.
    fn view_camera(&self) -> Element<'_, Message> {
        use iced::widget::{button, column, row, scrollable, text};

        let accent = theme::accent();

        // ── Session mode selector ──
        let mode_btn = |label: &'static str, icon: &'static str,
                        mode: CameraSessionMode| -> Element<'_, Message> {
            let is_selected = match (&self.camera_mode, &mode) {
                (CameraSessionMode::NewSession, CameraSessionMode::NewSession) => true,
                (CameraSessionMode::AppendTo(_), CameraSessionMode::AppendTo(_)) => true,
                (CameraSessionMode::Standalone, CameraSessionMode::Standalone) => true,
                _ => false,
            };
            let bg_color = if is_selected {
                Color { a: 0.20, ..accent }
            } else {
                Color::TRANSPARENT
            };
            let border_color = if is_selected { accent } else {
                Color { a: 0.30, ..theme::text_primary() }
            };
            container(
                button(
                    column![
                        text(icon).font(icons::ICON_FONT).size(22),
                        text(label).size(12),
                    ]
                    .spacing(4)
                    .align_x(iced::Alignment::Center),
                )
                .on_press(Message::CameraSessionModeChanged(mode))
                .style(theme::text_button_style)
                .padding([10, 16]),
            )
            .style(move |_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(bg_color)),
                border: iced::Border {
                    color: border_color,
                    width: 1.5,
                    radius: 10.0.into(),
                },
                ..Default::default()
            })
            .into()
        };

        let session_row = row![
            mode_btn("New Session", icons::ICON_ADD_BOX, CameraSessionMode::NewSession),
            mode_btn("Append", icons::ICON_HISTORY, CameraSessionMode::AppendTo(String::new())),
            mode_btn("Standalone", icons::ICON_PHOTO_CAMERA, CameraSessionMode::Standalone),
        ]
        .spacing(12);

        // If "Append" is selected, show the session picker below
        let append_picker: Option<Element<'_, Message>> =
            if let CameraSessionMode::AppendTo(ref current_sid) = self.camera_mode {
                if self.camera_sessions_loading {
                    Some(text("Loading sessions...").size(13).into())
                } else if self.camera_sessions.is_empty() {
                    Some(text("No sessions available.").size(13)
                        .color(Color { a: 0.55, ..theme::text_primary() })
                        .into())
                } else {
                    let items: Vec<Element<'_, Message>> = self.camera_sessions.iter().map(|s| {
                        let sid = s.session_id.clone();
                        let is_sel = *current_sid == sid;
                        let label_color = if is_sel { accent } else { theme::text_primary() };
                        let display_name = s.name.as_deref().unwrap_or("Unnamed");
                        button(
                            text(format!("{} ({} records)", display_name, s.total_count))
                                .size(13)
                                .color(label_color),
                        )
                        .on_press(Message::CameraSessionModeChanged(
                            CameraSessionMode::AppendTo(sid)
                        ))
                        .style(theme::text_button_style)
                        .padding([6, 12])
                        .into()
                    }).collect();
                    Some(
                        container(
                            scrollable(column(items).spacing(4))
                                .height(160)
                        )
                        .style(theme::section_card_style)
                        .padding(8)
                        .width(Length::Fill)
                        .into()
                    )
                }
            } else {
                None
            };

        // ── Shutter button ──
        let shutter_icon = if self.camera_capturing {
            icons::ICON_HOURGLASS
        } else {
            icons::ICON_PHOTO_CAMERA
        };
        let shutter_label = if self.camera_capturing { "Capturing…" } else { "Take Photo" };
        let mut shutter_btn = button(
            column![
                text(shutter_icon).font(icons::ICON_FONT).size(36),
                text(shutter_label).size(14),
            ]
            .spacing(4)
            .align_x(iced::Alignment::Center),
        )
        .style(theme::primary_button_style)
        .padding([20, 40]);
        if !self.camera_capturing {
            shutter_btn = shutter_btn.on_press(Message::CameraCapture);
        }

        // ── Error message ──
        let error_row: Option<Element<'_, Message>> = self.camera_error.as_ref().map(|e| {
            row![
                text(icons::ICON_WARNING).font(icons::ICON_FONT).size(14).color(theme::danger()),
                text(e).size(13).color(theme::danger()),
            ]
            .spacing(6)
            .into()
        });

        // ── Queued captures strip ──
        let queue_strip: Option<Element<'_, Message>> = if !self.camera_queue.is_empty() {
            let chips: Vec<Element<'_, Message>> = self.camera_queue.iter().enumerate().map(|(i, entry)| {
                let idx = i;
                row![
                    text(icons::ICON_PHOTO_CAMERA).font(icons::ICON_FONT).size(13),
                    text(&entry.name).size(12),
                    button(text(icons::ICON_CLOSE).font(icons::ICON_FONT).size(12))
                        .on_press(Message::CameraRemoveCapture(idx))
                        .style(theme::text_button_style)
                        .padding(2),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center)
                .into()
            }).collect();
            Some(
                container(
                    column![
                        text(format!("{} photo(s) queued", self.camera_queue.len()))
                            .size(13)
                            .color(Color { a: 0.70, ..theme::text_primary() }),
                        scrollable(column(chips).spacing(4)).height(100),
                    ]
                    .spacing(8),
                )
                .style(theme::section_card_style)
                .padding(12)
                .width(Length::Fill)
                .into()
            )
        } else {
            None
        };

        // ── Analyze button ──
        let analyze_btn: Option<Element<'_, Message>> = if !self.camera_queue.is_empty() {
            // In AppendTo mode with no sessions loaded yet, disable the button
            let append_no_session = matches!(&self.camera_mode, CameraSessionMode::AppendTo(_))
                && self.camera_sessions.is_empty()
                && !self.camera_sessions_loading;

            let mut btn = button(
                row![
                    text(icons::ICON_MONITORING).font(icons::ICON_FONT).size(16),
                    text(format!(" Analyze {} Photo(s)", self.camera_queue.len())).size(15),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .style(theme::primary_button_style)
            .padding([14, 28]);
            if !append_no_session {
                btn = btn.on_press(Message::CameraStartAnalysis);
            }
            Some(btn.into())
        } else {
            None
        };

        // ── Assemble page ──
        let mut page_col = column![
            // Back button
            button(
                row![
                    text(icons::ICON_ARROW_BACK).font(icons::ICON_FONT).size(16),
                    text(" Back").size(14),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::NavigateTo(Page::Analysis))
            .style(theme::text_button_style)
            .padding([6, 0]),

            // Title
            text("Camera Capture").size(22),

            // Session mode selector
            text("Session mode").size(13).color(Color { a: 0.60, ..theme::text_primary() }),
            session_row,
        ]
        .spacing(16)
        .padding([20, 16]);

        if let Some(picker) = append_picker {
            page_col = page_col.push(picker);
        }
        if let Some(err) = error_row {
            page_col = page_col.push(err);
        }

        page_col = page_col.push(
            container(shutter_btn)
                .width(Length::Fill)
                .center_x(Length::Fill),
        );

        if let Some(strip) = queue_strip {
            page_col = page_col.push(strip);
        }
        if let Some(analyze) = analyze_btn {
            page_col = page_col.push(
                container(analyze)
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            );
        }

        container(scrollable(page_col))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(crate::theme::active_palette().bg_base)),
                ..iced::widget::container::transparent(theme)
            })
            .into()
    }

    /// Analysis page — the original 3-column layout.
    fn view_analysis(&self) -> Element<'_, Message> {
        // ── Left column: file input + file list ──
        let mut left_col = column![].spacing(12).padding(12).width(Length::FillPortion(3));

        // Buttons — file picker and (on desktop) directory picker
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
            // Multiple files: name + status, with color coding
            let file_list = column(self.jobs.iter().map(|job| {
                let icon = match &job.status {
                    JobStatus::Queued    => icons::ICON_HOURGLASS,
                    JobStatus::Processing => icons::ICON_SYNC,
                    JobStatus::Done      => icons::ICON_CHECK_CIRCLE,
                    JobStatus::Error(_)  => icons::ICON_ERROR,
                };
                let is_error = matches!(job.status, JobStatus::Error(_));
                let is_done  = job.status == JobStatus::Done;
                let icon_color = if is_error { theme::danger() }
                    else if is_done { theme::success() }
                    else { Color { a: 0.55, ..theme::text_primary() } };
                let name_color = if is_error { theme::danger() } else { theme::text_primary() };

                let is_selected = self.selected_job == Some(job.id);
                let row_content: Element<'_, Message> = row![
                    text(icon).font(icons::ICON_FONT).size(14).color(icon_color),
                    text(&job.filename).width(Length::Fill).color(name_color),
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
                match &job.status {
                    JobStatus::Error(err_msg) => {
                        // ── Error card (improvement 4) ──
                        // Parse the error string for a user-friendly suggestion.
                        let suggestion = if err_msg.contains("ROI") || err_msg.contains("contour") || err_msg.contains("mask") {
                            "No pineapple region was detected. Make sure the whole fruit is visible and well-lit."
                        } else if err_msg.contains("scale") || err_msg.contains("calibrat") || err_msg.contains("marker") {
                            "Scale calibration failed. Ensure the reference marker is fully visible and unobstructed."
                        } else if err_msg.contains("decode") || err_msg.contains("image") || err_msg.contains("format") {
                            "The image could not be decoded. Try re-capturing with a supported format (JPEG/PNG)."
                        } else if err_msg.contains("fruitlet") || err_msg.contains("count") {
                            "Fruitlet counting failed. The pineapple surface may be too blurry or overexposed."
                        } else {
                            "An unexpected error occurred. Re-capture the image and try again. If the problem persists, check the console log."
                        };

                        col = col.push(
                            container(
                                column![
                                    row![
                                        text(icons::ICON_ERROR).font(icons::ICON_FONT).size(22)
                                            .color(theme::danger()),
                                        text("Analysis Failed").size(16).color(theme::danger()),
                                    ]
                                    .spacing(10)
                                    .align_y(iced::Alignment::Center),
                                    text(&job.filename).size(13)
                                        .color(Color { a: 0.7, ..theme::text_primary() }),
                                    container(
                                        text(err_msg.as_str()).size(12)
                                            .color(theme::danger()),
                                    )
                                    .style(|_t: &iced::Theme| iced::widget::container::Style {
                                        background: Some(iced::Background::Color(
                                            Color { a: 0.08, ..theme::danger() }
                                        )),
                                        border: iced::Border {
                                            color: Color { a: 0.25, ..theme::danger() },
                                            width: 1.0,
                                            radius: 6.0.into(),
                                        },
                                        ..Default::default()
                                    })
                                    .padding([8, 10])
                                    .width(Length::Fill),
                                    row![
                                        text(icons::ICON_INFO).font(icons::ICON_FONT).size(14)
                                            .color(theme::warning()),
                                        text(suggestion).size(12)
                                            .color(Color { a: 0.85, ..theme::text_primary() }),
                                    ]
                                    .spacing(8)
                                    .align_y(iced::Alignment::Start),
                                ]
                                .spacing(12)
                                .padding(20),
                            )
                            .width(Length::Fill)
                        );
                    }
                    _ => {
                        let status_text = match &job.status {
                            JobStatus::Queued     => "Queued".to_string(),
                            JobStatus::Processing => "Processing…".to_string(),
                            JobStatus::Done       => "Done".to_string(),
                            JobStatus::Error(_)   => unreachable!(),
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
                }
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

        // ── Batch summary toast ──
        if let Some((ok, fail)) = self.batch_toast {
            let (icon, msg, color) = if fail == 0 {
                (icons::ICON_CHECK_CIRCLE,
                 format!("Analysis complete: {} image(s) succeeded", ok),
                 theme::success())
            } else if ok == 0 {
                (icons::ICON_ERROR,
                 format!("Analysis failed: {} image(s) could not be processed", fail),
                 theme::danger())
            } else {
                (icons::ICON_WARNING,
                 format!("Analysis complete: {} succeeded, {} failed", ok, fail),
                 theme::warning())
            };
            right_col = right_col.push(
                container(
                    row![
                        text(icon).font(icons::ICON_FONT).size(16).color(color),
                        text(msg).size(13).color(color),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .style(move |_t: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(
                        Color { a: 0.10, ..color }
                    )),
                    border: iced::Border {
                        color: Color { a: 0.30, ..color },
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                })
                .padding([10, 14])
                .width(Length::Fill),
            );
        }

        // Results table header
        let table_width = f32::max(600.0, self.window_width * 0.5 - 40.0);
        let header = row![
            tip_hdr("File", "Source image filename", 4),
            tip_hdr("H", MetricColumn::Height.description(), 1),
            tip_hdr("D", MetricColumn::Width.description(), 1),
            tip_hdr("V", MetricColumn::Volume.description(), 1),
            tip_hdr("a", MetricColumn::Aeq.description(), 1),
            tip_hdr("b", MetricColumn::Beq.description(), 1),
            tip_hdr("S", MetricColumn::SurfaceArea.description(), 1),
            tip_hdr("Nf", MetricColumn::NTotal.description(), 1),
        ]
        .spacing(4);
        let header_container = container(header).style(theme::table_header_style).padding([8, 6]).width(Length::Fixed(table_width));

        // Show ALL jobs (done = metrics, error = dashes, others = hidden until done)
        let finished_jobs: Vec<&Job> = self.jobs.iter()
            .filter(|j| matches!(j.status, JobStatus::Done | JobStatus::Error(_)))
            .collect();

        let table_content = if finished_jobs.is_empty() {
            let pending = self.jobs.iter().any(|j| {
                matches!(j.status, JobStatus::Queued | JobStatus::Processing)
            });
            let placeholder = if pending { "Analyzing…" } else { "No results yet" };
            column![
                header_container,
                container(text(placeholder).color(Color { a: 0.55, ..theme::text_primary() }))
                    .center(Length::Fixed(table_width))
                    .padding(20)
            ]
        } else {
            let rows = column(finished_jobs.iter().enumerate().map(|(idx, job)| {
                let row_bg = theme::table_row_bg(idx, false, false);
                let dash = || text("—").size(13).width(Length::FillPortion(1))
                    .color(Color { a: 0.55, ..theme::text_primary() });
                if let Some(m) = &job.metrics {
                    container(
                        row![
                            text(&job.filename).size(13).width(Length::FillPortion(4)),
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
                    .width(Length::Fixed(table_width))
                    .into()
                } else {
                    // Failed job: red filename + dash placeholders
                    container(
                        row![
                            text(&job.filename).size(13).width(Length::FillPortion(4))
                                .color(theme::danger()),
                            dash(), dash(), dash(), dash(), dash(), dash(), dash(),
                        ]
                        .spacing(4)
                        .align_y(iced::Alignment::Center),
                    )
                    .style(row_bg)
                    .padding([6, 6])
                    .width(Length::Fixed(table_width))
                    .into()
                }
            }))
            .spacing(1);
            
            column![
                header_container,
                scrollable(rows)
            ]
        };

        right_col = right_col.push(
            scrollable(table_content)
                .direction(iced::widget::scrollable::Direction::Horizontal(
                    iced::widget::scrollable::Scrollbar::new()
                ))
        );

        let has_done = self.jobs.iter().any(|j| j.status == JobStatus::Done);
        if has_done {
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
                                js_interop::is_mobile(),
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
    // Palette is already set via thread_local; pineapple_theme() reads from it.
    theme::pineapple_theme()
}

fn main() -> iced::Result {
    console_log::init().expect("Initialize logger");
    console_error_panic_hook::set_once();

    iced::application::timed(
        || App::new(),
        App::update,
        App::subscription,
        App::view,
    )
    .centered()
    .theme(pineapple_app_theme)
    .font(NOTO_SANS_SC_BYTES)
    .font(icons::ICON_FONT_BYTES)
    .run()
}
