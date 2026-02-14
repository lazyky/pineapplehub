use image::{GrayImage, ImageBuffer, Luma};

/// Corrects cylindrical curvature using the user's single-axis perspective unwrap algorithm.
///
/// Ports logic from `unwrap.py` / `convert_pt.cpp`.
///
/// # Arguments
/// * `roi` - The input ROI image (grayscale).
/// * `radius` - The radius of the cylinder (typically width / 2).
/// * `focal_length_px` - The camera focal length in pixels.
///
/// # Returns
/// A new `GrayImage` with the curvature corrected.
pub fn cylindrical_unwrap(roi: &GrayImage, radius: f32, focal_length_px: f32) -> GrayImage {
    let (width, height) = roi.dimensions();

    // User's unwrap.py keeps output size same as input.
    // We will respect that for now, assuming the target plane is tangent and bounded same.
    let mut output = ImageBuffer::new(width, height);

    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;

    // Parameters for calculation
    // In unwrap.py: f = w, r = w.
    // Here we strictly follow: f = focal_length_px, r = radius.
    let f = focal_length_px;
    let r = radius;

    // omega in unwrap.py is w/2. This represents the half-width of the image plane.
    // We use the actual image half-width.
    let omega = width as f32 / 2.0;

    // Check validity of sqrt terms
    // z0 = f - sqrt(r^2 - omega^2)
    // If r < omega (Radius < Half Width), this is invalid for the assumed geometry (image wider than sphere diameter).
    // The previous spherical model assumed r = omega.
    // If r < omega, we clamp or error.
    // Given ROI is bounding box, r >= omega usually holds if radius is derived properly.
    // If r is slightly smaller due to rounding, clamp term to 0.
    let term1 = (r * r - omega * omega).max(0.0);
    let z0 = f - term1.sqrt();

    let roi_w = width as i32;
    let roi_h = height as i32;

    for y in 0..height {
        for x in 0..width {
            // Target Point (x, y) relative to center
            let pc_x = x as f32 - center_x;
            let pc_y = y as f32 - center_y;

            // Algorithm from convert_pt.cpp
            // zc = (2*z0 + sqrt(4*z0^2 - 4*(pc.x^2/f^2 + 1)*(z0^2 - r^2))) / (2*(pc.x^2/f^2 + 1))

            let a_term = (pc_x * pc_x) / (f * f) + 1.0;
            let c_term = z0 * z0 - r * r;

            // Discriminant: 4*z0^2 - 4*A*C
            // Simplified: 4 * (z0^2 - A*C)
            let discriminant = 4.0 * z0 * z0 - 4.0 * a_term * c_term;

            if discriminant < 0.0 {
                // Ray misses cylinder?
                // Just copy pixel or black?
                // Using black for out of bounds
                output.put_pixel(x, y, Luma([0]));
                continue;
            }

            let zc = (2.0 * z0 + discriminant.sqrt()) / (2.0 * a_term);

            // final_point (u, v) in source image relative to center
            // u = pc.x * zc / f
            // v = pc.y * zc / f
            // Note: This maps Target X to Source U linearly with depth Zc.
            let src_u = pc_x * zc / f;
            let src_v = pc_y * zc / f;

            let src_x = src_u + center_x;
            let src_y = src_v + center_y;

            // Bilinear Interpolation
            let pixel = interpolate_bilinear(roi, roi_w, roi_h, src_x, src_y);
            output.put_pixel(x, y, pixel);
        }
    }

    output
}

fn interpolate_bilinear(image: &GrayImage, w: i32, h: i32, x: f32, y: f32) -> Luma<u8> {
    if x < 0.0 || y < 0.0 || x > (w - 1) as f32 || y > (h - 1) as f32 {
        return Luma([0]);
    }

    let x_0 = x.floor() as i32;
    let y_0 = y.floor() as i32;
    let x_1 = (x_0 + 1).min(w - 1);
    let y_1 = (y_0 + 1).min(h - 1);

    let dx = x - x_0 as f32;
    let dy = y - y_0 as f32;

    let p00 = image.get_pixel(x_0 as u32, y_0 as u32)[0] as f32;
    let p10 = image.get_pixel(x_1 as u32, y_0 as u32)[0] as f32;
    let p01 = image.get_pixel(x_0 as u32, y_1 as u32)[0] as f32;
    let p11 = image.get_pixel(x_1 as u32, y_1 as u32)[0] as f32;

    let val = (1.0 - dy) * ((1.0 - dx) * p00 + dx * p10) + dy * ((1.0 - dx) * p01 + dx * p11);

    Luma([val.round() as u8])
}

/// Maps a point (x, y) from the unwrapped image back to the source image coordinates.
///
/// This is the inverse of the geometric transformation in `cylindrical_unwrap`.
pub fn map_point_back(
    x: f32,
    y: f32,
    width: u32,
    height: u32,
    radius: f32,
    focal_length_px: f32,
) -> Option<(f32, f32)> {
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let f = focal_length_px;
    let r = radius;

    let pc_x = x - center_x;
    let pc_y = y - center_y;

    let omega = width as f32 / 2.0;
    let term1 = (r * r - omega * omega).max(0.0);
    let z0 = f - term1.sqrt();

    // The forward transform was:
    // u = pc.x * zc / f  => pc.x corresponds to x in unwrapped image (target)
    // v = pc.y * zc / f
    // Wait, let's re-read cylindrical_unwrap.
    // Loop iterates x, y in OUTPUT (Target).
    // pc_x = x - center_x (Target centered)
    // zc = depth at target x, y
    // src_u = pc_x * zc / f (Source centered)
    // src_x = src_u + center_x

    // So if we have a point (x, y) in the OUTPUT (Unwrapped),
    // we just need to re-calculate zc and then src_u, src_v.
    // It's effectively the same math as the forward loop, just for a single point!

    let a_term = (pc_x * pc_x) / (f * f) + 1.0;
    let c_term = z0 * z0 - r * r;
    let discriminant = 4.0 * z0 * z0 - 4.0 * a_term * c_term;

    if discriminant < 0.0 {
        return None;
    }

    let zc = (2.0 * z0 + discriminant.sqrt()) / (2.0 * a_term);

    let src_u = pc_x * zc / f;
    let src_v = pc_y * zc / f;

    let src_x = src_u + center_x;
    let src_y = src_v + center_y;

    Some((src_x, src_y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    #[test]
    fn test_cylindrical_unwrap_basic() {
        let width = 100;
        let height = 100;
        let mut img = GrayImage::new(width, height);

        // Fill with distinct pattern
        for y in 0..height {
            for x in 0..width {
                // Diagonal gradient
                let val = ((x + y) / 2) as u8;
                img.put_pixel(x, y, Luma([val]));
            }
        }

        let radius = 50.0;
        let focal_length = 500.0; // Typical long focal length

        let output = cylindrical_unwrap(&img, radius, focal_length);

        assert_eq!(output.width(), width);
        assert_eq!(output.height(), height);

        // Center check
        let cx = width / 2;
        let cy = height / 2;
        let center_val = output.get_pixel(cx, cy)[0];
        let original_val = img.get_pixel(cx, cy)[0];

        // At center, distortion is minimal, should strictly equal or be very close
        assert!((center_val as i32 - original_val as i32).abs() < 2);
    }
}
