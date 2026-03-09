//! Job abstraction for batch processing.
//!
//! Each `Job` represents a single image going through the measurement pipeline.

use crate::pipeline::{FruitletMetrics, Intermediate};

/// Current status of a processing job.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JobStatus {
    /// Waiting in queue for a worker thread.
    Queued,
    /// Currently being processed by a worker.
    Processing,
    /// Processing completed successfully.
    Done,
    /// Processing failed with an error message.
    Error(String),
}

/// A single image processing job.
#[derive(Clone, Debug)]
pub(crate) struct Job {
    /// Unique identifier (index in the jobs vector).
    pub id: usize,
    /// Original filename for display and CSV export.
    pub filename: String,
    /// Current processing status.
    pub status: JobStatus,
    /// Pipeline intermediate steps (only populated in debug mode when user
    /// has the Pipeline toggle ON and selects this job).
    pub intermediates: Vec<Intermediate>,
    /// Final computed metrics (populated when status == Done).
    pub metrics: Option<FruitletMetrics>,
}
