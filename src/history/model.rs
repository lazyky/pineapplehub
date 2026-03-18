//! Data model for persistent analysis history.
//!
//! These types are serialized to/from IndexedDB via `serde` + `serde-wasm-bindgen`.

use serde::{Deserialize, Serialize};

use crate::pipeline::FruitletMetrics;

/// Snapshot of program-computed values before any manual edits.
///
/// Stored inside [`StoredMetrics`] so we can reset back to the original
/// machine-computed output even after manual overrides.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OriginalMetrics {
    pub major_length: f32,
    pub minor_length: f32,
    pub a_eq: Option<f32>,
    pub b_eq: Option<f32>,
}

/// Serializable metrics that support manual editing.
///
/// Separated from [`FruitletMetrics`] so we can add the `manually_edited` flag
/// and persist only the fields that make sense for history storage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StoredMetrics {
    /// Fruit height in mm (editable).
    pub major_length: f32,
    /// Fruit width in mm (editable).
    pub minor_length: f32,
    /// Volume in mm³ (computed, not editable).
    pub volume: f32,
    /// Equatorial fruitlet eye long axis in mm (editable).
    pub a_eq: Option<f32>,
    /// Equatorial fruitlet eye short axis in mm (editable).
    pub b_eq: Option<f32>,
    /// Whole-fruit surface area in mm² (computed).
    pub surface_area: Option<f32>,
    /// Estimated total fruitlet eye count (computed).
    pub n_total: Option<u32>,
    /// Whether the user has manually edited any field.
    pub manually_edited: bool,
    /// Snapshot of the original program-computed values (set on first manual edit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<OriginalMetrics>,
}

impl From<&FruitletMetrics> for StoredMetrics {
    fn from(m: &FruitletMetrics) -> Self {
        Self {
            major_length: m.major_length,
            minor_length: m.minor_length,
            volume: m.volume,
            a_eq: m.a_eq,
            b_eq: m.b_eq,
            surface_area: m.surface_area,
            n_total: m.n_total,
            manually_edited: false,
            original: None,
        }
    }
}

/// A single analysis record persisted in IndexedDB.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AnalysisRecord {
    /// Unique record identifier (simple UUID-like string).
    pub id: String,
    /// Batch session this record belongs to.
    pub session_id: String,
    /// Timestamp when the analysis was completed (ms since epoch).
    pub timestamp: f64,
    /// Original filename.
    pub filename: String,
    /// Computed (or manually edited) metrics.
    pub metrics: StoredMetrics,
    /// Marked as potentially inaccurate by the user.
    pub suspect: bool,
    /// Free-text note attached by the user.
    pub note: String,
}

/// Session-level metadata persisted in IndexedDB.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SessionMeta {
    /// Unique session identifier.
    pub session_id: String,
    /// Timestamp of the batch run.
    pub timestamp: f64,
    /// Total number of files in this batch.
    pub total_count: u32,
    /// Number of files that produced valid metrics.
    pub success_count: u32,
    /// Number of files that failed processing.
    pub failed_count: u32,
    /// Whether this session is starred (protected from auto-cleanup).
    pub starred: bool,
    /// User-assigned display name (None = show timestamp).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Aggregated session summary for the UI (not persisted directly;
/// computed from [`SessionMeta`] + record queries).
#[derive(Clone, Debug)]
pub(crate) struct SessionSummary {
    pub session_id: String,
    pub timestamp: f64,
    pub total_count: u32,
    pub failed_count: u32,
    pub suspect_count: u32,
    pub starred: bool,
    pub name: Option<String>,
}

impl SessionSummary {
    /// Build a summary from persisted metadata + a suspect count from records.
    pub fn from_meta(meta: &SessionMeta, suspect_count: u32) -> Self {
        Self {
            session_id: meta.session_id.clone(),
            timestamp: meta.timestamp,
            total_count: meta.total_count,
            failed_count: meta.failed_count,
            suspect_count,
            starred: meta.starred,
            name: meta.name.clone(),
        }
    }
}
