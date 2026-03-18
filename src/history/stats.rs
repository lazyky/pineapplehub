//! Statistical analysis for analysis records.
//!
//! Provides column-level descriptive statistics and IQR-based outlier detection.

use std::collections::{HashMap, HashSet};

use super::model::AnalysisRecord;

/// Descriptive statistics for a single numeric column.
#[derive(Clone, Debug)]
pub(crate) struct ColumnStats {
    pub mean: f64,
    pub median: f64,
    pub sd: f64,
    pub min: f64,
    pub max: f64,
    pub q1: f64,
    pub q3: f64,
    pub n: usize,
}

/// Names for the columns that can be analysed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MetricColumn {
    Height,
    Width,
    Volume,
    Aeq,
    Beq,
    SurfaceArea,
    NTotal,
}

impl MetricColumn {
    pub const ALL: [Self; 7] = [
        Self::Height,
        Self::Width,
        Self::Volume,
        Self::Aeq,
        Self::Beq,
        Self::SurfaceArea,
        Self::NTotal,
    ];

    /// Publication-quality display label with unit.
    pub fn label(self) -> &'static str {
        match self {
            Self::Height => "H (mm)",
            Self::Width => "D (mm)",
            Self::Volume => "V (mm³)",
            Self::Aeq => "a (mm)",
            Self::Beq => "b (mm)",
            Self::SurfaceArea => "S (mm²)",
            Self::NTotal => "Nf",
        }
    }



    /// Tooltip description explaining what this metric is.
    pub fn description(self) -> &'static str {
        match self {
            Self::Height => "Major axis length (height)",
            Self::Width => "Equatorial diameter (width)",
            Self::Volume => "Estimated volume",
            Self::Aeq => "Equiv. ellipsoid semi-axis a",
            Self::Beq => "Equiv. ellipsoid semi-axis b",
            Self::SurfaceArea => "Surface area",
            Self::NTotal => "Fruitlet count",
        }
    }

    /// Extract the value for this column from a record, if present.
    pub fn extract(self, r: &AnalysisRecord) -> Option<f64> {
        let m = &r.metrics;
        match self {
            Self::Height => Some(f64::from(m.major_length)),
            Self::Width => Some(f64::from(m.minor_length)),
            Self::Volume => Some(f64::from(m.volume)),
            Self::Aeq => m.a_eq.map(f64::from),
            Self::Beq => m.b_eq.map(f64::from),
            Self::SurfaceArea => m.surface_area.map(f64::from),
            Self::NTotal => m.n_total.map(|v| f64::from(v)),
        }
    }
}

/// Compute descriptive statistics for a slice of values.
pub(crate) fn compute_stats(values: &[f64]) -> Option<ColumnStats> {
    let n = values.len();
    if n < 2 {
        return None;
    }
    let sum: f64 = values.iter().sum();
    let mean = sum / n as f64;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
    let sd = var.sqrt();
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = sorted[0];
    let max = sorted[n - 1];
    let q1 = percentile(&sorted, 25.0);
    let median = percentile(&sorted, 50.0);
    let q3 = percentile(&sorted, 75.0);
    Some(ColumnStats {
        mean, median, sd, min, max, q1, q3, n,
    })
}

/// Linear interpolation percentile (0–100 scale).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let rank = (p / 100.0) * (n as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi.min(n - 1)] * frac
}




/// Compute stats from a slice of record references (for per-session grouping).
pub(crate) fn compute_all_stats_from_refs(records: &[&AnalysisRecord]) -> HashMap<MetricColumn, ColumnStats> {
    let mut result = HashMap::new();
    for col in MetricColumn::ALL {
        let values: Vec<f64> = records.iter().filter_map(|r| col.extract(r)).collect();
        if let Some(stats) = compute_stats(&values) {
            result.insert(col, stats);
        }
    }
    result
}

/// Detect outliers from a slice of record references (for per-session grouping).
pub(crate) fn detect_outliers_from_refs(
    records: &[&AnalysisRecord],
    stats: &HashMap<MetricColumn, ColumnStats>,
) -> HashMap<String, HashSet<MetricColumn>> {
    let mut outliers: HashMap<String, HashSet<MetricColumn>> = HashMap::new();
    for record in records {
        for col in MetricColumn::ALL {
            if let (Some(val), Some(st)) = (col.extract(record), stats.get(&col)) {
                let iqr = st.q3 - st.q1;
                let lower = st.q1 - 1.5 * iqr;
                let upper = st.q3 + 1.5 * iqr;
                if val < lower || val > upper {
                    outliers
                        .entry(record.id.clone())
                        .or_default()
                        .insert(col);
                }
            }
        }
    }
    outliers
}
