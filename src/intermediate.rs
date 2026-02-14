#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
use ::image::{DynamicImage, EncodableLayout, GrayImage, Luma, Rgba, imageops};
use iced::{
    Color, ContentFit, Element, Fill, Shadow,
    time::Instant,
    widget::{button, container, float, image, mouse_area, space, stack},
};
use image_debug_utils::{contours::remove_hypotenuse_owned, rect::to_axis_aligned_bounding_box};

use imageproc::{
    contours::{self, BorderType},
    contrast::{ThresholdType, adaptive_threshold, otsu_level, threshold},
    distance_transform::Norm,
    drawing::{draw_hollow_circle_mut, draw_line_segment_mut},
    filter::{gaussian_blur_f32, median_filter},
    geometric_transformations::{Interpolation, rotate_about_center},
    geometry::min_area_rect,
    morphology::close,
};
use rustfft::{FftPlanner, num_complex::Complex};
use sipper::{Straw, sipper};
use std::sync::Arc;

use crate::{
    Message, Preview, error::Error, ui::preview::ResultImg, utils::dynamic_image_to_handle,
};

pub(crate) type EncodedImage = Vec<u8>;

/// Matches `docs/user_guide/debug_interpretation_zh.md`
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    Original,
    Smoothing,        // Step 1
    ScaleCalibration, // Step 2 (Replacs ExclusionMap)
    Binary,           // Step 3 (Texture Patch)
    BinaryFusion,     // Step 4 (Morphology Closing)
    RoiExtraction,    // Step 5 (Morphology / ROI Extraction)
    Reconstruction,   // Step 6 (Find Contours / Reconstructed Surface)
    FinalCount,       // Step 7 (Frequency Analysis / Final Count)
}

#[derive(Clone, Debug)]
pub(crate) struct Intermediate {
    pub(crate) current_step: Step,
    pub(crate) preview: Preview,
    /// Derived from Step 2: Scale Calibration
    pub(crate) pixels_per_mm: Option<f32>,
    /// Carried over context image (e.g., Reconstructed Surface)
    pub(crate) context_image: Option<Arc<DynamicImage>>,
    /// ROI Image (Color, High-Res if available) - Persisted for Step 7 Viz
    pub(crate) roi_image: Option<Arc<DynamicImage>>,
    /// Original High Resolution Image (for FFT)
    pub(crate) original_high_res: Option<Arc<DynamicImage>>,
    /// Extracted from EXIF (FocalLength * Px/Unit). Required for Perspective Correction.
    pub(crate) focal_length_px: Option<f32>,
    /// Persisted coordinate transform for mapping points back to original image
    pub(crate) transform: Option<CoordinateTransform>,
}

#[derive(Clone, Debug)]
pub(crate) struct CoordinateTransform {
    pub bbox_x: u32,
    pub bbox_y: u32,
    pub extract_x: i32,
    pub extract_y: i32,
    pub local_width: u32,
    pub local_height: u32,
    pub angle_rad: f32,
    pub original_width: u32,
    pub original_height: u32,
    pub radius: f32,
    pub focal_length_px: f32,
}

const COIN_RADIUS_MM: f32 = 12.5;

impl Intermediate {
    pub(crate) fn process(self) -> impl Straw<Self, EncodedImage, Error> {
        sipper(async move |mut sender| {
            let image: DynamicImage = self.preview.clone().into();

            // Generate Blurhash for UI transition
            if let Ok(blurhash) = blurhash::encode(
                4,
                3,
                image.width(),
                image.height(),
                image.to_rgba8().as_bytes(),
            ) {
                let _ = sender
                    .send(blurhash::decode(&blurhash, 20, 20, 1.0).unwrap())
                    .await;
            }

            match self.current_step {
                Step::Original => {
                    // Step 1: Smoothing
                    // Doc: Gaussian Smoothing (sigma = 1.0)
                    let smoothed = gaussian_blur_f32(&median_filter(&image.to_rgba8(), 1, 1), 1.0);

                    Ok(Intermediate {
                        current_step: Step::Smoothing,
                        preview: Preview::ready(smoothed.into(), Instant::now()),
                        pixels_per_mm: None, // Not calculated yet
                        context_image: None,
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        focal_length_px: self.focal_length_px,
                        transform: None,
                    })
                }
                Step::Smoothing => {
                    // Step 2: Scale Calibration
                    // Doc: Detect coin (Circularity > 0.85). Calculate pixels_per_mm.
                    let smoothed_luma = image.to_luma8();
                    let (vis_img, px_per_mm) = perform_scale_calibration(&smoothed_luma);

                    Ok(Intermediate {
                        current_step: Step::ScaleCalibration,
                        preview: Preview::ready(vis_img.into(), Instant::now()),
                        pixels_per_mm: px_per_mm,

                        context_image: Some(Arc::new(DynamicImage::ImageLuma8(smoothed_luma))),
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        focal_length_px: self.focal_length_px,
                        transform: None,
                    })
                }
                Step::ScaleCalibration => {
                    // Step 3: Adaptive Thresholding (Texture Patch)
                    // Doc: Use derived R and C.
                    let smoothed = self
                        .context_image
                        .as_ref()
                        .ok_or(Error::General("Missing context".into()))?
                        .as_luma8()
                        .unwrap();
                    let px_per_mm = self
                        .pixels_per_mm
                        .ok_or(Error::General("Scale calibration failed".into()))?;

                    // Param Derivation (Doc Step 1.3)
                    // Adaptive Radius = 1.0 * R_coin (approx 12.5mm)
                    let adaptive_radius = (COIN_RADIUS_MM * px_per_mm).round() as u32;

                    // Derive Contrast C from global standard deviation (Doc: Derived from variance)
                    let std_dev = calculate_std_dev(&smoothed);
                    // Use -C to require pixel > mean + C (Peak Detection)
                    // Heuristic: 0.5 * std_dev is usually a good dynamic contrast for peak vs background
                    let threshold_val = -(std_dev * 0.5) as i32;

                    // Imageproc `adaptive_threshold` uses (pixel > mean - t) -> 255
                    // We pass `threshold_val` (negative). So pixel > mean - (-C) => pixel > mean + C
                    let binary = adaptive_threshold(smoothed, adaptive_radius, threshold_val);

                    // Debug Logs RE-ADDED for Troubleshooting
                    use web_sys::console;
                    console::log_1(
                        &format!(
                            "[Step 3] px/mm: {:.2}, Radius: {}, StdDev: {:.2}, Thresh: {}",
                            px_per_mm, adaptive_radius, std_dev, threshold_val
                        )
                        .into(),
                    );

                    Ok(Intermediate {
                        current_step: Step::Binary,
                        preview: Preview::ready(binary.into(), Instant::now()),
                        pixels_per_mm: self.pixels_per_mm,
                        context_image: self.context_image.clone(),
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        focal_length_px: self.focal_length_px,
                        transform: None,
                    })
                }
                Step::Binary => {
                    // Step 4: Binary Fusion (Morphological Closing)
                    // Doc Step 2.2: B_fused = Close(B, R_morph)
                    // R_morph = 0.15 * R_coin (approx 1.8mm)
                    let binary = image.to_luma8(); // Previous step output
                    let px_per_mm = self
                        .pixels_per_mm
                        .ok_or(Error::General("Missing scale".into()))?;

                    // Doc Update: Needs larger radius to fuse peaks into a skin mask
                    // Adjusted to 0.25 to prevent merging skin with flesh
                    let morph_radius = (0.25 * COIN_RADIUS_MM * px_per_mm).round() as u8;
                    let fused = close(&binary, Norm::L2, morph_radius);

                    Ok(Intermediate {
                        current_step: Step::BinaryFusion,
                        preview: Preview::ready(fused.into(), Instant::now()),
                        pixels_per_mm: self.pixels_per_mm,
                        context_image: self.context_image.clone(), // Keep smoothed image for Step 5 crop
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        focal_length_px: self.focal_length_px,
                        transform: None,
                    })
                }
                Step::BinaryFusion => {
                    // Step 5: ROI Extraction (Morphology / ROI Extraction)
                    // Doc Step 2.3 & 2.4: Physical Area Filter & ROI Selection (Texture Score)
                    let fused = image.to_luma8();
                    let smoothed = self.context_image.as_ref().unwrap().as_luma8().unwrap();
                    let px_per_mm = self
                        .pixels_per_mm
                        .ok_or(Error::General("Missing scale".into()))?;

                    let (roi_img_low_res, roi_rect_low_res) =
                        extract_best_roi(&fused, smoothed, px_per_mm)?;

                    // Extract ROI
                    if let Some(roi_rect_low_res) = roi_rect_low_res {
                        let roi_arc_low_res = Arc::new(DynamicImage::ImageLuma8(roi_img_low_res));

                        // Prepare High-Res ROI if available
                        let mut best_roi = roi_arc_low_res.clone();

                        // Scale factor if identifying on low-res but cropping high-res
                        // We will apply the SAME rotation logic to High Res (scaled).
                        let mut transform = None;

                        if let Some(ref high_res) = self.original_high_res {
                            use web_sys::console;
                            let scale = high_res.width() as f32 / image.width() as f32;

                            // 1. Scale Rotated Rect Params
                            let hr_cx = roi_rect_low_res.cx * scale;
                            let hr_cy = roi_rect_low_res.cy * scale;
                            // Angle matches (rotation is scale invariant)
                            // Size scales
                            let hr_w = (roi_rect_low_res.width * scale).round() as u32;
                            let hr_h = (roi_rect_low_res.height * scale).round() as u32;

                            console::log_1(
                                &format!(
                                    "[Step 5] Extracting High-Res ROI: Center({:.1}, {:.1}), Size({}x{}), Angle({:.1} deg)",
                                    hr_cx, hr_cy, hr_w, hr_h, roi_rect_low_res.angle_rad.to_degrees()
                                )
                                .into(),
                            );

                            // 2. Rotate High Res Image
                            // Optimization: High-res rotation is slow.
                            // Ideally we crop a larger bounding box first?
                            // For simplicity/robustness first: Rotate full image (or context if possible).
                            // Given WASM constraints, full 12MP rotation might be heavy (1-2s).
                            // Let's try direct rotation for now. If slow, optimization:
                            // a. Crop bounding box of the rotated rect (safe margin).
                            // b. Rotate the crop.
                            // c. Crop the final upright rect from center.

                            // Implementing Optimization (a-c):
                            // bounding box radius approx
                            let diag = ((hr_w as f32).powi(2) + (hr_h as f32).powi(2)).sqrt();
                            let safe_r = (diag / 2.0).ceil() as u32;
                            let bbox_x = (hr_cx as i32 - safe_r as i32).max(0) as u32;
                            let bbox_y = (hr_cy as i32 - safe_r as i32).max(0) as u32;
                            let bbox_w = (safe_r * 2).min(high_res.width() - bbox_x);
                            let bbox_h = (safe_r * 2).min(high_res.height() - bbox_y);

                            let local_crop =
                                high_res.crop_imm(bbox_x, bbox_y, bbox_w, bbox_h).to_luma8();

                            // Adjusted center in local crop
                            let local_cx = hr_cx - bbox_x as f32;
                            let local_cy = hr_cy - bbox_y as f32;

                            // Rotate local crop
                            // Note: rotate_about_center keeps image size same.
                            // We need to ensure local crop is large enough to hold rotated content.
                            // Current safe_r strategy (diagonal) should be enough.

                            let rotated_local = rotate_about_center(
                                &local_crop,
                                roi_rect_low_res.angle_rad,
                                Interpolation::Bilinear,
                                Luma([0]),
                            );

                            // Now crop the Upright ROI from `rotated_local`.
                            // The center of rotation `(local_cx, local_cy)` is where our ROI center is.
                            // ROI is `(hr_w, hr_h)` centered at `(local_cx, local_cy)`.
                            let extract_x = (local_cx - hr_w as f32 / 2.0).round() as i32;
                            let extract_y = (local_cy - hr_h as f32 / 2.0).round() as i32;

                            // Safe Crop
                            let final_roi = imageops::crop_imm(
                                &rotated_local,
                                extract_x.max(0) as u32,
                                extract_y.max(0) as u32,
                                hr_w,
                                hr_h,
                            )
                            .to_image();

                            best_roi = Arc::new(DynamicImage::ImageLuma8(final_roi));

                            // Capture Transform
                            transform = Some(CoordinateTransform {
                                bbox_x,
                                bbox_y,
                                extract_x,
                                extract_y,
                                local_width: rotated_local.width(),
                                local_height: rotated_local.height(),
                                angle_rad: roi_rect_low_res.angle_rad,
                                original_width: high_res.width(),
                                original_height: high_res.height(),
                                radius: best_roi.width() as f32 / 2.0, // Will be updated if not valid?
                                focal_length_px: 0.0,                  // Will fill later
                            });
                        }

                        // Perform Perspective Correction
                        // User requirement: Error if focal length is missing
                        use web_sys::console;
                        console::log_1(
                            &format!("[Step 5] Checking Focal Length: {:?}", self.focal_length_px)
                                .into(),
                        );

                        let focal_length = self.focal_length_px.ok_or(Error::General(
                            "Missing EXIF Focal Length. Cannot perform perspective correction."
                                .into(),
                        ))?;

                        // Spatially Adaptive Filtering does not require Unwrap.
                        // We simply pass the Rotated ROI.

                        // Use BEST ROI directly
                        let corrected_arc = best_roi.clone();
                        let radius = best_roi.width() as f32 / 2.0; // Estimate for old transforms (if needed)

                        // Update transform
                        if let Some(t) = &mut transform {
                            t.radius = radius;
                            t.focal_length_px = focal_length;
                        }

                        // Preview: Just Original ROI
                        let preview_combined = best_roi.to_luma8();

                        Ok(Intermediate {
                            current_step: Step::RoiExtraction,
                            preview: Preview::ready(
                                DynamicImage::ImageLuma8(preview_combined).into(),
                                Instant::now(),
                            ),
                            pixels_per_mm: self.pixels_per_mm,
                            context_image: Some(corrected_arc.clone()), // Use CORRECTED ROI for Step 6 input
                            roi_image: Some(corrected_arc), // Use CORRECTED ROI for Step 7 Viz
                            original_high_res: self.original_high_res.clone(),
                            focal_length_px: self.focal_length_px,
                            transform,
                        })
                    } else {
                        // If no ROI found, pass through the original image or an error
                        Err(Error::General("No ROI found in Step 5".into()))
                    }
                }
                Step::RoiExtraction => {
                    // Step 6: Frequency Domain Counting
                    // Input: ROI Image (from roi_image if available, else context_image)
                    let input_roi = if let Some(ref roi) = self.roi_image {
                        roi.clone()
                    } else if let Some(ref ctx) = self.context_image {
                        ctx.clone()
                    } else {
                        Arc::new(image.clone()) // Fallback
                    };

                    // Calculate Px/mm scale
                    let scale = input_roi.width() as f32 / image.width() as f32; // For Resizing UI

                    // Correct Logic: Use ratio of Source Resolution to Preview Resolution for Physics.
                    let scale_factor = if let Some(ref high_res) = self.original_high_res {
                        high_res.width() as f32 / image.width() as f32
                    } else {
                        1.0 // Assume Preview scale if no High Res
                    };
                    let px_per_mm = self.pixels_per_mm.unwrap_or(10.0) * scale_factor;

                    // New Algorithm: Spatially Adaptive Filtering
                    // Replace FFT Reconstruction

                    use crate::adaptive;
                    let reconstructed =
                        adaptive::filter_fruitlets(&input_roi.to_luma8(), px_per_mm);

                    // Visualization: Just the response map
                    // Since we removed spectrum, we don't need side-by-side.
                    let combined_preview = reconstructed.clone();

                    // No spectrum vis
                    // We can reuse spectrum_vis field in Intermediate to store something else or empty?
                    // Intermediate struct definition isn't shown here but logic below uses `reconstructed` and `spectrum_vis`.
                    // Wait, `reconstructed` is used for `context_image`.

                    // We need a dummy spectrum_vis if we don't want to change Intermediate struct layout/logic elsewhere?
                    // Or simply don't use it.
                    // The code below creates `combined_preview` from `reconstructed` and `spectrum_vis`.
                    // I replaced that logic with `let combined_preview = reconstructed.clone();`.

                    // Downscale for preview (keep consistent UI)
                    // If we operated on high-res, reconstructed is high-res.
                    // Scale back to preview size.
                    let preview_img = if scale > 1.1 {
                        DynamicImage::ImageLuma8(combined_preview).resize(
                            image.width(),
                            image.height(),
                            imageops::FilterType::Lanczos3,
                        )
                    } else {
                        DynamicImage::ImageLuma8(combined_preview)
                    };

                    Ok(Intermediate {
                        current_step: Step::Reconstruction,
                        preview: Preview::ready(preview_img.into(), Instant::now()),
                        pixels_per_mm: self.pixels_per_mm, // Keep Low-Res Scale for UI flow? Or update? Better keep original logic unless Step 7 needs HR.
                        context_image: Some(Arc::new(DynamicImage::ImageLuma8(reconstructed))), // Store HR Reconstructed (Egg Crate)
                        roi_image: self.roi_image.clone(), // Pass Color ROI to Step 7
                        original_high_res: self.original_high_res.clone(),
                        focal_length_px: self.focal_length_px,
                        transform: self.transform,
                    })
                }
                Step::Reconstruction => {
                    // Step 7: Final Count
                    // Use High-Res Reconstructed Image from context_image
                    let input_handle = if let Some(ref ctx_img) = self.context_image {
                        ctx_img.clone()
                    } else {
                        Arc::new(image.clone())
                    };

                    let scale = input_handle.width() as f32 / image.width() as f32; // For Resizing UI

                    let scale_factor = if let Some(ref high_res) = self.original_high_res {
                        high_res.width() as f32 / image.width() as f32
                    } else {
                        1.0
                    };
                    let px_per_mm = self.pixels_per_mm.unwrap_or(10.0) * scale_factor;

                    // Decide which image to draw on
                    // If roi_image is available (Color), use it. Otherwise use input_handle (Reconstructed/Gray).
                    // We need to pass the background image to count_fruitlets or handle drawing here.
                    // Let's modify count_fruitlets to take an optional background image?
                    // Or more robustly: count_fruitlets returns the peaks, we draw here.
                    // FOR NOW: count_fruitlets returns a drawn image. We should pass the Color ROI (converted to Dynamic) if available.
                    let viz_bg = if let Some(ref roi) = self.roi_image {
                        roi.clone()
                    } else {
                        input_handle.clone()
                    };

                    let (_count, vis_unwrapped, centers) =
                        count_fruitlets(&input_handle.to_luma8(), &viz_bg, px_per_mm);

                    // Map markers back to Original Image if transform helps
                    let vis_final = if let Some(ref transform) = self.transform {
                        if let Some(ref original_hr) = self.original_high_res {
                            // 1. Create visualization of Original Image (Scaled down)
                            // Target width same as Unwrapped Vis for side-by-side
                            let target_w = vis_unwrapped.width();
                            let scale_factor = target_w as f32 / original_hr.width() as f32;
                            let target_h =
                                (original_hr.height() as f32 * scale_factor).round() as u32;

                            let mut vis_original = original_hr
                                .resize(target_w, target_h, imageops::FilterType::Lanczos3)
                                .to_rgba8();

                            // 2. Map points and Draw
                            let cross_size =
                                (1.5 * px_per_mm * scale_factor).max(3.0).round() as i32;
                            let color = Rgba([0, 255, 0, 255]); // Green for Original

                            for (ux, uy) in centers {
                                // Map (u, v) Unwrapped -> (x, y) Original Source
                                // Note: transform struct has params for High-Res
                                // Our u, v are from input_handle which IS High-Res (or Low Res with corrected scale)
                                // If input_handle is scaled, we need to adjust u, v.
                                // In Step::Reconstruction, input_handle came from context_image which IS reconstructed High-Res.
                                // So u, v are correct.

                                if let Some((sx, sy)) = crate::correction::map_point_back(
                                    ux as f32,
                                    uy as f32,
                                    input_handle.width(), // Unwrapped Width
                                    input_handle.height(),
                                    transform.radius,
                                    transform.focal_length_px,
                                ) {
                                    // Inverse Rotate?
                                    // map_point_back returns coordinates in the ROTATED source image (relative to global frame? No.)
                                    // map_point_back returns (src_x, src_y) which are coordinates in the ROTATED ROI frame (Top-Left 0,0 of Rotated ROI).
                                    // Wait, let's verify map_point_back implementation in correction.rs.
                                    // It returns coordinates relative to the input ROI of cylindrical_unwrap.
                                    // That input ROI was `best_roi` (Rotated & Cropped).

                                    // So (sx, sy) are in `best_roi` frame.
                                    // We need to map `best_roi` frame -> Original Image frame.
                                    // `best_roi` was extracted from `rotated_local` which was extracted from `local_crop` ...
                                    // Transform struct has: bbox_x, bbox_y, extract_x, extract_y, angle_rad, local_width/height.

                                    // 1. Un-crop from `final_roi` (best_roi)
                                    // (sx, sy) is valid.

                                    // 2. Un-crop from `rotated_local`
                                    // The `extract_x` was offset of `final_roi` top-left relative to `rotated_local` center?
                                    // No. `extract_x` was top-left of crop in `rotated_local` coordinates.
                                    let rx = sx + transform.extract_x as f32;
                                    let ry = sy + transform.extract_y as f32;

                                    // 3. Un-rotate
                                    // `rotated_local` was rotated around its center `(w/2, h/2)`.
                                    // Center of rotation in `rotated_local`:
                                    let rcx = transform.local_width as f32 / 2.0;
                                    let rcy = transform.local_height as f32 / 2.0;

                                    let dx = rx - rcx;
                                    let dy = ry - rcy;

                                    // Inverse Rotation (-angle)
                                    let theta = transform.angle_rad;
                                    // Forward: x' = x cos - y sin
                                    // Inverse: x = x' cos + y' sin
                                    // dx, dy are the "prime" coordinates.
                                    let dcx = dx * theta.cos() + dy * theta.sin();
                                    let dcy = -dx * theta.sin() + dy * theta.cos();

                                    // dcx, dcy are relative to center of `local_crop`.
                                    // Center of `local_crop` is `(local_width/2, local_height/2)`?
                                    // No, `rotate_about_center` keeps image size. So center is same.
                                    let lcx = rcx + dcx; // Local Crop X
                                    let lcy = rcy + dcy; // Local Crop Y

                                    // 4. Un-crop from Global
                                    let gx = lcx + transform.bbox_x as f32;
                                    let gy = lcy + transform.bbox_y as f32;

                                    // Draw on Vis Original (Scaled)
                                    let vx = gx * scale_factor;
                                    let vy = gy * scale_factor;

                                    draw_line_segment_mut(
                                        &mut vis_original,
                                        (vx - cross_size as f32, vy - cross_size as f32),
                                        (vx + cross_size as f32, vy + cross_size as f32),
                                        color,
                                    );
                                    draw_line_segment_mut(
                                        &mut vis_original,
                                        (vx - cross_size as f32, vy + cross_size as f32),
                                        (vx + cross_size as f32, vy - cross_size as f32),
                                        color,
                                    );
                                }
                            }

                            // Combine Side-by-Side
                            let combined_w = vis_original.width() + 10 + vis_unwrapped.width();
                            let combined_h = vis_original.height().max(vis_unwrapped.height());
                            let mut combined =
                                DynamicImage::new_rgba8(combined_w, combined_h).to_rgba8();

                            // White background
                            for p in combined.pixels_mut() {
                                *p = Rgba([255, 255, 255, 255]);
                            }

                            imageops::replace(&mut combined, &vis_original, 0, 0);
                            imageops::replace(
                                &mut combined,
                                &vis_unwrapped.to_rgba8(),
                                (vis_original.width() + 10) as i64,
                                0,
                            );

                            DynamicImage::ImageRgba8(combined)
                        } else {
                            vis_unwrapped
                        }
                    } else {
                        vis_unwrapped
                    };

                    // Downscale visualization for preview if needed
                    // Scale back to preview size.
                    let vis = if scale > 1.1 {
                        vis_final.resize(
                            image.width(),
                            image.height(),
                            imageops::FilterType::Lanczos3,
                        )
                    } else {
                        vis_final
                    };

                    Ok(Intermediate {
                        current_step: Step::FinalCount,
                        preview: Preview::Ready {
                            blurhash: None,
                            result_img: Box::new(ResultImg::new(vis.into(), Instant::now())),
                        },
                        pixels_per_mm: self.pixels_per_mm,
                        context_image: None,
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        focal_length_px: self.focal_length_px,
                        transform: self.transform,
                    })
                }
                _ => Ok(self),
            }
        })
    }

    // UI Card rendering remains largely same but updated for Step enum
    pub(crate) fn card(&self, now: Instant) -> Element<'_, Message> {
        let image = {
            let thumbnail: Element<'_, _> = if let Preview::Ready { result_img, .. } = &self.preview
            {
                float(
                    image(dynamic_image_to_handle(&result_img.img))
                        .width(Fill)
                        .content_fit(ContentFit::Contain)
                        .opacity(result_img.fade_in.interpolate(0.0, 1.0, now)),
                )
                .scale(result_img.zoom.interpolate(1.0, 1.1, now))
                .translate(move |bounds, viewport| {
                    bounds.zoom(1.1).offset(&viewport.shrink(10))
                        * result_img.zoom.interpolate(0.0, 1.0, now)
                })
                .style(move |_theme| float::Style {
                    shadow: Shadow {
                        color: Color::BLACK.scale_alpha(result_img.zoom.interpolate(0.0, 1.0, now)),
                        blur_radius: result_img.zoom.interpolate(0.0, 20.0, now),
                        ..Shadow::default()
                    },
                    ..float::Style::default()
                })
                .into()
            } else {
                space::horizontal().into()
            };

            if let Some(blurhash) = self.preview.blurhash(now) {
                let blurhash = image(&blurhash.handle)
                    .width(Fill)
                    .height(Fill)
                    .content_fit(ContentFit::Fill)
                    .opacity(blurhash.fade_in.interpolate(0.0, 1.0, now));

                stack![blurhash, thumbnail].into()
            } else {
                thumbnail
            }
        };

        let card = mouse_area(container(image).style(container::dark))
            .on_enter(Message::ThumbnailHovered(self.current_step.clone(), true))
            .on_exit(Message::ThumbnailHovered(self.current_step.clone(), false));

        let is_result = matches!(self.preview, Preview::Ready { .. });

        button(card)
            .on_press_maybe(is_result.then_some(Message::Open(self.current_step.clone())))
            .padding(0)
            .style(button::text)
            .into()
    }
}

// ================= Algorithm Implementations =================

fn perform_scale_calibration(image: &GrayImage) -> (DynamicImage, Option<f32>) {
    // 1. Scale Calibration
    // Doc: Otsu -> Find Contours -> Highest Circularity (> 0.85)
    let level = otsu_level(image);
    let binary = threshold(image, level, ThresholdType::Binary);
    // Find contours
    let contours = contours::find_contours::<i32>(&binary);

    // Filter & Select
    // Doc: Highest Circularity (> 0.85)
    let mut best_coin: Option<(f32, contours::Contour<i32>)> = None;

    for contour in &contours {
        let area = contour_area(contour);
        if area < 100.0 {
            continue;
        } // Basic noise filter

        let perimeter = contour_perimeter(contour);
        if perimeter == 0.0 {
            continue;
        }

        let circularity = (4.0 * std::f32::consts::PI * area) / (perimeter * perimeter);

        if circularity > 0.85 {
            // Check if better (higher circularity or larger area? Doc says Highest Circularity)
            // But let's also prefer larger objects to avoid small circular noise
            if let Some((best_circ, ref best_c)) = best_coin {
                // Heuristic: If circularity is similar, pick larger. If much better, pick circularity.
                let best_area = contour_area(best_c);
                if circularity > best_circ && area > best_area * 0.5 {
                    best_coin = Some((circularity, contour.clone()));
                } else if area > best_area && circularity > 0.85 {
                    best_coin = Some((circularity, contour.clone()));
                }
            } else {
                best_coin = Some((circularity, contour.clone()));
            }
        }
    }

    let mut vis_img = DynamicImage::ImageLuma8(image.clone()).to_rgba8();
    let mut px_per_mm = None;

    if let Some((_, contour)) = best_coin {
        // Derive pixels_per_mm
        // Doc: pixels_per_mm = Radius_coin_px / 12.5mm
        let area = contour_area(&contour);
        let radius_px = (area / std::f32::consts::PI).sqrt();
        px_per_mm = Some(radius_px / COIN_RADIUS_MM);

        // Visual: Red Box/Circle
        let rect = min_area_rect(&contour.points);
        let bbox = to_axis_aligned_bounding_box(&rect);
        // Draw 4 lines for rect
        let color = Rgba([255, 0, 0, 255]);
        let (x, y, w, h) = (
            bbox.x as f32,
            bbox.y as f32,
            bbox.width as f32,
            bbox.height as f32,
        );
        draw_line_segment_mut(&mut vis_img, (x, y), (x + w, y), color);
        draw_line_segment_mut(&mut vis_img, (x + w, y), (x + w, y + h), color);
        draw_line_segment_mut(&mut vis_img, (x + w, y + h), (x, y + h), color);
        draw_line_segment_mut(&mut vis_img, (x, y + h), (x, y), color);

        // Also draw simple circle approximation
        let cx = bbox.x + bbox.width / 2;
        let cy = bbox.y + bbox.height / 2;
        draw_hollow_circle_mut(
            &mut vis_img,
            (cx as i32, cy as i32),
            radius_px as i32,
            Rgba([255, 0, 0, 255]),
        );
    }

    (vis_img.into(), px_per_mm)
}

#[derive(Clone, Copy, Debug)]
struct RotatedRect {
    cx: f32,
    cy: f32,
    width: f32,     // Upright Width
    height: f32,    // Upright Height
    angle_rad: f32, // Rotation applied to image to make it upright
}

use imageproc::point::Point;

fn get_rotated_rect_info(points: &[Point<i32>]) -> RotatedRect {
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

fn extract_best_roi(
    fused: &GrayImage,
    smoothed: &GrayImage,
    px_per_mm: f32,
) -> Result<(GrayImage, Option<RotatedRect>), Error> {
    use web_sys::console;
    console::log_1(&"[Step 5] Starting extract_best_roi...".into());

    // Step 2b: Physical Area Filter & ROI Selection
    let contours = contours::find_contours::<i32>(fused);
    console::log_1(&format!("[Step 5] find_contours found: {}", contours.len()).into());

    let candidates = remove_hypotenuse_owned(contours, 5.0, Some(BorderType::Outer));

    let coin_area_px = std::f32::consts::PI * (COIN_RADIUS_MM * px_per_mm).powi(2);
    // Relaxed area filter: 0.05 * CoinArea
    let min_area = 0.05 * coin_area_px;

    let mut stats = Vec::new();

    for (i, contour) in candidates.iter().enumerate() {
        let area = contour_area(contour);
        if area < min_area {
            continue;
        }

        // Scoring on Axis-Aligned Crop (Approximation)
        let rect_obj = min_area_rect(&contour.points);
        let alien_rect = to_axis_aligned_bounding_box(&rect_obj);

        let crop = imageops::crop_imm(
            smoothed,
            alien_rect.x as u32,
            alien_rect.y as u32,
            alien_rect.width,
            alien_rect.height,
        )
        .to_image();

        let score = calculate_texture_score(&crop, px_per_mm);

        // We defer Rotation calculation until best is found to save perf
        stats.push((i, area, score, contour));
    }

    // Sort by Score Descending
    stats.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Log Top 5
    console::log_1(&format!("[Step 5] Top Candidates (Total {}):", stats.len()).into());
    for (rank, (i, area, score, _)) in stats.iter().take(5).enumerate() {
        console::log_1(
            &format!(
                "  #{}: Contour {} | Area: {:.2} | Score: {:.4}",
                rank + 1,
                i,
                area,
                score
            )
            .into(),
        );
    }

    if let Some((_, _, _, best_contour)) = stats.first() {
        // Now Perform Rotated Crop
        let rect_obj = min_area_rect(&best_contour.points);
        let r_rect = get_rotated_rect_info(&rect_obj);

        console::log_1(&format!("[Step 5] Rotating Best Contour: {:?}", r_rect).into());

        // Rotate SMOOTHED image for Low-Res ROI
        // NOTE: We should rotate a context around the contour center, not full image?
        // Full image rotation on Low Res (preview) is acceptable (likely < 1MP).

        let rotated_full = rotate_about_center(
            smoothed,
            r_rect.angle_rad,
            Interpolation::Bilinear,
            Luma([0]),
        );

        // Calculate New Center in Rotated Image?
        // rotate_about_center rotates around image center (w/2, h/2).
        // So the Point (cx, cy) moves to new position.
        // Formula:
        // x' = (x - cx) cos T - (y - cy) sin T + cx
        // y' = (x - cx) sin T + (y - cy) cos T + cy
        // Wait, rotate_about_center docs: "Rotates around the center of the image".
        // Center = (w/2, h/2).

        let icx = smoothed.width() as f32 / 2.0;
        let icy = smoothed.height() as f32 / 2.0;
        let theta = r_rect.angle_rad;
        let cos_t = theta.cos();
        let sin_t = theta.sin();

        let dx = r_rect.cx - icx;
        let dy = r_rect.cy - icy;

        let new_cx = dx * cos_t - dy * sin_t + icx;
        let new_cy = dx * sin_t + dy * cos_t + icy;

        // Now crop from rotated_full at (new_cx, new_cy) with (width, height)
        let extract_x = (new_cx - r_rect.width / 2.0).round() as i32;
        let extract_y = (new_cy - r_rect.height / 2.0).round() as i32;

        let best_crop = imageops::crop_imm(
            &rotated_full,
            extract_x.max(0) as u32,
            extract_y.max(0) as u32,
            r_rect.width.round() as u32,
            r_rect.height.round() as u32,
        )
        .to_image();

        Ok((best_crop, Some(r_rect)))
    } else {
        Err(Error::General("No valid ROI found".into()))
    }
}

fn calculate_texture_score(image: &GrayImage, px_per_mm: f32) -> f32 {
    // Doc Step 2.4: Constrained Texture Score
    // Mask energy in range [0.7 * D_target, 1.3 * D_target]
    // D_target = N / P_px = N / (25mm * px_per_mm)
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return 0.0;
    }
    let n = w.min(h) as f32; // Usually FFT dimensions
    // Expected period in pixels
    let p_px = 25.0 * px_per_mm;
    let d_target = if p_px > 0.0 { n / p_px } else { 0.0 };

    if d_target == 0.0 {
        return 0.0;
    }

    let (rows, cols) = (image.height() as usize, image.width() as usize);
    let mut data = Vec::with_capacity(rows * cols);
    for py in 0..rows {
        for px in 0..cols {
            data.push(Complex::new(
                image.get_pixel(px as u32, py as u32)[0] as f32,
                0.0,
            ));
        }
    }

    let spectrum = fft2(data, rows, cols);

    let min_r = 0.7 * d_target;
    let max_r = 1.3 * d_target;

    let mut energy = 0.0;

    for r in 0..rows {
        for c in 0..cols {
            let fr = if r < rows / 2 {
                r as f32
            } else {
                (r as f32) - (rows as f32)
            };
            let fc = if c < cols / 2 {
                c as f32
            } else {
                (c as f32) - (cols as f32)
            };
            let dist = (fr * fr + fc * fc).sqrt();

            if dist >= min_r && dist <= max_r {
                energy += spectrum[r * cols + c].norm();
            }
        }
    }

    // Normalize by Area
    energy / (rows * cols) as f32
}

fn reconstruct_surface(roi: &GrayImage, px_per_mm: f32) -> (GrayImage, GrayImage) {
    // Step 3: Frequency Domain Counting
    // FFT -> Frequency Locking -> Bandpass -> IFFT
    let (rows, cols) = (roi.height() as usize, roi.width() as usize);
    let mut data = Vec::with_capacity(rows * cols);

    // Apply Windowing (Hanning) to reduce leakage before FFT
    let mut roi_values = Vec::with_capacity(rows * cols);
    for py in 0..rows {
        for px in 0..cols {
            let wy =
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * py as f32 / (rows as f32 - 1.0)).cos());
            let wx =
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * px as f32 / (cols as f32 - 1.0)).cos());
            let val = roi.get_pixel(px as u32, py as u32)[0] as f32;
            roi_values.push(val); // Keep raw for reference if needed
            data.push(Complex::new(val * wx * wy, 0.0));
        }
    }

    let spectrum = fft2(data, rows, cols);

    // Frequency Locking (Estimate target frequency based on 25mm spacing)
    // N = Dimension size. P_px = Period in pixels.
    // D_target (Freq Index) = N / P_px
    let n = rows.min(cols) as f32;
    let p_px = 25.0 * px_per_mm; // 25mm spacing
    let d_target = if p_px > 0.0 { n / p_px } else { 10.0 };

    // Bandpass Filter Design
    // Passband: [0.6 * d_target, 1.4 * d_target]
    // Zero out DC and low freqs (< 3.0)
    let min_r = (0.6 * d_target).max(3.0);
    let max_r = 1.4 * d_target;

    use web_sys::console;
    console::log_1(
        &format!(
            "[Step 6] FFT Recon | Px/mm: {:.2}, D_target: {:.2}, Passband: [{:.2}, {:.2}]",
            px_per_mm, d_target, min_r, max_r
        )
        .into(),
    );

    // Filter Logic
    // Create new spectrum with mask applied
    let mut filtered_spectrum = Vec::with_capacity(rows * cols);

    for r in 0..rows {
        for c in 0..cols {
            // Find radial distance from DC (0,0) handling wrapping
            let fr = if r < rows / 2 {
                r as f32
            } else {
                (r as f32) - (rows as f32)
            };
            let fc = if c < cols / 2 {
                c as f32
            } else {
                (c as f32) - (cols as f32)
            };
            let dist = (fr * fr + fc * fc).sqrt();

            let mask = if dist >= min_r && dist <= max_r {
                1.0
            } else {
                0.0
            };

            filtered_spectrum.push(spectrum[r * cols + c] * mask);
        }
    }

    // Inverse FFT
    let reconstructed_complex = ifft2(filtered_spectrum.clone(), rows, cols);

    // Convert to Magnitude and Normalize
    let mut recon_values = Vec::with_capacity(rows * cols);
    let mut valid_values = Vec::new();

    for val_complex in reconstructed_complex.iter() {
        let val = val_complex.norm(); // Magnitude
        recon_values.push(val);
        // Collect statistics (all pixels, since windowing handled edges somewhat)
        // Or strictly mask? Let's just use all nonzero result pixels.
        if val > 1e-5 {
            valid_values.push(val);
        }
    }

    // Robust Normalization (p1 - p99)
    valid_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let count = valid_values.len();
    let (min_v, max_v) = if count > 100 {
        (valid_values[count / 100], valid_values[count * 99 / 100])
    } else if count > 0 {
        (valid_values[0], valid_values[count - 1])
    } else {
        (0.0, 1.0)
    };

    let range = max_v - min_v;
    let mut recon_img = GrayImage::new(cols as u32, rows as u32);

    for y in 0..rows {
        for x in 0..cols {
            // Mask out original background
            if roi.get_pixel(x as u32, y as u32)[0] > 10 {
                let val = recon_values[y * cols + x];
                let norm = if range > 0.0001 {
                    ((val - min_v) / range * 255.0).clamp(0.0, 255.0)
                } else {
                    0.0
                };
                recon_img.put_pixel(x as u32, y as u32, Luma([norm as u8]));
            } else {
                recon_img.put_pixel(x as u32, y as u32, Luma([0]));
            }
        }
    }

    // Visualization of Spectrum (reused from existing helper)
    let spectrum_vis = visualize_spectrum(&spectrum, rows, cols);

    (recon_img, spectrum_vis)
}

fn visualize_spectrum(spectrum: &[Complex<f32>], rows: usize, cols: usize) -> GrayImage {
    let mut vis = GrayImage::new(cols as u32, rows as u32);
    let mut max_log = 0.0f32;
    let mut log_vals = Vec::with_capacity(rows * cols);

    for val in spectrum {
        let mag = val.norm();
        let log_val = (1.0 + mag).ln();
        if log_val > max_log {
            max_log = log_val;
        }
        log_vals.push(log_val);
    }

    if max_log == 0.0 {
        max_log = 1.0;
    }

    for r in 0..rows {
        for c in 0..cols {
            // FFT Shift: Swap quadrants to center (0,0) freq
            let sr = (r + rows / 2) % rows;
            let sc = (c + cols / 2) % cols;

            let val = log_vals[r * cols + c];
            let norm = (val / max_log * 255.0) as u8;
            vis.put_pixel(sc as u32, sr as u32, Luma([norm]));
        }
    }
    vis
}

fn count_fruitlets(
    recon_img: &GrayImage,
    viz_bg: &DynamicImage,
    px_per_mm: f32,
) -> (u32, DynamicImage, Vec<(u32, u32)>) {
    // Step 4: Counting
    // Doc: Dynamic Threshold T = 0.4 * max
    // Doc: Physical NMS Radius = 0.5 * R_coin (approx 6mm)

    let (w, h) = recon_img.dimensions();
    let mut max_val = 0u8;
    for p in recon_img.pixels() {
        max_val = max_val.max(p[0]);
    }

    // Simplified Normalization in count_fruitlets
    // Step 6 (IFFT) produces a robustly normalized image (0-255).
    // The background is BLACK (0) because DC component was removed in Bandpass.
    // Fruitlets are bright white blobs.

    // Dynamic Threshold:
    let mut actual_max = 0u8;
    for p in recon_img.pixels() {
        actual_max = actual_max.max(p[0]);
    }

    // Threshold: 50% of Max Intensity (Increased from 25% to avoid valleys/noise).
    // LoG response is very high contrast, so we can be stricter.
    // If image is very dark (max < 40), it might be empty or failed.
    let threshold = if actual_max > 40 {
        (actual_max as f32 * 0.50) as u8
    } else {
        20 // very low threshold
    };

    let safe_threshold = threshold.max(50); // Minimum brightness to be a fruitlet

    // User Feedback: Fruitlet diameter ~ Coin diameter.
    // Coin radius ~12.5mm => Diameter ~25mm.
    // previous NMS 0.5 * R (~6mm) is too small. Increase to 1.0 * R (~12.5mm) to ensure 1 peak per fruitlet.
    let nms_radius = 1.0 * COIN_RADIUS_MM * px_per_mm;

    use web_sys::console;
    console::log_1(
        &format!(
            "[Step 7] Count: MaxVal {}, T_calc {}, T_used {}, NMS_R {:.1}",
            actual_max, threshold, safe_threshold, nms_radius
        )
        .into(),
    );

    let threshold = safe_threshold;
    let mut centers = Vec::new();
    let bg_luma = viz_bg.to_luma8(); // For masking checks

    // Local Maxima Finding
    // Masking is now handled in Step 6 (reconstruct_surface) which sets invalid areas to 128 (below threshold).
    // So we can cycle through the whole image (minus 1px border for 3x3 check).

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            // Mask Check: If original image is dark (background), ignore.
            // Also check the recon_img value itself.
            if bg_luma.get_pixel(x, y)[0] < 20 {
                continue;
            }

            let val = recon_img.get_pixel(x, y)[0];
            if val < threshold {
                continue;
            }

            // 3x3 Block check for local max
            let mut is_max = true;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    if recon_img.get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32)[0] > val
                    {
                        is_max = false;
                        break;
                    }
                }
            }
            if is_max {
                centers.push((x, y, val));
            }
        }
    }

    // Physical NMS
    // Sort by value descending
    centers.sort_by(|a, b| b.2.cmp(&a.2));

    let mut final_centers = Vec::new();
    let nms_sq = nms_radius * nms_radius;

    for (cx, cy, _) in centers {
        let mut keep = true;
        for (fx, fy) in &final_centers {
            let dist_sq = (cx as f32 - *fx as f32).powi(2) + (cy as f32 - *fy as f32).powi(2);
            if dist_sq < nms_sq {
                keep = false;
                break;
            }
        }
        if keep {
            final_centers.push((cx, cy));
        }
    }

    // Visualization
    let mut result_img = viz_bg.to_rgba8();
    let (w_recon, h_recon) = recon_img.dimensions();
    let (w_viz, h_viz) = result_img.dimensions();

    // Calculate scaling factor between detection map (recon) and visualization image (viz_bg)
    let scale_x = w_viz as f32 / w_recon as f32;
    let scale_y = h_viz as f32 / h_recon as f32;

    // Reduce size: 0.6 * px_per_mm (approx 6mm radius)
    let cross_size = (0.6 * px_per_mm * scale_x).round() as i32;

    for (cx, cy) in final_centers.iter().copied() {
        // Scale coordinates
        let scaled_x = (cx as f32 * scale_x).round();
        let scaled_y = (cy as f32 * scale_y).round();
        let r = cross_size as f32;
        let color = Rgba([255, 0, 0, 255]);
        // let thickness = 3.0; // Implied by loop -1..=1

        for t in -1..=1 {
            let offset = t as f32;
            // Diagonal 1 (\)
            draw_line_segment_mut(
                &mut result_img,
                (scaled_x - r + offset, scaled_y - r),
                (scaled_x + r + offset, scaled_y + r),
                color,
            );
            // Diagonal 2 (/)
            draw_line_segment_mut(
                &mut result_img,
                (scaled_x - r + offset, scaled_y + r),
                (scaled_x + r + offset, scaled_y - r),
                color,
            );
        }
    }

    // Return count, visualization, and centers
    (
        final_centers.len() as u32,
        DynamicImage::ImageRgba8(result_img),
        final_centers.iter().map(|&(x, y)| (x, y)).collect(),
    )
}

// ================= Helpers =================

fn calculate_std_dev(image: &GrayImage) -> f32 {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return 0.0;
    }

    let count = (w * h) as f32;
    let mut sum = 0.0;
    let mut sq_sum = 0.0;

    for p in image.pixels() {
        let val = p[0] as f32;
        sum += val;
        sq_sum += val * val;
    }

    let mean = sum / count;
    let variance = (sq_sum / count) - (mean * mean);
    variance.sqrt()
}

#[allow(clippy::cast_precision_loss)]
fn contour_area(contour: &imageproc::contours::Contour<i32>) -> f32 {
    let points = &contour.points;
    if points.is_empty() {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..points.len() {
        let p1 = points[i];
        let p2 = points[(i + 1) % points.len()];
        area += (p1.x * p2.y - p2.x * p1.y) as f32;
    }
    (area / 2.0).abs()
}

#[allow(clippy::cast_precision_loss)]
fn contour_perimeter(contour: &imageproc::contours::Contour<i32>) -> f32 {
    let points = &contour.points;
    if points.is_empty() {
        return 0.0;
    }
    let mut perimeter = 0.0;
    for i in 0..points.len() {
        let p1 = points[i];
        let p2 = points[(i + 1) % points.len()];
        let dx = (p1.x - p2.x) as f32;
        let dy = (p1.y - p2.y) as f32;
        perimeter += (dx * dx + dy * dy).sqrt();
    }
    perimeter
}

fn fft2(data: Vec<Complex<f32>>, rows: usize, cols: usize) -> Vec<Complex<f32>> {
    let mut planner = FftPlanner::new();
    let fft_row = planner.plan_fft_forward(cols);
    let fft_col = planner.plan_fft_forward(rows);
    let scratch_len = fft_row
        .get_inplace_scratch_len()
        .max(fft_col.get_inplace_scratch_len());
    let mut scratch = vec![Complex::new(0.0, 0.0); scratch_len];
    let mut intermediate = data;
    for r in 0..rows {
        fft_row.process_with_scratch(&mut intermediate[r * cols..(r + 1) * cols], &mut scratch);
    }
    let mut result = vec![Complex::new(0.0, 0.0); rows * cols];
    for c in 0..cols {
        let mut col_data: Vec<_> = (0..rows).map(|r| intermediate[r * cols + c]).collect();
        fft_col.process_with_scratch(&mut col_data, &mut scratch);
        for r in 0..rows {
            result[r * cols + c] = col_data[r];
        }
    }
    result
}

#[allow(clippy::cast_precision_loss)]
#[allow(dead_code)]
fn ifft2(input: Vec<Complex<f32>>, rows: usize, cols: usize) -> Vec<Complex<f32>> {
    let mut planner = FftPlanner::new();
    let fft_row = planner.plan_fft_inverse(cols);
    let fft_col = planner.plan_fft_inverse(rows);
    let scratch_len = fft_row
        .get_inplace_scratch_len()
        .max(fft_col.get_inplace_scratch_len());
    let mut scratch = vec![Complex::new(0.0, 0.0); scratch_len];
    let mut intermediate = input;
    for r in 0..rows {
        fft_row.process_with_scratch(&mut intermediate[r * cols..(r + 1) * cols], &mut scratch);
    }
    let mut result = vec![Complex::new(0.0, 0.0); rows * cols];
    for c in 0..cols {
        let mut col_data: Vec<_> = (0..rows).map(|r| intermediate[r * cols + c]).collect();
        fft_col.process_with_scratch(&mut col_data, &mut scratch);
        for r in 0..rows {
            result[r * cols + c] = col_data[r] / (rows * cols) as f32;
        }
    }
    result
}
