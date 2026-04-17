use image::{DynamicImage, ImageBuffer, Luma, Rgba, RgbaImage, imageops};
use imageops::FilterType;
use imageproc::{
    contours::find_contours_with_threshold,
    distance_transform::Norm,
    drawing::{draw_hollow_polygon_mut, draw_line_segment_mut},
    geometric_transformations::{Interpolation, rotate_about_center},
    geometry::{arc_length, min_area_rect},
    morphology::{dilate, erode},
    point::Point,
};

use std::sync::Arc;

use iced::time::Instant;

use crate::{Preview, correction::unwrap, error::Error};

use super::roi_extraction::extract_best_roi;
use super::{CoordinateTransform, FruitletMetrics, Intermediate, Step};

/// Helper to find minAreaRect bounds for an unwrapped image
fn get_tight_bounds(
    img: &ImageBuffer<Luma<u8>, Vec<u8>>,
) -> Option<([Point<i32>; 4], Vec<Point<i32>>)> {
    let otsu = imageproc::contrast::otsu_level(img);
    let binary =
        imageproc::contrast::threshold(img, otsu, imageproc::contrast::ThresholdType::Binary);

    // Multi-scale morphology (Doc §3.3): resize to 0.25x
    let w = binary.width();
    let h = binary.height();
    let small_w = (w as f32 * 0.25).max(1.0) as u32;
    let small_h = (h as f32 * 0.25).max(1.0) as u32;

    let small = imageops::resize(&binary, small_w, small_h, imageops::FilterType::Nearest);

    // Morphology close (dilate then erode) then open (erode then dilate)
    // 5x5 equivalent = radius 2
    let d1 = dilate(&small, Norm::LInf, 2);
    let closed = erode(&d1, Norm::LInf, 2);
    // 7x7 equivalent = radius 3
    let e1 = erode(&closed, Norm::LInf, 3);
    let opened = dilate(&e1, Norm::LInf, 3);

    // Restore size
    let restored = imageops::resize(&opened, w, h, imageops::FilterType::Nearest);

    let contours = find_contours_with_threshold(&restored, 127);
    contours
        .into_iter()
        .filter(|c| c.points.len() >= 3)
        .max_by(|a, b| arc_length(&a.points, true).total_cmp(&arc_length(&b.points, true)))
        .map(|c| {
            let corners = min_area_rect(&c.points);
            Some((corners, c.points))
        })
        .flatten()
}

/// Extracts rect metrics (major/minor lengths, angle, center) from min_area_rect box points.
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

/// Integrates volume of a body of revolution using the disk method
/// with trapezoidal cross-section area interpolation.
///
/// For each contour point, we compute:
/// - `t`: the signed projection along the rotation axis (major axis direction)
/// - `r`: the signed perpendicular distance from the rotation axis
///
/// Only the **upper half** of the contour (r ≥ 0) is used for integration.
/// A single profile is sufficient to define a body of revolution; using both
/// halves would interleave upper/lower profiles in the sorted sequence,
/// producing incorrect slab interpolation.
///
/// After filtering and sorting by t, consecutive point pairs contribute
/// trapezoidal slabs:  π × (r₀² + r₁²) / 2 × Δt
///
/// The `t_scale` parameter applies a linear correction to the axial coordinate
/// so that `t` values (from HORIZ_UNWRAP, where the axial direction is NOT
/// perspective-corrected) are rescaled to match the true physical height
/// obtained from VERT_UNWRAP.
///
/// Uses f64 accumulator internally to reduce rounding errors.
fn integrate_volume(contour: &[Point<i32>], cx: f32, cy: f32, angle: f32, t_scale: f32) -> f32 {
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    // Collect (t, r) pairs for the UPPER HALF of the contour (r ≥ 0).
    // A single profile fully defines the body of revolution.
    let mut tr_points: Vec<(f32, f32)> = Vec::with_capacity(contour.len());
    for pt in contour {
        let lx = pt.x as f32 - cx;
        let ly = pt.y as f32 - cy;
        // t = projection along rotation axis (major axis)
        let t = (lx * cos_a + ly * sin_a) * t_scale;
        // r = signed perpendicular distance from rotation axis
        let r = -lx * sin_a + ly * cos_a;
        if r >= 0.0 {
            tr_points.push((t, r));
        }
    }

    // Sort by t (along-axis coordinate)
    tr_points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Disk integration with trapezoidal area interpolation on the upper profile:
    // For each consecutive pair, the cross-section area is linearly
    // interpolated as π(r₀² + r₁²)/2, which is more accurate than
    // using max(r₀, r₁).
    let mut vol: f64 = 0.0;
    for w in tr_points.windows(2) {
        let dt = (w[1].0 - w[0].0) as f64;
        let r0 = w[0].1 as f64;
        let r1 = w[1].1 as f64;
        vol += std::f64::consts::PI * (r0 * r0 + r1 * r1) / 2.0 * dt;
    }
    vol as f32
}

/// Integrates surface area of a body of revolution using the same
/// contour data and conventions as `integrate_volume`.
///
/// For a curve r(t) rotated about the t-axis, the surface area is:
///   S = ∫ 2π r(t) √(1 + (dr/dt)²) dt
///
/// Unlike volume (where dt≈0 ⇒ contribution≈0), the surface area
/// integral accumulates arc-length ds = √(Δt²+Δr²), so raw pixel-level
/// contour points that zigzag in r at similar t values drastically
/// inflate the result.  To avoid this, we bin the upper-half contour
/// and take the maximum r in each bucket, producing a clean envelope
/// profile r(t).
///
/// The bin width is set to `t_scale` (≈ 1 original pixel in the scaled
/// coordinate system) so that each bin is guaranteed to contain contour
/// points.  Any remaining empty bins are linearly interpolated from
/// their neighbours.
fn integrate_surface_area(
    contour: &[Point<i32>],
    cx: f32,
    cy: f32,
    angle: f32,
    t_scale: f32,
) -> f32 {
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    // Collect upper-half (t, r) pairs
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

    // Determine t range
    let t_min = tr_points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let t_max = tr_points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let t_range = t_max - t_min;
    if t_range <= 0.0 {
        return 0.0;
    }

    // Bin width ≈ t_scale so each bin spans ~1 original (unscaled) pixel.
    // This prevents empty bins caused by stretched coordinates.
    let bin_width = t_scale.max(1.0);
    let n_bins = ((t_range / bin_width).ceil() as usize).max(2);
    let bin_width = t_range / n_bins as f32; // recompute to exactly span range

    let mut bins: Vec<f32> = vec![-1.0; n_bins]; // -1 = empty sentinel

    for &(t, r) in &tr_points {
        let idx = ((t - t_min) / bin_width).floor() as usize;
        let idx = idx.min(n_bins - 1);
        if r > bins[idx] {
            bins[idx] = r;
        }
    }

    // Linearly interpolate any remaining empty bins from neighbours
    // (forward pass then backward pass to handle runs of empties)
    let mut last_valid: Option<(usize, f32)> = None;
    for i in 0..n_bins {
        if bins[i] >= 0.0 {
            // Fill any gap between last_valid and i
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
    // Fill leading/trailing empties with nearest valid value
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

    // Build the profile and integrate
    let mut area: f64 = 0.0;
    for i in 0..(n_bins - 1) {
        let t0 = (t_min + (i as f32 + 0.5) * bin_width) as f64;
        let t1 = (t_min + (i as f32 + 1.5) * bin_width) as f64;
        let r0 = bins[i].max(0.0) as f64;
        let r1 = bins[i + 1].max(0.0) as f64;
        let dt = t1 - t0;
        let dr = r1 - r0;
        let r_avg = (r0 + r1) / 2.0;
        let ds = (dt * dt + dr * dr).sqrt();
        area += 2.0 * std::f64::consts::PI * r_avg * ds;
    }
    area as f32
}

/// Draws a dashed line from `start` to `end` on the preview image.
fn draw_dashed_line(img: &mut RgbaImage, start: (f32, f32), end: (f32, f32), color: Rgba<u8>) {
    let dash_length = 10.0;
    let gap_length = 5.0;

    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= 0.1 {
        return;
    }
    let (ux, uy) = (dx / dist, dy / dist);

    let mut curr = 0.0;
    while curr < dist {
        let s = (start.0 + ux * curr, start.1 + uy * curr);
        let mut end_curr = curr + dash_length;
        if end_curr > dist {
            end_curr = dist;
        }
        let e = (start.0 + ux * end_curr, start.1 + uy * end_curr);
        draw_line_segment_mut(img, s, e, color);
        curr += dash_length + gap_length;
    }
}

/// Draws all four edges of the bounding rectangle using `draw_hollow_polygon_mut`,
/// plus a dashed centerline along the major (longer) axis.
fn draw_panel_bounds(
    color_preview: &mut RgbaImage,
    box_points: &[Point<i32>; 4],
    x_offset: u32,
    y_offset: u32,
) {
    let red = Rgba([255, 0, 0, 255]);

    // Offset all points to the panel position
    let offset_points: Vec<Point<i32>> = box_points
        .iter()
        .map(|p| Point::new(p.x + x_offset as i32, p.y + y_offset as i32))
        .collect();

    // draw_hollow_polygon_mut expects Point<f32>
    let float_points: Vec<Point<f32>> = offset_points
        .iter()
        .map(|p| Point::new(p.x as f32, p.y as f32))
        .collect();

    // Draw all 4 edges of the rectangle
    draw_hollow_polygon_mut(color_preview, &float_points, red);

    // Identify the two longer edges and draw a dashed centerline between their midpoints.
    // Edge 0-1 and Edge 1-2 are adjacent; compare their lengths to find the major axis.
    let d01 = {
        let dx = (box_points[1].x - box_points[0].x) as f32;
        let dy = (box_points[1].y - box_points[0].y) as f32;
        dx * dx + dy * dy
    };
    let d12 = {
        let dx = (box_points[2].x - box_points[1].x) as f32;
        let dy = (box_points[2].y - box_points[1].y) as f32;
        dx * dx + dy * dy
    };

    // Midpoints of the two longer (major-axis) edges
    let (mid_a, mid_b) = if d01 >= d12 {
        // Edges 0-1 and 2-3 are the longer pair
        (
            (
                (offset_points[0].x + offset_points[1].x) as f32 / 2.0,
                (offset_points[0].y + offset_points[1].y) as f32 / 2.0,
            ),
            (
                (offset_points[2].x + offset_points[3].x) as f32 / 2.0,
                (offset_points[2].y + offset_points[3].y) as f32 / 2.0,
            ),
        )
    } else {
        // Edges 1-2 and 3-0 are the longer pair
        (
            (
                (offset_points[1].x + offset_points[2].x) as f32 / 2.0,
                (offset_points[1].y + offset_points[2].y) as f32 / 2.0,
            ),
            (
                (offset_points[3].x + offset_points[0].x) as f32 / 2.0,
                (offset_points[3].y + offset_points[0].y) as f32 / 2.0,
            ),
        )
    };

    draw_dashed_line(color_preview, mid_a, mid_b, red);
}

/// Process the `BinaryFusion` step: ROI extraction, dual-axis unwrapping,
/// metric calculation, and visualization panel composition.
pub(crate) fn process_binary_fusion(
    inter: &Intermediate,
    image: &DynamicImage,
) -> Result<Intermediate, Error> {
    // Step 5: ROI Extraction (Morphology / ROI Extraction) & Unwrapping
    let smoothed = inter
        .context_image
        .as_ref()
        .ok_or(Error::General("Missing context image".into()))?
        .as_luma8()
        .ok_or(Error::General("Context image is not Luma8".into()))?;
    let px_per_mm = inter
        .pixels_per_mm
        .ok_or(Error::General("Missing scale".into()))?;
    let contours = inter
        .contours
        .as_ref()
        .ok_or(Error::General("Missing contours".into()))?;

    let fused_luma = inter
        .fused_image
        .as_ref()
        .ok_or(Error::General("Missing fused image".into()))?
        .as_luma8()
        .ok_or(Error::General("Fused image is not Luma8".into()))?;
    let contours_vec: Vec<_> = (**contours).clone();
    let roi_rect_low_res = extract_best_roi(smoothed, px_per_mm, contours_vec, fused_luma)?;

    // Extract ROI
    if let Some(roi_rect_low_res) = roi_rect_low_res {
        let gray_original = if let Some(ref hr) = inter.original_high_res {
            hr.to_luma8()
        } else {
            image.to_luma8()
        };

        let scale = gray_original.width() as f32 / image.width() as f32;

        let hr_cx = roi_rect_low_res.cx * scale;
        let hr_cy = roi_rect_low_res.cy * scale;
        let hr_w = (roi_rect_low_res.width * scale).round() as u32;
        let hr_h = (roi_rect_low_res.height * scale).round() as u32;

        // Unrotated Context ROI for center panel preview
        let diag = ((hr_w as f32).powi(2) + (hr_h as f32).powi(2)).sqrt();
        let safe_x = (hr_cx - diag / 2.0).round() as i32;
        let safe_y = (hr_cy - diag / 2.0).round() as i32;
        let safe_w = diag.ceil() as u32;
        let safe_h = diag.ceil() as u32;

        let bbox_x = safe_x.max(0) as u32;
        let bbox_y = safe_y.max(0) as u32;
        let bbox_w = safe_w;
        let bbox_h = safe_h;

        let mut padded_crop: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(safe_w, safe_h);

        for y in 0..safe_h {
            for x in 0..safe_w {
                let src_x = safe_x + x as i32;
                let src_y = safe_y + y as i32;
                if src_x >= 0
                    && src_x < gray_original.width() as i32
                    && src_y >= 0
                    && src_y < gray_original.height() as i32
                {
                    padded_crop.put_pixel(
                        x,
                        y,
                        *gray_original.get_pixel(src_x as u32, src_y as u32),
                    );
                } else {
                    padded_crop.put_pixel(x, y, Luma([0]));
                }
            }
        }

        // To replicate cv2.warpPerspective which inherently rotates the ROI to upright:
        // 1. Rotate `center_panel` around its center by the box angle.
        // `min_area_rect` gives us an angle where width/height are aligned.
        let rotated_panel = rotate_about_center(
            &padded_crop,
            roi_rect_low_res.angle_rad,
            Interpolation::Bilinear,
            Luma([0]),
        );

        // 2. Crop exactly the (hr_w, hr_h) region from the center of this rotated panel
        let rot_center_x = rotated_panel.width() as f32 / 2.0;
        let rot_center_y = rotated_panel.height() as f32 / 2.0;
        let crop_x = (rot_center_x - hr_w as f32 / 2.0).round() as i32;
        let crop_y = (rot_center_y - hr_h as f32 / 2.0).round() as i32;

        // Clamp crop parameters to rotated_panel bounds
        let rp_w = rotated_panel.width();
        let rp_h = rotated_panel.height();
        let crop_x = crop_x.max(0).min(rp_w.saturating_sub(1) as i32) as u32;
        let crop_y = crop_y.max(0).min(rp_h.saturating_sub(1) as i32) as u32;
        let clamped_w = hr_w.min(rp_w - crop_x).max(1);
        let clamped_h = hr_h.min(rp_h - crop_y).max(1);

        let warped = imageops::crop_imm(
            &rotated_panel,
            crop_x,
            crop_y,
            clamped_w,
            clamped_h,
        )
        .to_image();

        let best_roi = Arc::new(DynamicImage::ImageLuma8(warped.clone()));

        let transform = CoordinateTransform {
            bbox_x,
            bbox_y,
            extract_x: 0,
            extract_y: 0,
            local_width: bbox_w,
            local_height: bbox_h,
            angle_rad: roi_rect_low_res.angle_rad,
            radius: best_roi.width() as f32 / 2.0,
        };

        // Direct tilt-aware Unwrapping
        // ---- 6. Mathematical Forward Mapping for Exact Metrics & Unwrapped Views ----
        // Panel 1 & 2: vertical unwrap
        let hr_w = warped.width();
        let hr_h = warped.height();

        // Left panel: vertical_unwrapped (Vertical Cylinder)
        let vert_unwrapped = unwrap(&warped);

        // Right panel: horizontal_unwrapped (Horizontal Cylinder)
        let horiz_rotated = ::image::imageops::rotate90(&warped);
        // Using `unwrap` directly (with f = r = H_roi) matches Doc §3.2 HORIZ_UNWRAP:
        // the rotated image's width is H_roi, so `unwrap` implicitly uses H_roi as f and r.
        // This is the correct projection for obtaining accurate radial (width) values.
        let horiz_unwrapped = unwrap(&horiz_rotated);

        // Build 3 panel image: vert_unwrapped (w x h) | warped | horiz_unwrapped (h x w)
        let padding = 10;
        let panel1_w = hr_w;
        let panel2_w = hr_w; // warped width
        let panel3_w = hr_h;

        let max_h = hr_h.max(hr_w);
        let total_w = panel1_w + panel2_w + panel3_w + padding * 2;

        let mut combined_preview: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(total_w, max_h);

        for p in combined_preview.pixels_mut() {
            *p = Luma([255]);
        }

        // Offsets
        let x1 = 0;
        let x2 = panel1_w + padding;
        let x3 = panel1_w + panel2_w + padding * 2;

        // Center them vertically
        let y1 = (max_h - hr_h) / 2;
        let y2 = (max_h - hr_h) / 2; // warped height
        let y3 = (max_h - hr_w) / 2;

        imageops::replace(&mut combined_preview, &vert_unwrapped, x1 as i64, y1 as i64);
        imageops::replace(&mut combined_preview, &warped, x2 as i64, y2 as i64);
        imageops::replace(
            &mut combined_preview,
            &horiz_unwrapped,
            x3 as i64,
            y3 as i64,
        );

        let mut color_preview: RgbaImage = DynamicImage::ImageLuma8(combined_preview).to_rgba8();

        // --- DRAW TIGHT BOUNDING BASELINES ---
        let mut calculated_metrics = None;
        let mut horiz_contour_arc: Option<Arc<Vec<imageproc::point::Point<i32>>>> = None;
        let mut horiz_metrics_opt: Option<(f32, f32, f32, f32, f32)> = None;

        // Panel 1: vert_unwrapped (Vertical Cylinder)
        // Left panel: top/bottom edges solid, vertical centerline dashed
        // VERT_UNWRAP corrects height curvature → its major axis = authentic height.
        // We extract major length here for dimension assignment AND for t-axis rescaling.
        let mut vert_major_px: Option<f32> = None;
        if let Some((box_points, _contour)) = get_tight_bounds(&vert_unwrapped) {
            let (major, minor, _angle, _cx, _cy) = compute_rect_metrics(&box_points);

            draw_panel_bounds(&mut color_preview, &box_points, x1, y1);

            vert_major_px = Some(major);

            if let Some(px_per_mm) = inter.pixels_per_mm {
                let hr_px_per_mm = px_per_mm * scale;
                let mm_per_px = 1.0 / hr_px_per_mm;
                let v_major = major * mm_per_px;
                let v_minor = minor * mm_per_px;

                log::info!(
                    "VERT_UNWRAP (Corrects Height): V_major(Height)={}, V_minor(Width)={}",
                    v_major,
                    v_minor,
                );

                calculated_metrics = Some(FruitletMetrics {
                    major_length: v_major, // Authentic Height
                    minor_length: v_minor, // Temp, will be replaced by HORIZ minor
                    volume: 0.0,           // Will be computed from HORIZ contour
                    a_eq: None,
                    b_eq: None,
                    alpha: None,
                    surface_area: None, // Will be computed from HORIZ contour
                    n_total: None,
                });
            }
        }

        // Panel 3: horiz_unwrapped (Horizontal Cylinder)
        // Right panel: left/right edges solid, horizontal centerline dashed
        // HORIZ_UNWRAP corrects width curvature → its minor axis = authentic width,
        // and its contour provides accurate radial (r) values for volume integration.
        // The axial (t) coordinates are rescaled by vert_major/horiz_major to fuse
        // the corrected height from VERT_UNWRAP.
        if let Some((box_points, contour)) = get_tight_bounds(&horiz_unwrapped) {
            let (major, minor, angle, cx, cy) = compute_rect_metrics(&box_points);

            draw_panel_bounds(&mut color_preview, &box_points, x3, y3);

            // Save HORIZ_UNWRAP data for fruitlet counting step
            horiz_contour_arc = Some(Arc::new(
                contour
                    .iter()
                    .map(|p| imageproc::point::Point::new(p.x, p.y))
                    .collect(),
            ));
            horiz_metrics_opt = Some((major, minor, angle, cx, cy));

            // Dual-view fusion: rescale t-axis so axial coordinates match
            // the perspective-corrected height from VERT_UNWRAP.
            let t_scale = if let Some(v_major) = vert_major_px {
                if major > 0.0 { v_major / major } else { 1.0 }
            } else {
                1.0
            };

            let vol = integrate_volume(&contour, cx, cy, angle, t_scale);
            let surf = integrate_surface_area(&contour, cx, cy, angle, t_scale);

            if let Some(metrics) = calculated_metrics.as_mut() {
                if let Some(px_per_mm) = inter.pixels_per_mm {
                    let hr_px_per_mm = px_per_mm * scale;
                    let mm_per_px = 1.0 / hr_px_per_mm;

                    let h_major = major * mm_per_px;
                    let h_minor = minor * mm_per_px;
                    let h_vol = vol * mm_per_px.powi(3);

                    log::info!(
                        "HORIZ_UNWRAP (Corrects Width): H_major={}, H_minor(Width)={}, t_scale={:.4}, H_vol={}",
                        h_major,
                        h_minor,
                        t_scale,
                        h_vol
                    );

                    // VERT_UNWRAP major → Authentic Height (already set)
                    // HORIZ_UNWRAP minor → Authentic Width
                    // HORIZ_UNWRAP contour + t_scale → Authentic Volume
                    metrics.minor_length = h_minor;
                    metrics.volume = h_vol;
                    let s_mm2 = surf * mm_per_px.powi(2);
                    metrics.surface_area = Some(s_mm2);

                    // Reference: prolate spheroid surface area for comparison
                    let a_sph = metrics.major_length / 2.0; // semi-major (height)
                    let b_sph = h_minor / 2.0; // semi-minor (width)
                    let s_ref = if (a_sph - b_sph).abs() < 0.1 {
                        4.0 * std::f32::consts::PI * a_sph * a_sph
                    } else if a_sph > b_sph {
                        let e = (1.0 - (b_sph / a_sph).powi(2)).sqrt();
                        2.0 * std::f32::consts::PI * b_sph * b_sph
                            + 2.0 * std::f32::consts::PI * a_sph * b_sph / e * (e).asin()
                    } else {
                        let e = (1.0 - (a_sph / b_sph).powi(2)).sqrt();
                        2.0 * std::f32::consts::PI * a_sph * a_sph
                            + std::f32::consts::PI * b_sph * b_sph / e
                                * ((1.0 + e) / (1.0 - e)).ln()
                    };

                    log::info!(
                        "SURFACE AREA: contour_integral={:.1}mm², spheroid_ref={:.1}mm², ratio={:.3}, t_scale={:.4}",
                        s_mm2,
                        s_ref,
                        s_mm2 / s_ref,
                        t_scale
                    );

                    log::info!(
                        "FINAL METRICS: Height={}, Width={}, Volume={}, SurfaceArea={}",
                        metrics.major_length,
                        metrics.minor_length,
                        metrics.volume,
                        metrics.surface_area.unwrap_or(0.0)
                    );
                }
            }
        }

        // Downscale for preview (keep consistent UI)
        let preview_img = if scale > 1.1 {
            DynamicImage::ImageRgba8(color_preview).resize(
                total_w.min(1000), // cap width to avoid massive rendering hang
                max_h,
                FilterType::Lanczos3,
            )
        } else {
            DynamicImage::ImageRgba8(color_preview)
        };

        Ok(Intermediate {
            current_step: Step::RoiExtraction,
            preview: Preview::ready(preview_img.into(), Instant::now()),
            pixels_per_mm: inter.pixels_per_mm,
            binary_image: inter.binary_image.clone(),
            fused_image: inter.fused_image.clone(),
            contours: inter.contours.clone(),
            context_image: Some(best_roi.clone()),
            roi_image: Some(best_roi),
            original_high_res: inter.original_high_res.clone(),
            transform: Some(transform),
            metrics: calculated_metrics,
            horiz_contour: horiz_contour_arc,
            horiz_rect_metrics: horiz_metrics_opt,
            scale_factor: Some(scale),
        })
    } else {
        Err(Error::General("No ROI found in Step 5".into()))
    }
}
