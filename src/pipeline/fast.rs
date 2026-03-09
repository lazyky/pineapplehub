//! Fast (headless) pipeline for Web Worker parallel processing.
//!
//! This module contains a **pure computation** path that produces
//! `FruitletMetrics` without any iced UI types (`Preview`, `Instant`,
//! `Handle`), browser APIs, or `log` macros.  Every function here is
//! `Send + Sync` and safe to call from rayon Web Worker threads.

use image::{DynamicImage, ImageBuffer, ImageReader, Luma, imageops};
use imageproc::{
    contours::find_contours_with_threshold,
    contrast::adaptive_threshold,
    distance_transform::Norm,
    filter::{gaussian_blur_f32, median_filter},
    geometry::{arc_length, min_area_rect},
    morphology::{dilate, erode},
    point::Point,
    region_labelling::{Connectivity, connected_components},
};
use std::collections::HashMap;
use std::io::Cursor;

use crate::correction::unwrap;
use crate::error::Error;
use crate::js_interop::FileEntry;
use crate::pipeline::scale_calibration::perform_scale_calibration;
use crate::pipeline::{COIN_RADIUS_MM, FruitletMetrics};

// ── Helpers (duplicated from unwrap_metrics.rs to avoid iced deps) ──

fn get_tight_bounds(
    img: &ImageBuffer<Luma<u8>, Vec<u8>>,
) -> Option<([Point<i32>; 4], Vec<Point<i32>>)> {
    let otsu = imageproc::contrast::otsu_level(img);
    let binary =
        imageproc::contrast::threshold(img, otsu, imageproc::contrast::ThresholdType::Binary);
    let w = binary.width();
    let h = binary.height();
    let small_w = (w as f32 * 0.25).max(1.0) as u32;
    let small_h = (h as f32 * 0.25).max(1.0) as u32;
    let small = imageops::resize(&binary, small_w, small_h, imageops::FilterType::Nearest);
    let d1 = dilate(&small, Norm::LInf, 2);
    let closed = erode(&d1, Norm::LInf, 2);
    let e1 = erode(&closed, Norm::LInf, 3);
    let opened = dilate(&e1, Norm::LInf, 3);
    let restored = imageops::resize(&opened, w, h, imageops::FilterType::Nearest);
    let contours = find_contours_with_threshold(&restored, 127);
    contours
        .into_iter()
        .max_by(|a, b| arc_length(&a.points, true).total_cmp(&arc_length(&b.points, true)))
        .map(|c| {
            let corners = min_area_rect(&c.points);
            (corners, c.points)
        })
}

fn compute_rect_metrics(box_points: &[Point<i32>; 4]) -> (f32, f32, f32, f32, f32) {
    let dx1 = (box_points[0].x - box_points[1].x) as f32;
    let dy1 = (box_points[0].y - box_points[1].y) as f32;
    let l1 = (dx1 * dx1 + dy1 * dy1).sqrt();
    let dx2 = (box_points[1].x - box_points[2].x) as f32;
    let dy2 = (box_points[1].y - box_points[2].y) as f32;
    let l2 = (dx2 * dx2 + dy2 * dy2).sqrt();
    let (major, minor, dx_major, dy_major) = if l1 > l2 {
        (l1, l2, dx1, dy1)
    } else {
        (l2, l1, dx2, dy2)
    };
    let angle = dy_major.atan2(dx_major);
    let cx = (box_points[0].x + box_points[1].x + box_points[2].x + box_points[3].x) as f32 / 4.0;
    let cy = (box_points[0].y + box_points[1].y + box_points[2].y + box_points[3].y) as f32 / 4.0;
    (major, minor, angle, cx, cy)
}

fn integrate_volume(contour: &[Point<i32>], cx: f32, cy: f32, angle: f32, t_scale: f32) -> f32 {
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let mut tr_points: Vec<(f32, f32)> = Vec::with_capacity(contour.len());
    for pt in contour {
        let lx = pt.x as f32 - cx;
        let ly = pt.y as f32 - cy;
        let t = (lx * cos_a + ly * sin_a) * t_scale;
        let r = -lx * sin_a + ly * cos_a;
        if r >= 0.0 {
            tr_points.push((t, r));
        }
    }
    tr_points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut vol: f64 = 0.0;
    for w in tr_points.windows(2) {
        let dt = (w[1].0 - w[0].0) as f64;
        let r0 = w[0].1 as f64;
        let r1 = w[1].1 as f64;
        vol += std::f64::consts::PI * (r0 * r0 + r1 * r1) / 2.0 * dt;
    }
    vol as f32
}

fn integrate_surface_area(
    contour: &[Point<i32>],
    cx: f32,
    cy: f32,
    angle: f32,
    t_scale: f32,
) -> f32 {
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let mut tr_points: Vec<(f32, f32)> = Vec::with_capacity(contour.len());
    for pt in contour {
        let lx = pt.x as f32 - cx;
        let ly = pt.y as f32 - cy;
        let t = (lx * cos_a + ly * sin_a) * t_scale;
        let r = -lx * sin_a + ly * cos_a;
        if r >= 0.0 {
            tr_points.push((t, r));
        }
    }
    if tr_points.is_empty() {
        return 0.0;
    }
    let t_min = tr_points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let t_max = tr_points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let t_range = t_max - t_min;
    if t_range <= 0.0 {
        return 0.0;
    }
    let bin_width = t_scale.max(1.0);
    let n_bins = ((t_range / bin_width).ceil() as usize).max(2);
    let bin_width = t_range / n_bins as f32;
    let mut bins: Vec<f32> = vec![-1.0; n_bins];
    for &(t, r) in &tr_points {
        let idx = ((t - t_min) / bin_width).floor() as usize;
        let idx = idx.min(n_bins - 1);
        if r > bins[idx] {
            bins[idx] = r;
        }
    }
    let mut last_valid: Option<(usize, f32)> = None;
    for i in 0..n_bins {
        if bins[i] >= 0.0 {
            if let Some((prev_i, prev_r)) = last_valid {
                let gap = i - prev_i;
                if gap > 1 {
                    for k in (prev_i + 1)..i {
                        let frac = (k - prev_i) as f32 / gap as f32;
                        bins[k] = prev_r + frac * (bins[i] - prev_r);
                    }
                }
            }
            last_valid = Some((i, bins[i]));
        }
    }
    if let Some((first_valid_i, first_r)) = bins.iter().position(|&r| r >= 0.0).map(|i| (i, bins[i])) {
        for b in bins.iter_mut().take(first_valid_i) {
            *b = first_r;
        }
    }
    if let Some(last_valid_i) = bins.iter().rposition(|&r| r >= 0.0) {
        let last_r = bins[last_valid_i];
        for b in bins.iter_mut().skip(last_valid_i + 1) {
            *b = last_r;
        }
    }
    let mut area: f64 = 0.0;
    for i in 0..(n_bins - 1) {
        let r0 = bins[i].max(0.0) as f64;
        let r1 = bins[i + 1].max(0.0) as f64;
        let dt = bin_width as f64;
        let dr = r1 - r0;
        let r_avg = (r0 + r1) / 2.0;
        let ds = (dt * dt + dr * dr).sqrt();
        area += 2.0 * std::f64::consts::PI * r_avg * ds;
    }
    area as f32
}

// ── Fruitlet counting helpers ──

struct Region {
    area: u32,
    points: Vec<Point<i32>>,
    centroid_x: f32,
    centroid_y: f32,
    bbox_min_y: i32,
    bbox_max_y: i32,
}

fn collect_regions(labels: &ImageBuffer<Luma<u32>, Vec<u32>>) -> HashMap<u32, Region> {
    let mut regions: HashMap<u32, (u32, Vec<Point<i32>>, f64, f64, i32, i32)> = HashMap::new();
    for (x, y, px) in labels.enumerate_pixels() {
        let label = px.0[0];
        if label == 0 {
            continue;
        }
        let entry = regions.entry(label).or_insert((0, Vec::new(), 0.0, 0.0, y as i32, y as i32));
        entry.0 += 1;
        entry.1.push(Point::new(x as i32, y as i32));
        entry.2 += x as f64;
        entry.3 += y as f64;
        entry.4 = entry.4.min(y as i32);
        entry.5 = entry.5.max(y as i32);
    }
    regions
        .into_iter()
        .map(|(label, (area, points, sx, sy, min_y, max_y))| {
            (label, Region {
                area,
                points,
                centroid_x: sx as f32 / area as f32,
                centroid_y: sy as f32 / area as f32,
                bbox_min_y: min_y,
                bbox_max_y: max_y,
            })
        })
        .collect()
}

fn compute_fruitlet_rect(box_points: &[Point<i32>; 4]) -> (f32, f32, f32) {
    let dx1 = (box_points[0].x - box_points[1].x) as f32;
    let dy1 = (box_points[0].y - box_points[1].y) as f32;
    let l1 = (dx1 * dx1 + dy1 * dy1).sqrt();
    let dx2 = (box_points[1].x - box_points[2].x) as f32;
    let dy2 = (box_points[1].y - box_points[2].y) as f32;
    let l2 = (dx2 * dx2 + dy2 * dy2).sqrt();
    let (major, minor, dx_major, dy_major) = if l1 > l2 {
        (l1, l2, dx1, dy1)
    } else {
        (l2, l1, dx2, dy2)
    };
    let angle = dy_major.atan2(dx_major);
    (major, minor, angle)
}

/// Pre-decoded image data ready for worker processing.
/// Created on the main thread (where image decoders work safely),
/// then sent to rayon Web Workers for CPU-intensive computation.
pub(crate) struct PreparedImage {
    /// High-res grayscale (for crop/rotate/unwrap/counting)
    pub gray_hr: ImageBuffer<Luma<u8>, Vec<u8>>,
    /// Downsampled to 1024px (for calibration/ROI detection)
    pub resized: DynamicImage,
    /// Scale factor: gray_hr.width / resized.width
    pub scale: f32,
}

/// Phase 1 — runs on **main thread** (sequentially).
/// Decodes the image, extracts gray_hr + resized, drops the original.
/// Peak memory: one full-res decoded image at a time (~48MB).
pub(crate) fn prepare_image(entry: &FileEntry) -> Result<PreparedImage, Error> {
    let original = ImageReader::new(Cursor::new(&entry.data))
        .with_guessed_format()
        .map_err(|e| Error::General(format!("Format detect: {e}")))?
        .decode()?;
    let gray_hr = original.to_luma8();
    let resized = original.resize(1024, 1024, imageops::Lanczos3);
    let scale = gray_hr.width() as f32 / resized.width() as f32;
    // `original` is dropped here — frees ~48MB (RGBA full-res)
    Ok(PreparedImage { gray_hr, resized, scale })
}

/// Phase 2 — runs on **rayon Web Workers** (in parallel).
/// Pure computation: smoothing, calibration, ROI, crop, rotate, unwrap, counting.
/// No image decoding, no browser APIs needed.
pub(crate) fn process_prepared(prep: &PreparedImage) -> Result<FruitletMetrics, Error> {
    console_error_panic_hook::set_once();
    use imageproc::geometric_transformations::{Interpolation, rotate_about_center};

    let image = &prep.resized;
    let gray_hr = &prep.gray_hr;
    let scale = prep.scale;

    // Helper to log from worker threads (bypasses Rust log infra)
    macro_rules! wlog {
        ($($arg:tt)*) => {
            web_sys::console::log_1(&format!($($arg)*).into());
        }
    }

    wlog!("[fast] step 2: smoothing");
    let smoothed = gaussian_blur_f32(&median_filter(&image.to_rgba8(), 1, 1), 1.0);

    wlog!("[fast] step 3: scale calibration");
    let smoothed_luma = DynamicImage::ImageRgba8(smoothed).to_luma8();
    let (_vis_img, px_per_mm, _binary, _fused, contours) =
        perform_scale_calibration(&smoothed_luma);

    let px_per_mm_val = px_per_mm.ok_or(Error::General("Scale calibration failed".into()))?;
    wlog!("[fast] step 3 done: px_per_mm={}", px_per_mm_val);

    wlog!("[fast] step 4: ROI extraction");
    let roi_rect = super::roi_extraction::extract_best_roi(&smoothed_luma, px_per_mm_val, contours)?
        .ok_or(Error::General("No ROI found".into()))?;

    let hr_px_per_mm = px_per_mm_val * scale;
    let mm_per_px = 1.0 / hr_px_per_mm;

    let hr_cx = roi_rect.cx * scale;
    let hr_cy = roi_rect.cy * scale;
    let hr_w = (roi_rect.width * scale).round() as u32;
    let hr_h = (roi_rect.height * scale).round() as u32;

    wlog!("[fast] step 5: crop+rotate HR image ({}x{}, scale={})", hr_w, hr_h, scale);

    let diag = ((hr_w as f32).powi(2) + (hr_h as f32).powi(2)).sqrt();
    let safe_x = (hr_cx - diag / 2.0).round() as i32;
    let safe_y = (hr_cy - diag / 2.0).round() as i32;
    let safe_w = diag.ceil() as u32;
    let safe_h = diag.ceil() as u32;

    let mut padded_crop: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(safe_w, safe_h);
    for y in 0..safe_h {
        for x in 0..safe_w {
            let src_x = safe_x + x as i32;
            let src_y = safe_y + y as i32;
            if src_x >= 0 && src_x < gray_hr.width() as i32
                && src_y >= 0 && src_y < gray_hr.height() as i32
            {
                padded_crop.put_pixel(x, y, *gray_hr.get_pixel(src_x as u32, src_y as u32));
            }
        }
    }

    wlog!("[fast] step 5b: rotate_about_center");
    let rotated_panel = rotate_about_center(
        &padded_crop,
        roi_rect.angle_rad,
        Interpolation::Bilinear,
        Luma([0]),
    );
    let rot_cx = rotated_panel.width() as f32 / 2.0;
    let rot_cy = rotated_panel.height() as f32 / 2.0;
    let crop_x = (rot_cx - hr_w as f32 / 2.0).round().max(0.0) as u32;
    let crop_y = (rot_cy - hr_h as f32 / 2.0).round().max(0.0) as u32;

    let warped = imageops::crop_imm(&rotated_panel, crop_x, crop_y, hr_w, hr_h).to_image();

    wlog!("[fast] step 6: dual-axis unwrap");
    let vert_unwrapped = unwrap(&warped);
    let horiz_rotated = imageops::rotate90(&warped);
    let horiz_unwrapped = unwrap(&horiz_rotated);
    wlog!("[fast] step 6 done");

    // Vert unwrap: get major length (authentic height)
    let mut metrics = FruitletMetrics {
        major_length: 0.0,
        minor_length: 0.0,
        volume: 0.0,
        a_eq: None,
        b_eq: None,
        alpha: None,
        surface_area: None,
        n_total: None,
    };

    let mut vert_major_px: Option<f32> = None;
    if let Some((box_points, _contour)) = get_tight_bounds(&vert_unwrapped) {
        let (major, minor, _angle, _cx, _cy) = compute_rect_metrics(&box_points);
        vert_major_px = Some(major);
        metrics.major_length = major * mm_per_px;
        metrics.minor_length = minor * mm_per_px; // temp, replaced by horiz
    }

    // Horiz unwrap: get minor length (authentic width), volume, surface area
    let mut horiz_contour_data: Option<(Vec<Point<i32>>, f32, f32, f32, f32, f32)> = None;
    if let Some((box_points, contour)) = get_tight_bounds(&horiz_unwrapped) {
        let (major, minor, angle, cx, cy) = compute_rect_metrics(&box_points);
        let t_scale = if let Some(v_major) = vert_major_px {
            if major > 0.0 { v_major / major } else { 1.0 }
        } else {
            1.0
        };

        let vol = integrate_volume(&contour, cx, cy, angle, t_scale);
        let surf = integrate_surface_area(&contour, cx, cy, angle, t_scale);

        metrics.minor_length = minor * mm_per_px;
        metrics.volume = vol * mm_per_px.powi(3);
        metrics.surface_area = Some(surf * mm_per_px.powi(2));

        horiz_contour_data = Some((contour, major, minor, angle, cx, cy));
    }

    wlog!("[fast] step 7: fruitlet counting");
    let roi_gray = warped;
    let roi_w = roi_gray.width();
    let roi_h = roi_gray.height();

    let block_radius = (COIN_RADIUS_MM * hr_px_per_mm).round() as u32;
    let binary_fc = adaptive_threshold(&roi_gray, block_radius, 0);
    let opened = dilate(&erode(&binary_fc, Norm::LInf, 2), Norm::LInf, 2);
    let labels = connected_components(&opened, Connectivity::Four, Luma([0u8]));
    let regions = collect_regions(&labels);

    let coin_area_px = std::f32::consts::PI * (COIN_RADIUS_MM * hr_px_per_mm).powi(2);
    let area_min = (0.2 * coin_area_px) as u32;
    let area_max = (2.0 * coin_area_px) as u32;
    let aspect_tiers = [(0.4_f32, 1.0_f32), (0.3, 1.0), (0.2, 1.0)];

    let equator_y = roi_h as f32 / 2.0;
    let center_x = roi_w as f32 / 2.0;

    let mut selected: Option<(f32, f32, f32)> = None; // (major, minor, angle)

    'tiers: for &(ar_min, ar_max) in &aspect_tiers {
        let mut candidates: Vec<(f32, f32, f32, f32)> = Vec::new(); // (major, minor, angle, dist)

        for (_label, region) in &regions {
            if region.area < area_min || region.area > area_max {
                continue;
            }
            if region.bbox_max_y < equator_y as i32 || region.bbox_min_y > equator_y as i32 {
                continue;
            }
            let rect = min_area_rect(&region.points);
            let (major, minor, angle) = compute_fruitlet_rect(&rect);
            if major <= 0.0 {
                continue;
            }
            let aspect = minor / major;
            if aspect < ar_min || aspect > ar_max {
                continue;
            }
            let dist = (region.centroid_x - center_x).abs();
            candidates.push((major, minor, angle, dist));
        }

        if !candidates.is_empty() {
            candidates.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));
            let best = &candidates[0];
            selected = Some((best.0, best.1, best.2));
            break 'tiers;
        }
    }

    if let Some((major_px, minor_px, angle_raw)) = selected {
        let a_eq_mm = major_px * mm_per_px;
        let b_eq_mm = minor_px * mm_per_px;
        let pi = std::f32::consts::PI;
        let alpha = {
            let diff = (angle_raw - std::f32::consts::FRAC_PI_2).abs();
            if diff > pi / 2.0 { pi - diff } else { diff }
        };
        metrics.a_eq = Some(a_eq_mm);
        metrics.b_eq = Some(b_eq_mm);
        metrics.alpha = Some(alpha);

        let a_eye = a_eq_mm * b_eq_mm;
        if a_eye > 0.0 {
            if let Some(surface_area) = metrics.surface_area {
                // Polar cap subtraction
                let cap_area = if let Some((ref horiz_contour, _h_major, _h_minor, h_angle, h_cx, h_cy)) =
                    horiz_contour_data
                {
                    let cos_a = h_angle.cos();
                    let sin_a = h_angle.sin();
                    let mut tr: Vec<(f32, f32)> = horiz_contour
                        .iter()
                        .map(|pt| {
                            let lx = pt.x as f32 - h_cx;
                            let ly = pt.y as f32 - h_cy;
                            let t = lx * cos_a + ly * sin_a;
                            let r = (-lx * sin_a + ly * cos_a).abs();
                            (t, r)
                        })
                        .collect();
                    tr.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    let t_min = tr.first().map(|p| p.0).unwrap_or(0.0);
                    let t_max = tr.last().map(|p| p.0).unwrap_or(0.0);
                    let window_px = (a_eq_mm / mm_per_px) / 2.0;
                    let front: Vec<f32> = tr.iter().filter(|(t, _)| *t <= t_min + window_px).map(|(_, r)| *r).collect();
                    let back: Vec<f32> = tr.iter().filter(|(t, _)| *t >= t_max - window_px).map(|(_, r)| *r).collect();
                    let r_front = if front.is_empty() { 0.0 } else { front.iter().sum::<f32>() / front.len() as f32 };
                    let r_back = if back.is_empty() { 0.0 } else { back.iter().sum::<f32>() / back.len() as f32 };
                    let r_front_mm = r_front * mm_per_px;
                    let r_back_mm = r_back * mm_per_px;
                    std::f32::consts::PI * (r_front_mm.powi(2) + r_back_mm.powi(2))
                } else {
                    0.0
                };

                let s_effective = (surface_area - cap_area).max(0.0);
                let n_total = (s_effective / a_eye).floor() as u32;
                metrics.n_total = Some(n_total);
            }
        }
    }

    wlog!("[fast] COMPLETE — all steps done");
    Ok(metrics)
}
