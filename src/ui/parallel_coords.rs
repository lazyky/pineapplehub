//! Parallel Coordinates Plot using plotters-iced2.
//!
//! Each vertical axis represents a metric column (H, D, V, a_eq, b_eq, S, Nf).
//! Each polyline represents one sample record. Axes are independently scaled.
//! Suspect (outlier) samples are drawn in red; normal samples in semi-transparent blue-grey.
//! Clicking a polyline emits `JumpToRecord(record_id)`.
//! Hovering shows a tooltip with sample name and metric values.

use plotters::prelude::*;
use plotters_iced2::{Chart, ChartWidget, DrawingBackend};

use iced::widget::canvas;
use iced::{Element, Length};

use crate::Message;
use crate::history::stats::MetricColumn;

/// Font name that matches the embedded Noto Sans SC font loaded via iced::application::font().
/// In WASM there are no system fonts; we must reference the embedded font by its family name
/// so that `canvas::Frame::fill_text()` can resolve it.
const CHART_FONT: &str = "Noto Sans SC";

/// Hit-test tolerance: max pixel distance from a line segment for a click/hover to register.
const HIT_TOLERANCE: f64 = 6.0;

/// Pre-extracted data for one sample in the parallel coords plot.
#[derive(Clone)]
struct SampleLine {
    record_id: String,
    filename: String,
    /// Normalized values [0..1] for each of the 7 axes. None if missing.
    values: [Option<f64>; 7],
    /// Raw (original) values for tooltip display.
    raw_values: [Option<f64>; 7],
    suspect: bool,
}

/// Chart interaction state (managed by iced widget tree).
#[derive(Default)]
pub struct ChartState {
    /// Index of the currently hovered sample (into `samples` vec).
    hovered_index: Option<usize>,
    /// Cursor position in widget-local coords (for tooltip placement).
    cursor_pos: Option<(f32, f32)>,
}

/// Data holder for the parallel coordinates chart.
/// Owns its data so it can be embedded in the iced widget tree.
pub(crate) struct ParallelCoordsChart {
    samples: Vec<SampleLine>,
    /// min/max per axis (for tick labels)
    mins: [f64; 7],
    maxs: [f64; 7],
    normal_count: usize,
    suspect_count: usize,
}

impl Default for ParallelCoordsChart {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            mins: [0.0; 7],
            maxs: [1.0; 7],
            normal_count: 0,
            suspect_count: 0,
        }
    }
}

impl ParallelCoordsChart {
    pub fn new(
        records: &[crate::history::model::AnalysisRecord],
        _outlier_cells: &std::collections::HashMap<String, std::collections::HashSet<MetricColumn>>,
    ) -> Self {
        let axes = MetricColumn::ALL;
        let n_axes = axes.len();

        // Compute min/max per axis
        let mut mins = [f64::INFINITY; 7];
        let mut maxs = [f64::NEG_INFINITY; 7];
        for record in records {
            for (i, col) in axes.iter().enumerate() {
                if let Some(val) = col.extract(record) {
                    if val < mins[i] { mins[i] = val; }
                    if val > maxs[i] { maxs[i] = val; }
                }
            }
        }
        // Prevent zero-range + add 5% padding
        for i in 0..n_axes {
            if (maxs[i] - mins[i]).abs() < 1e-12 {
                mins[i] -= 1.0;
                maxs[i] += 1.0;
            }
            let range = maxs[i] - mins[i];
            mins[i] -= range * 0.05;
            maxs[i] += range * 0.05;
        }

        // Pre-normalize all sample values, keep raw values for tooltip
        let mut normal_count = 0usize;
        let mut suspect_count = 0usize;
        let samples: Vec<SampleLine> = records
            .iter()
            .map(|record| {
                let mut values = [None; 7];
                let mut raw_values = [None; 7];
                for (i, col) in axes.iter().enumerate() {
                    if let Some(val) = col.extract(record) {
                        raw_values[i] = Some(val);
                        values[i] = Some((val - mins[i]) / (maxs[i] - mins[i]));
                    }
                }
                if record.suspect { suspect_count += 1; } else { normal_count += 1; }
                SampleLine {
                    record_id: record.id.clone(),
                    filename: record.filename.clone(),
                    values,
                    raw_values,
                    suspect: record.suspect,
                }
            })
            .collect();

        Self { samples, mins, maxs, normal_count, suspect_count }
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn view(&self) -> Element<'_, Message> {
        ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fixed(320.0))
            // WORKAROUND: iced canvas::Text silently fails to render in WASM/WebGL
            // when using the default Shaping::Basic. Switching to Advanced fixes it.
            // Upstream issue: https://github.com/iced-rs/iced/issues/3199
            .text_shaping(iced::widget::text::Shaping::Advanced)
            .into()
    }

    /// Compute layout constants from widget bounds (must match draw_chart).
    fn layout(&self, bounds: iced::Rectangle) -> ChartLayout {
        let n_axes = MetricColumn::ALL.len();
        let w = bounds.width as f64;
        let h = bounds.height as f64;
        let margin_left = 50.0;
        let margin_right = 30.0;
        let margin_top = 30.0;
        let margin_bottom = 30.0;
        let plot_w = w - margin_left - margin_right;
        let plot_h = h - margin_top - margin_bottom;
        let axis_spacing = if n_axes > 1 { plot_w / (n_axes as f64 - 1.0) } else { plot_w };
        ChartLayout { margin_left, margin_top, plot_w, plot_h, axis_spacing }
    }

    /// Hit-test: find the closest sample to a point within HIT_TOLERANCE.
    /// Returns the sample index if found. Suspects are tested first (drawn on top).
    fn hit_test(&self, click_x: f64, click_y: f64, layout: &ChartLayout) -> Option<usize> {
        let to_pixel = |axis_i: usize, norm: f64| -> (f64, f64) {
            let x = layout.margin_left + axis_i as f64 * layout.axis_spacing;
            let y = layout.margin_top + layout.plot_h * (1.0 - norm);
            (x, y)
        };

        let mut best_dist = HIT_TOLERANCE;
        let mut best_idx: Option<usize> = None;

        // Test all samples; suspects drawn on top so they should win ties
        for (idx, sample) in self.samples.iter().enumerate().rev() {
            let pts: Vec<(f64, f64)> = sample.values.iter().enumerate()
                .filter_map(|(i, v)| v.map(|n| to_pixel(i, n)))
                .collect();
            let d = min_distance_to_polyline(click_x, click_y, &pts);
            if d < best_dist {
                best_dist = d;
                best_idx = Some(idx);
            }
        }
        best_idx
    }
}

struct ChartLayout {
    margin_left: f64,
    margin_top: f64,
    plot_w: f64,
    plot_h: f64,
    axis_spacing: f64,
}

/// Minimum distance from a point to a polyline (sequence of segments).
fn min_distance_to_polyline(px: f64, py: f64, pts: &[(f64, f64)]) -> f64 {
    let mut min_d = f64::INFINITY;
    for w in pts.windows(2) {
        let d = point_to_segment_dist(px, py, w[0].0, w[0].1, w[1].0, w[1].1);
        if d < min_d { min_d = d; }
    }
    min_d
}

/// Distance from point (px,py) to line segment (x1,y1)-(x2,y2).
fn point_to_segment_dist(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-12 {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }
    let t = ((px - x1) * dx + (py - y1) * dy) / len_sq;
    let t = t.clamp(0.0, 1.0);
    let proj_x = x1 + t * dx;
    let proj_y = y1 + t * dy;
    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

impl Chart<Message> for ParallelCoordsChart {
    type State = ChartState;

    fn build_chart<DB: DrawingBackend>(
        &self,
        _state: &Self::State,
        _builder: plotters_iced2::ChartBuilder<DB>,
    ) {
        // Not used — we override draw_chart.
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> (iced::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let layout = self.layout(bounds);
                    if layout.plot_w > 0.0 && layout.plot_h > 0.0 {
                        if let Some(idx) = self.hit_test(pos.x as f64, pos.y as f64, &layout) {
                            return (iced::event::Status::Captured, Some(Message::JumpToRecord(
                                self.samples[idx].record_id.clone(),
                            )));
                        }
                    }
                }
            }
            canvas::Event::Mouse(iced::mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let layout = self.layout(bounds);
                    let new_hover = if layout.plot_w > 0.0 && layout.plot_h > 0.0 {
                        self.hit_test(pos.x as f64, pos.y as f64, &layout)
                    } else {
                        None
                    };
                    if state.hovered_index != new_hover {
                        state.hovered_index = new_hover;
                        state.cursor_pos = Some((pos.x, pos.y));
                        // Return a Noop message to force widget redraw;
                        // plotters-iced2 only redraws when a message is published.
                        return (iced::event::Status::Captured, Some(Message::Noop));
                    }
                    state.cursor_pos = Some((pos.x, pos.y));
                } else if state.hovered_index.is_some() {
                    state.hovered_index = None;
                    state.cursor_pos = None;
                    return (iced::event::Status::Captured, Some(Message::Noop));
                }
            }
            _ => {}
        }
        (iced::event::Status::Ignored, None)
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        if state.hovered_index.is_some() {
            iced::mouse::Interaction::Pointer
        } else {
            iced::mouse::Interaction::Idle
        }
    }

    fn draw_chart<DB: DrawingBackend>(
        &self,
        state: &Self::State,
        root: plotters::drawing::DrawingArea<DB, plotters::coord::Shift>,
    ) {
        let axes = MetricColumn::ALL;
        let n_axes = axes.len();
        if self.samples.is_empty() || n_axes == 0 {
            return;
        }

        // Dark background
        root.fill(&crate::theme::chart::BG).ok();

        let (w, h) = root.dim_in_pixel();
        let w = w as f64;
        let h = h as f64;

        let margin_left = 50.0;
        let margin_right = 30.0;
        let margin_top = 30.0;
        let margin_bottom = 30.0;

        let plot_w = w - margin_left - margin_right;
        let plot_h = h - margin_top - margin_bottom;

        if plot_w <= 0.0 || plot_h <= 0.0 {
            return;
        }

        let axis_spacing = plot_w / (n_axes as f64 - 1.0).max(1.0);

        // Helper: pixel coords from (axis_index, normalized 0..1)
        let to_pixel = |axis_i: usize, norm: f64| -> (i32, i32) {
            let x = margin_left + axis_i as f64 * axis_spacing;
            let y = margin_top + plot_h * (1.0 - norm);
            (x as i32, y as i32)
        };

        // Use the embedded font for all text
        let axis_color = crate::theme::chart::AXIS;
        let label_color = crate::theme::chart::LABEL;
        let label_style = (CHART_FONT, 11).into_font().color(&label_color);
        let tick_style = (CHART_FONT, 9).into_font().color(&crate::theme::chart::TICK);

        // Draw axes + labels + ticks
        for (i, col) in axes.iter().enumerate() {
            let x = (margin_left + i as f64 * axis_spacing) as i32;
            let y_top = margin_top as i32;
            let y_bot = (margin_top + plot_h) as i32;

            // Axis line
            root.draw(&PathElement::new(
                vec![(x, y_top), (x, y_bot)],
                axis_color.stroke_width(1),
            )).ok();

            // Axis label (top)
            root.draw(&plotters::element::Text::new(
                col.label().to_string(),
                (x, y_top - 10),
                label_style.clone(),
            )).ok();

            // Format tick value
            let fmt = |v: f64| -> String {
                if v.abs() >= 10_000.0 { format!("{:.0}", v) }
                else if v.abs() >= 100.0 { format!("{:.1}", v) }
                else { format!("{:.2}", v) }
            };

            // Max (top)
            root.draw(&plotters::element::Text::new(
                fmt(self.maxs[i]),
                (x + 4, y_top + 2),
                tick_style.clone(),
            )).ok();

            // Min (bottom)
            root.draw(&plotters::element::Text::new(
                fmt(self.mins[i]),
                (x + 4, y_bot - 12),
                tick_style.clone(),
            )).ok();
        }

        // Draw polylines: normal first (background), then suspects (foreground)
        let normal_color = crate::theme::chart::NORMAL_LINE;
        let suspect_color = crate::theme::chart::SUSPECT_LINE;
        let highlight_color = crate::theme::chart::HIGHLIGHT_LINE;

        for (idx, sample) in self.samples.iter().enumerate() {
            if sample.suspect { continue; }
            // Dim non-hovered lines when something is hovered
            let is_hovered = state.hovered_index == Some(idx);
            if is_hovered { continue; } // draw hovered line last
            let pts: Vec<(i32, i32)> = sample.values.iter().enumerate()
                .filter_map(|(i, v)| v.map(|n| to_pixel(i, n)))
                .collect();
            if pts.len() >= 2 {
                root.draw(&PathElement::new(pts, normal_color.stroke_width(1))).ok();
            }
        }

        for (idx, sample) in self.samples.iter().enumerate() {
            if !sample.suspect { continue; }
            let is_hovered = state.hovered_index == Some(idx);
            if is_hovered { continue; }
            let pts: Vec<(i32, i32)> = sample.values.iter().enumerate()
                .filter_map(|(i, v)| v.map(|n| to_pixel(i, n)))
                .collect();
            if pts.len() >= 2 {
                root.draw(&PathElement::new(pts, suspect_color.stroke_width(2))).ok();
            }
        }

        // Draw hovered line on top with highlight color
        if let Some(hovered_idx) = state.hovered_index {
            if let Some(sample) = self.samples.get(hovered_idx) {
                let pts: Vec<(i32, i32)> = sample.values.iter().enumerate()
                    .filter_map(|(i, v)| v.map(|n| to_pixel(i, n)))
                    .collect();
                if pts.len() >= 2 {
                    root.draw(&PathElement::new(pts, highlight_color.stroke_width(3))).ok();
                }
            }
        }

        // Legend
        let legend_y = (margin_top + plot_h + 15.0) as i32;
        let lx = margin_left as i32;

        root.draw(&Rectangle::new(
            [(lx, legend_y), (lx + 12, legend_y + 8)],
            crate::theme::chart::LEGEND_NORMAL.filled(),
        )).ok();
        root.draw(&plotters::element::Text::new(
            format!("Normal ({})", self.normal_count),
            (lx + 16, legend_y - 1),
            tick_style.clone(),
        )).ok();

        let sx = lx + 110;
        root.draw(&Rectangle::new(
            [(sx, legend_y), (sx + 12, legend_y + 8)],
            crate::theme::chart::LEGEND_SUSPECT.filled(),
        )).ok();
        root.draw(&plotters::element::Text::new(
            format!("Suspect ({})", self.suspect_count),
            (sx + 16, legend_y - 1),
            tick_style.clone(),
        )).ok();

        // ── Tooltip ──
        if let (Some(hovered_idx), Some((cx, cy))) = (state.hovered_index, state.cursor_pos) {
            if let Some(sample) = self.samples.get(hovered_idx) {
                let tooltip_style = (CHART_FONT, 10).into_font().color(&WHITE);
                let tooltip_w = 160;
                let tooltip_h = 14 * (n_axes + 1) as i32 + 8; // filename + 7 metrics + padding
                // Position: offset from cursor, clamped to chart area
                let tx = ((cx as i32) + 12).min(w as i32 - tooltip_w - 4);
                let ty = ((cy as i32) - tooltip_h / 2).clamp(2, h as i32 - tooltip_h - 2);

                // Semi-transparent background
                root.draw(&Rectangle::new(
                    [(tx, ty), (tx + tooltip_w, ty + tooltip_h)],
                    crate::theme::chart::TOOLTIP_BG.filled(),
                )).ok();
                // Border
                root.draw(&Rectangle::new(
                    [(tx, ty), (tx + tooltip_w, ty + tooltip_h)],
                    crate::theme::chart::TOOLTIP_BORDER.stroke_width(1),
                )).ok();

                // Filename (header)
                let status = if sample.suspect { " ⚠" } else { "" };
                root.draw(&plotters::element::Text::new(
                    format!("{}{}", sample.filename, status),
                    (tx + 4, ty + 2),
                    (CHART_FONT, 10).into_font().color(&crate::theme::chart::TOOLTIP_HEADER),
                )).ok();

                // Metric values
                for (i, col) in axes.iter().enumerate() {
                    let val_str = match sample.raw_values[i] {
                        Some(v) if v.abs() >= 10_000.0 => format!("{:.0}", v),
                        Some(v) if v.abs() >= 100.0 => format!("{:.1}", v),
                        Some(v) => format!("{:.2}", v),
                        None => "-".to_string(),
                    };
                    root.draw(&plotters::element::Text::new(
                        format!("{}: {}", col.label(), val_str),
                        (tx + 4, ty + 16 + i as i32 * 14),
                        tooltip_style.clone(),
                    )).ok();
                }
            }
        }
    }
}
