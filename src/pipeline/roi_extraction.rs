use image::GrayImage;
use imageproc::{
    contours::{self, BorderType},
    geometry::{contour_area as geometry_contour_area, min_area_rect},
    point::Point,
};

use crate::error::Error;

use super::COIN_RADIUS_MM;

#[derive(Clone, Copy, Debug)]
pub(crate) struct RotatedRect {
    pub(crate) cx: f32,
    pub(crate) cy: f32,
    pub(crate) width: f32,     // Upright Width
    pub(crate) height: f32,    // Upright Height
    pub(crate) angle_rad: f32, // Rotation applied to image to make it upright
}

pub(crate) fn get_rotated_rect_info(points: &[Point<i32>]) -> RotatedRect {
    // We expect 4 points.
    if points.len() != 4 {
        return RotatedRect {
            cx: 0.0,
            cy: 0.0,
            width: 0.0,
            height: 0.0,
            angle_rad: 0.0,
        };
    }

    // Convert to float for simpler math
    let pts: Vec<(f32, f32)> = points.iter().map(|p| (p.x as f32, p.y as f32)).collect();

    // Calculate Edge Lengths
    // Edge 0: 0-1
    // Edge 1: 1-2
    let d0 = ((pts[1].0 - pts[0].0).powi(2) + (pts[1].1 - pts[0].1).powi(2)).sqrt();
    let d1 = ((pts[2].0 - pts[1].0).powi(2) + (pts[2].1 - pts[1].1).powi(2)).sqrt();

    let cx = (pts[0].0 + pts[1].0 + pts[2].0 + pts[3].0) / 4.0;
    let cy = (pts[0].1 + pts[1].1 + pts[2].1 + pts[3].1) / 4.0;

    // Identify Long Axis
    // Pineapple is usually Taller than Wide.
    // We want the Long Axis to be Vertical (Y).

    let (width, height, theta) = if d0 > d1 {
        // Edge 0 is Height
        // Angle of Edge 0
        let dx = pts[1].0 - pts[0].0;
        let dy = pts[1].1 - pts[0].1;
        let theta = dy.atan2(dx);
        (d1, d0, theta)
    } else {
        // Edge 1 is Height
        let dx = pts[2].0 - pts[1].0;
        let dy = pts[2].1 - pts[1].1;
        let theta = dy.atan2(dx);
        (d0, d1, theta)
    };

    // Calculate minimal rotation to vertical
    // We want to rotate such that the long axis becomes vertical.
    // This could be -PI/2 (Up) or PI/2 (Down).
    // We choose the rotation with smallest magnitude to avoid flipping the image upside down
    // if it is already mostly upright.

    let pi = std::f32::consts::PI;
    let normalize = |mut r: f32| {
        while r <= -pi {
            r += 2.0 * pi;
        }
        while r > pi {
            r -= 2.0 * pi;
        }
        r
    };

    let rot_up = normalize(-std::f32::consts::FRAC_PI_2 - theta);
    let rot_down = normalize(std::f32::consts::FRAC_PI_2 - theta);

    let angle = if rot_up.abs() < rot_down.abs() {
        rot_up
    } else {
        rot_down
    };

    RotatedRect {
        cx,
        cy,
        width,
        height,
        angle_rad: angle,
    }
}

pub(crate) fn extract_best_roi(
    smoothed: &GrayImage,
    px_per_mm: f32,
    contours: Vec<imageproc::contours::Contour<i32>>,
    _fused: &GrayImage,
) -> Result<Option<RotatedRect>, Error> {
    // 2. Filter by Physical Area
    let coin_area_px = std::f32::consts::PI * (COIN_RADIUS_MM * px_per_mm).powi(2);
    let min_area = 0.2 * coin_area_px;

    // Compute centroid and area for each qualifying Otsu candidate
    struct OtsuCandidate {
        centroid_x: i32,
        centroid_y: i32,
    }

    let mut candidates: Vec<OtsuCandidate> = Vec::with_capacity(contours.len());
    for contour in &contours {
        let area = geometry_contour_area(&contour.points).abs() as f32;
        if area > min_area {
            let n = contour.points.len() as i64;
            let (sx, sy) = contour.points.iter().fold((0i64, 0i64), |(sx, sy), pt| {
                (sx + pt.x as i64, sy + pt.y as i64)
            });
            candidates.push(OtsuCandidate {
                centroid_x: if n > 0 { (sx / n) as i32 } else { 0 },
                centroid_y: if n > 0 { (sy / n) as i32 } else { 0 },
            });
        }
    }

    if candidates.is_empty() {
        return Err(Error::General("No valid ROI found".into()));
    }

    // 3. Low-threshold binarization — use as natural object boundaries for grouping
    //
    // At threshold 25, fruit tissue (≈ 30+) stays white while background gaps
    // (≈ 5-15) remain black, providing natural object separation.
    let low_binary = imageproc::contrast::threshold(
        smoothed,
        25,
        imageproc::contrast::ThresholdType::Binary,
    );
    let low_contours = contours::find_contours::<i32>(&low_binary);
    let outer_low: Vec<_> = low_contours
        .iter()
        .filter(|c| c.border_type == BorderType::Outer)
        .collect();

    // 4. Group Otsu candidates by which low-threshold contour contains their centroid
    //
    // For each low-threshold contour, check if it contains any Otsu candidates.
    // Use the low-threshold contour's own geometric area (NOT the sum of Otsu
    // fragment areas) as the area term.  Otsu fragments only capture bright
    // fruitlet mounds; inter-fruitlet gaps are black in the Otsu mask, so the
    // sum of fragment areas drastically underrepresents the peel side's true
    // physical extent.  The low-threshold contour area correctly reflects each
    // object's full size, making edge_density the sole discriminator.
    let (img_w, img_h) = smoothed.dimensions();
    let bg_threshold = 15u8;

    struct GroupScore {
        low_contour_idx: usize,
        score: f32,
        edge_density: f64,
        contour_area: f32,
    }

    let mut group_scores: Vec<GroupScore> = Vec::new();

    for (li, lc) in outer_low.iter().enumerate() {
        // Low-threshold contour's own geometric area
        let contour_area = geometry_contour_area(&lc.points).abs() as f32;
        if contour_area < min_area {
            continue; // Too small (e.g. noise speck)
        }

        // Compute AABB of this low-threshold contour
        let (lx_min, ly_min, lx_max, ly_max) = lc.points.iter().fold(
            (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
            |(xn, yn, xx, yx), pt| (xn.min(pt.x), yn.min(pt.y), xx.max(pt.x), yx.max(pt.y)),
        );

        // Check that at least one Otsu candidate's centroid falls within this AABB.
        // This prevents non-fruit objects (ruler, background artifacts) whose Otsu
        // contours were already filtered out from being scored.
        let has_member = candidates.iter().any(|cand| {
            cand.centroid_x >= lx_min
                && cand.centroid_x <= lx_max
                && cand.centroid_y >= ly_min
                && cand.centroid_y <= ly_max
        });

        if !has_member {
            continue;
        }

        // Compute edge density over the low-threshold contour's AABB
        let bx0 = (lx_min.max(0) as u32).min(img_w.saturating_sub(1));
        let by0 = (ly_min.max(0) as u32).min(img_h.saturating_sub(1));
        let bx1 = (lx_max.max(0) as u32).min(img_w.saturating_sub(1));
        let by1 = (ly_max.max(0) as u32).min(img_h.saturating_sub(1));

        let mut gradient_sum: f64 = 0.0;
        let mut pixel_count: u32 = 0;

        for y in by0..by1.min(img_h - 1) {
            for x in bx0..bx1.min(img_w - 1) {
                let p = smoothed.get_pixel(x, y).0[0];
                if p <= bg_threshold {
                    continue;
                }
                let px_right = smoothed.get_pixel(x + 1, y).0[0];
                let py_down = smoothed.get_pixel(x, y + 1).0[0];
                let dx = (p as i16 - px_right as i16).unsigned_abs() as f64;
                let dy = (p as i16 - py_down as i16).unsigned_abs() as f64;
                gradient_sum += dx + dy;
                pixel_count += 1;
            }
        }

        let edge_density = if pixel_count > 0 {
            gradient_sum / pixel_count as f64
        } else {
            0.0
        };

        let score = edge_density as f32 * contour_area.sqrt();

        log::info!(
            "[ROI Score] Group {} (low-thresh contour): contour_area={:.0}, edge_density={:.2}, score={:.1}",
            li,
            contour_area,
            edge_density,
            score
        );

        group_scores.push(GroupScore {
            low_contour_idx: li,
            score,
            edge_density,
            contour_area,
        });
    }

    // Sort by score descending
    group_scores.sort_by(|a, b| b.score.total_cmp(&a.score));

    if let Some(best) = group_scores.first() {
        // The winning group's low-threshold contour IS the bounding contour
        let best_lc = outer_low[best.low_contour_idx];
        if best_lc.points.len() < 3 {
            return Err(Error::General("Best ROI contour has too few points".into()));
        }
        let rect = min_area_rect(&best_lc.points);
        let r_rect = get_rotated_rect_info(&rect);

        log::info!(
            "[Step 5] Best ROI (low-thresh guided, {} pts): score={:.2}, edge_density={:.2}, contour_area={:.0}, rect={:?}",
            best_lc.points.len(),
            best.score,
            best.edge_density,
            best.contour_area,
            r_rect
        );

        Ok(Some(r_rect))
    } else {
        Err(Error::General("No valid ROI found".into()))
    }
}
