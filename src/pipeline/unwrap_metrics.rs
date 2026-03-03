use image::{DynamicImage, ImageBuffer, Luma, Rgba, RgbaImage, imageops};
use imageops::FilterType;
use imageproc::{
    contours::find_contours_with_threshold,
    distance_transform::Norm,
    drawing::draw_line_segment_mut,
    geometric_transformations::{Interpolation, rotate_about_center},
    geometry::min_area_rect,
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

    // Python matching morphology: resize to 0.25x
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
    if let Some(longest) = contours.into_iter().max_by_key(|c| c.points.len()) {
        let corners = min_area_rect(&longest.points);
        Some((corners, longest.points))
    } else {
        None
    }
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
/// - `r`: the perpendicular distance from the rotation axis
///
/// All contour points are used (both halves of the fruit). After sorting by t,
/// consecutive point pairs contribute a trapezoidal slab:
///   π × (r₀² + r₁²) / 2 × Δt
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

    // Collect (t, r) pairs for the FULL contour (both halves)
    let mut tr_points: Vec<(f32, f32)> = Vec::with_capacity(contour.len());
    for pt in contour {
        let lx = pt.x as f32 - cx;
        let ly = pt.y as f32 - cy;
        // t = projection along rotation axis (major axis)
        let t = (lx * cos_a + ly * sin_a) * t_scale;
        // r = perpendicular distance from rotation axis
        let r = (-lx * sin_a + ly * cos_a).abs();
        tr_points.push((t, r));
    }

    // Sort by t (along-axis coordinate)
    tr_points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Disk integration with trapezoidal area interpolation:
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

/// Draws solid top/bottom edges and a dashed centerline for a panel's tight bounds.
fn draw_panel_bounds(
    color_preview: &mut RgbaImage,
    box_points: &mut [Point<i32>; 4],
    x_offset: u32,
    y_offset: u32,
) {
    let red = Rgba([255, 0, 0, 255]);
    box_points.sort_by_key(|p| p.y);

    // Top edge = 0 and 1, Bottom edge = 2 and 3
    let top_mid_x = (box_points[0].x as f32 + box_points[1].x as f32) / 2.0 + x_offset as f32;
    let top_mid_y = (box_points[0].y as f32 + box_points[1].y as f32) / 2.0 + y_offset as f32;

    let bot_mid_x = (box_points[2].x as f32 + box_points[3].x as f32) / 2.0 + x_offset as f32;
    let bot_mid_y = (box_points[2].y as f32 + box_points[3].y as f32) / 2.0 + y_offset as f32;

    // Solid top edge
    draw_line_segment_mut(
        color_preview,
        (
            box_points[0].x as f32 + x_offset as f32,
            box_points[0].y as f32 + y_offset as f32,
        ),
        (
            box_points[1].x as f32 + x_offset as f32,
            box_points[1].y as f32 + y_offset as f32,
        ),
        red,
    );
    // Solid bottom edge
    draw_line_segment_mut(
        color_preview,
        (
            box_points[2].x as f32 + x_offset as f32,
            box_points[2].y as f32 + y_offset as f32,
        ),
        (
            box_points[3].x as f32 + x_offset as f32,
            box_points[3].y as f32 + y_offset as f32,
        ),
        red,
    );

    // Dashed centerline
    draw_dashed_line(
        color_preview,
        (top_mid_x, top_mid_y),
        (bot_mid_x, bot_mid_y),
        red,
    );
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

    let contours_vec: Vec<_> = (**contours).clone();
    let (_, roi_rect_low_res) = extract_best_roi(smoothed, image, px_per_mm, contours_vec)?;

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

        let warped = imageops::crop_imm(
            &rotated_panel,
            crop_x.max(0) as u32,
            crop_y.max(0) as u32,
            hr_w,
            hr_h,
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
            focal_length_px: inter.focal_length_px,
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
        // Reverting to `unwrap` because Python's `unwrap(cv2.rotate(warped))` implicitly uses
        // the rotated image's width (which is `hr_h`) as `f` and `r`.
        // While physically "distorted", this is the exact projection Python uses for volume integration.
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

        // Panel 1: vert_unwrapped (Vertical Cylinder)
        // Left panel: top/bottom edges solid, vertical centerline dashed
        // VERT_UNWRAP corrects height curvature → its major axis = authentic height.
        // We extract major length here for dimension assignment AND for t-axis rescaling.
        let mut vert_major_px: Option<f32> = None;
        if let Some((mut box_points, _contour)) = get_tight_bounds(&vert_unwrapped) {
            let (major, minor, _angle, _cx, _cy) = compute_rect_metrics(&box_points);

            draw_panel_bounds(&mut color_preview, &mut box_points, x1, y1);

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
                });
            }
        }

        // Panel 3: horiz_unwrapped (Horizontal Cylinder)
        // Right panel: left/right edges solid, horizontal centerline dashed
        // HORIZ_UNWRAP corrects width curvature → its minor axis = authentic width,
        // and its contour provides accurate radial (r) values for volume integration.
        // The axial (t) coordinates are rescaled by vert_major/horiz_major to fuse
        // the corrected height from VERT_UNWRAP.
        if let Some((mut box_points, contour)) = get_tight_bounds(&horiz_unwrapped) {
            let (major, minor, angle, cx, cy) = compute_rect_metrics(&box_points);

            draw_panel_bounds(&mut color_preview, &mut box_points, x3, y3);

            // Dual-view fusion: rescale t-axis so axial coordinates match
            // the perspective-corrected height from VERT_UNWRAP.
            let t_scale = if let Some(v_major) = vert_major_px {
                if major > 0.0 { v_major / major } else { 1.0 }
            } else {
                1.0
            };

            let vol = integrate_volume(&contour, cx, cy, angle, t_scale);

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

                    log::info!(
                        "FINAL METRICS: Height={}, Width={}, Volume={}",
                        metrics.major_length,
                        metrics.minor_length,
                        metrics.volume
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
            focal_length_px: inter.focal_length_px,
            transform: Some(transform),
            metrics: calculated_metrics,
        })
    } else {
        Err(Error::General("No ROI found in Step 5".into()))
    }
}
