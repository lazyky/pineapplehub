use image::{GrayImage, ImageBuffer, Luma};

/// Unwraps an already-upright cropped cylinder image using the inverse cylindrical projection.
/// Assumes f = w and r = w (Doc §3.1 auto-scaling geometry).
pub fn unwrap(img: &GrayImage) -> GrayImage {
    let w = img.width() as f32;
    unwrap_with_radius(img, w, w)
}

/// Per-column precomputed values for the cylindrical unwrap.
/// All of these depend only on the x-coordinate and are constant across rows,
/// so precomputing them reduces the expensive `sqrt` from O(W×H) to O(W).
struct ColumnData {
    src_x: f32,
    /// zc / f — the depth ratio used to compute src_y per row
    zc_over_f: f32,
}

/// Unwraps an arbitrarily rotated cylinder where focal length and radius
/// might not be equal to the current image width.
pub fn unwrap_with_radius(img: &GrayImage, f: f32, r: f32) -> GrayImage {
    let w = img.width();
    let h = img.height();

    // Need at least 2×2 pixels for bilinear interpolation
    if w < 2 || h < 2 {
        return img.clone();
    }

    let wf = w as f32;
    let hf = h as f32;

    let mut output = ImageBuffer::new(w, h);

    let omega = wf / 2.0;

    let term1 = (r * r - omega * omega).max(0.0);
    let z0 = f - term1.sqrt();
    let c_term = z0 * z0 - r * r;

    // Hoist loop-invariant products
    let two_z0 = 2.0 * z0;
    let four_z0_sq = 4.0 * z0 * z0;
    let f_sq = f * f;
    let half_w = wf / 2.0;
    let half_h = hf / 2.0;

    // Precompute all column-invariant values: a_term, discriminant, zc, src_x
    // This reduces the expensive sqrt from O(W*H) to O(W).
    let columns: Vec<Option<ColumnData>> = (0..w)
        .map(|x| {
            let pc_x = x as f32 - half_w;
            let a_term = (pc_x * pc_x) / f_sq + 1.0;
            let discriminant = four_z0_sq - 4.0 * a_term * c_term;

            if discriminant < 0.0 {
                None // ray misses the cylinder
            } else {
                let zc = (two_z0 + discriminant.sqrt()) / (2.0 * a_term);
                Some(ColumnData {
                    src_x: pc_x * zc / f + half_w,
                    zc_over_f: zc / f,
                })
            }
        })
        .collect();

    let w_clamp = w.saturating_sub(2);  // safe upper bound for x0
    let h_clamp = h.saturating_sub(2);  // safe upper bound for y0

    for y in 0..h {
        let pc_y = y as f32 - half_h;

        for (x, col) in columns.iter().enumerate() {
            let Some(col) = col else { continue };

            let src_x = col.src_x;
            let src_y = pc_y * col.zc_over_f + half_h;

            // Skip if source point is outside the image entirely.
            if src_x < 0.0 || src_x >= wf || src_y < 0.0 || src_y >= hf {
                continue;
            }

            // Bilinear interpolation: clamp the 2×2 neighborhood to valid indices
            let x0 = (src_x.floor() as u32).min(w_clamp);
            let y0 = (src_y.floor() as u32).min(h_clamp);
            let x1 = x0 + 1;
            let y1 = y0 + 1;

            let wx1 = (src_x - x0 as f32).clamp(0.0, 1.0);
            let wx0 = 1.0 - wx1;
            let wy1 = (src_y - y0 as f32).clamp(0.0, 1.0);
            let wy0 = 1.0 - wy1;

            let p00 = img.get_pixel(x0, y0).0[0] as f32;
            let p10 = img.get_pixel(x1, y0).0[0] as f32;
            let p01 = img.get_pixel(x0, y1).0[0] as f32;
            let p11 = img.get_pixel(x1, y1).0[0] as f32;

            let val = p00 * wx0 * wy0 + p10 * wx1 * wy0 + p01 * wx0 * wy1 + p11 * wx1 * wy1;
            output.put_pixel(x as u32, y, Luma([val as u8]));
        }
    }

    output
}
