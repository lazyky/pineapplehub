use image::{DynamicImage, GrayImage, Luma, imageops};

// --- Configuration Constants ---
const TARGET_PX_PER_MM: f32 = 3.0; // Target resolution for processing
const MIN_SCALE_FACTOR: f32 = 1.0;
const ILLUMINATION_SIGMA_MM: f32 = 25.0; // Sigma for background subtraction (mm)
const FRUITLET_SIGMA_MM: f32 = 2.0; // Sigma for fruitlet detection (mm)
const MIN_FORESHORTENING: f32 = 0.3; // Minimum k value (~70 deg tilt)
const MAX_RADIUS_SIGMA_RATIO: f32 = 3.0; // Kernel radius = 3 * sigma
const MARGIN_SIGMA_RATIO: f32 = 3.5; // Safety margin = 3.5 * sigma
const COMPETITION_SCALES: [f32; 2] = [1.0, 0.9]; // Scales for response competition
const KERNEL_CUTOFF_SQ: f32 = 9.0; // (3 sigma)^2

/// Ellipsoid Model for Fruitlet Filtering
pub struct EllipsoidModel {
    width: f32,
    height: f32,
    center_x: f32,
    center_y: f32,
}

impl EllipsoidModel {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width as f32,
            height: height as f32,
            center_x: width as f32 / 2.0,
            center_y: height as f32 / 2.0,
        }
    }

    /// Returns foreshortening factor k(r) and rotation angle phi
    pub fn get_params(&self, x: u32, y: u32) -> (f32, f32) {
        // Normalized coordinates [-1, 1]
        let u = (x as f32 - self.center_x) / (self.width / 2.0);
        let v = (y as f32 - self.center_y) / (self.height / 2.0);

        let r_sq = u * u + v * v;
        let r = r_sq.sqrt();

        // Clamp r to avoid k -> 0
        let r_clamped = r.min(0.95);

        // Foreshortening k = sqrt(1 - r^2)
        // Set minimum k (0.3 ~ 70 deg tilt)
        let k = (1.0 - r_clamped * r_clamped).sqrt().max(MIN_FORESHORTENING);

        // Rotation angle pointing to center
        let phi = v.atan2(u);

        (k, phi)
    }
}

/// Applies spatially adaptive filtering to enhance fruitlet signals.
/// Returns the response map.
///
/// Includes automatic downscaling optimization.
pub fn filter_fruitlets(image: &GrayImage, px_per_mm: f32) -> GrayImage {
    let (w_orig, h_orig) = image.dimensions();

    // 1. Downscale if necessary
    let (working_img, working_px_per_mm, scale) =
        prepare_image_for_processing(image, px_per_mm, w_orig, h_orig);

    // 2. Illumination Correction (High-Pass)
    let high_pass_img = apply_illumination_correction(&working_img, working_px_per_mm);

    // 3. Adaptive Filtering (LoG)
    let response_map = compute_response_map(&high_pass_img, working_px_per_mm);

    // 4. Restore original resolution
    restore_resolution(response_map, scale, w_orig, h_orig)
}

fn prepare_image_for_processing(
    image: &GrayImage,
    px_per_mm: f32,
    w_orig: u32,
    h_orig: u32,
) -> (GrayImage, f32, f32) {
    // Optimization: Downscale if resolution is too high
    let scale = if px_per_mm > 1.5 * TARGET_PX_PER_MM {
        TARGET_PX_PER_MM / px_per_mm
    } else {
        MIN_SCALE_FACTOR
    };

    if scale < MIN_SCALE_FACTOR {
        let new_w = (w_orig as f32 * scale).round() as u32;
        let new_h = (h_orig as f32 * scale).round() as u32;
        log::info!(
            "[Adaptive] Downscaling input: {}x{} -> {}x{} (Scale {:.2})",
            w_orig,
            h_orig,
            new_w,
            new_h,
            scale
        );

        let dynamic = DynamicImage::ImageLuma8(image.clone());
        (
            dynamic
                .resize_exact(new_w, new_h, imageops::FilterType::Triangle)
                .to_luma8(),
            TARGET_PX_PER_MM,
            scale,
        )
    } else {
        (image.clone(), px_per_mm, 1.0)
    }
}

fn apply_illumination_correction(image: &GrayImage, px_per_mm: f32) -> GrayImage {
    let bg_sigma = ILLUMINATION_SIGMA_MM * px_per_mm;
    log::info!(
        "[Adaptive] Illumination Correction (Sigma={:.1}px)...",
        bg_sigma
    );

    // We use imageproc's gaussian blur
    let background = imageproc::filter::gaussian_blur_f32(image, bg_sigma);

    // Subtract background: HighPass = Background - Original (Positive if Original is darker than background)
    // We expect fruitlet eyes to be darker than local mean.
    let (w, h) = image.dimensions();
    let mut high_pass_img = GrayImage::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let val = image.get_pixel(x, y)[0] as f32;
            let bg = background.get_pixel(x, y)[0] as f32;

            // CHANGE: Detect Dark Spots (Fruitlet Centers/Eyes)
            let diff = bg - val;
            let out = diff.max(0.0).min(255.0) as u8;
            high_pass_img.put_pixel(x, y, Luma([out]));
        }
    }
    high_pass_img
}

fn compute_response_map(image: &GrayImage, px_per_mm: f32) -> GrayImage {
    let (w, h) = image.dimensions();
    let model = EllipsoidModel::new(w, h);

    let sigma_pixels = FRUITLET_SIGMA_MM * px_per_mm;

    // Safety check for kernel size
    if sigma_pixels < 1.0 {
        log::warn!("[Adaptive] Warning: Sigma too small (<1px). Returning raw image.");
        return image.clone();
    }

    let mut response_map = GrayImage::new(w, h);
    let max_sigma = sigma_pixels * 1.0;
    let margin = (MARGIN_SIGMA_RATIO * max_sigma).ceil() as u32;

    let safe_w = w.saturating_sub(margin);
    let safe_h = h.saturating_sub(margin);

    if safe_w < margin || safe_h < margin {
        log::warn!("[Adaptive] Image too small for kernel margin.");
        return image.clone();
    }

    // Optimization: Pre-compute Gaussian/LoG constants
    let inv_2s_long_sq = 1.0 / (2.0 * sigma_pixels * sigma_pixels);
    let radius = (MAX_RADIUS_SIGMA_RATIO * sigma_pixels).ceil() as i32;

    for y in margin..safe_h {
        for x in margin..safe_w {
            let (k_geo, phi) = model.get_params(x, y);
            let mut max_resp = -100.0f32;

            let cos_a = phi.cos();
            let sin_a = phi.sin();

            for &scale_mod in &COMPETITION_SCALES {
                let k_eff = (k_geo * scale_mod).clamp(0.2, 1.0);
                let sigma_short = sigma_pixels * k_eff;
                let inv_2s_short_sq = 1.0 / (2.0 * sigma_short * sigma_short);

                let mut sum_v = 0.0;

                for ky in -radius..=radius {
                    let dy = ky as f32;
                    for kx in -radius..=radius {
                        let dx = kx as f32;

                        let dist_u = dx * cos_a + dy * sin_a;
                        let dist_v = -dx * sin_a + dy * cos_a;

                        let r_sq =
                            dist_u * dist_u * inv_2s_long_sq + dist_v * dist_v * inv_2s_short_sq;

                        if r_sq > KERNEL_CUTOFF_SQ {
                            continue;
                        }

                        let g = (1.0 - r_sq) * (-r_sq).exp();

                        // Unsafe check: coordinates are within bounds by definition of margin/radius loop
                        let val = image.get_pixel((x as i32 + kx) as u32, (y as i32 + ky) as u32)[0]
                            as f32;

                        sum_v += val * g;
                    }
                }

                if sum_v > max_resp {
                    max_resp = sum_v;
                }
            }

            response_map.put_pixel(x, y, Luma([max_resp.clamp(0.0, 255.0) as u8]));
        }
    }

    response_map
}

fn restore_resolution(image: GrayImage, scale: f32, w_orig: u32, h_orig: u32) -> GrayImage {
    if scale < MIN_SCALE_FACTOR {
        log::info!("[Adaptive] Upscaling response...");
        let dynamic = DynamicImage::ImageLuma8(image);
        dynamic
            .resize(w_orig, h_orig, imageops::FilterType::Triangle)
            .to_luma8()
    } else {
        image
    }
}
