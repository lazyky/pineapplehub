pub(crate) mod fast;
pub(crate) mod fruitlet_counting;
pub(crate) mod roi_extraction;
pub(crate) mod scale_calibration;
pub(crate) mod unwrap_metrics;

use ::image::{DynamicImage, EncodableLayout};
use iced::{
    Color, ContentFit, Element, Fill, Shadow,
    time::Instant,
    widget::{button, container, float, image, mouse_area, space, stack},
};
use imageproc::filter::{gaussian_blur_f32, median_filter};
use sipper::{Straw, sipper};

use std::sync::Arc;

use crate::{Message, Preview, error::Error, theme, utils::dynamic_image_to_handle};

use scale_calibration::perform_scale_calibration;

pub(crate) type EncodedImage = Vec<u8>;

/// Matches `docs/user_guide/debug_interpretation_zh.md`
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    Original,
    Smoothing,        // Step 1
    ScaleCalibration, // Step 2 (Replaces ExclusionMap)
    Binary,           // Step 3 (Texture Patch)
    BinaryFusion,     // Step 4 (Morphology Closing)
    RoiExtraction,    // Step 5 (Morphology / ROI Extraction)
    FruitletCounting, // Step 6 (Fruitlet Eye Segmentation & Counting)
}

impl Step {
    /// Human-readable label for UI display.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Original => "0. Original",
            Self::Smoothing => "1. Smoothing",
            Self::ScaleCalibration => "2. Scale Calibration",
            Self::Binary => "3. Texture Patch",
            Self::BinaryFusion => "4. Binary Fusion",
            Self::RoiExtraction => "5. ROI Extraction & Unwrap",
            Self::FruitletCounting => "6. Fruitlet Counting",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FruitletMetrics {
    pub major_length: f32,
    pub minor_length: f32,
    pub volume: f32,
    /// Equatorial fruitlet eye long axis (mm)
    pub a_eq: Option<f32>,
    /// Equatorial fruitlet eye short axis (mm)
    pub b_eq: Option<f32>,
    /// Fruitlet eye orientation angle (rad)
    pub alpha: Option<f32>,
    /// Whole-fruit surface area (mm²), computed via contour integration
    pub surface_area: Option<f32>,
    /// Estimated total fruitlet eye count on the whole fruit
    pub n_total: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct Intermediate {
    pub(crate) current_step: Step,
    pub(crate) preview: Preview,
    /// Derived from Step 2: Scale Calibration
    pub(crate) pixels_per_mm: Option<f32>,
    /// Cached from robust extraction pipeline
    pub(crate) binary_image: Option<Arc<DynamicImage>>,
    pub(crate) fused_image: Option<Arc<DynamicImage>>,
    pub(crate) contours: Option<Arc<Vec<imageproc::contours::Contour<i32>>>>,
    /// Carried over context image (e.g., Reconstructed Surface)
    pub(crate) context_image: Option<Arc<DynamicImage>>,
    /// ROI Image (Color, High-Res if available) - Persisted for Step 7 Viz
    #[allow(dead_code)]
    pub(crate) roi_image: Option<Arc<DynamicImage>>,
    /// Original High Resolution Image (for FFT)
    pub(crate) original_high_res: Option<Arc<DynamicImage>>,
    /// Persisted coordinate transform for mapping points back to original image
    #[allow(dead_code)]
    pub(crate) transform: Option<CoordinateTransform>,
    /// Calculated metrics: major length, minor length, volume, fruitlet counts
    pub(crate) metrics: Option<FruitletMetrics>,
    /// HORIZ_UNWRAP longest contour points (for r_bot extraction)
    pub(crate) horiz_contour: Option<Arc<Vec<imageproc::point::Point<i32>>>>,
    /// HORIZ_UNWRAP bounding rect metrics: (major, minor, angle, cx, cy)
    pub(crate) horiz_rect_metrics: Option<(f32, f32, f32, f32, f32)>,
    /// High-res / preview scale factor
    pub(crate) scale_factor: Option<f32>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct CoordinateTransform {
    pub bbox_x: u32,
    pub bbox_y: u32,
    pub extract_x: i32,
    pub extract_y: i32,
    pub local_width: u32,
    pub local_height: u32,
    pub angle_rad: f32,
    pub radius: f32,
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
                if let Ok(decoded) = blurhash::decode(&blurhash, 20, 20, 1.0) {
                    let _ = sender.send(decoded).await;
                } else {
                    log::error!("Blurhash decode failed");
                }
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
                        binary_image: None,
                        fused_image: None,
                        contours: None,
                        context_image: None,
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        transform: None,
                        metrics: None,
                        horiz_contour: None,
                        horiz_rect_metrics: None,
                        scale_factor: None,
                    })
                }
                Step::Smoothing => {
                    // Step 2: Scale Calibration
                    // Doc: Detect coin (Circularity > 0.85). Calculate pixels_per_mm.
                    let smoothed_luma = image.to_luma8();
                    let (vis_img, px_per_mm, binary, fused, contours) =
                        perform_scale_calibration(&smoothed_luma);

                    Ok(Intermediate {
                        current_step: Step::ScaleCalibration,
                        preview: Preview::ready(vis_img.into(), Instant::now()),
                        pixels_per_mm: px_per_mm,
                        binary_image: Some(Arc::new(DynamicImage::ImageLuma8(binary))),
                        fused_image: Some(Arc::new(DynamicImage::ImageLuma8(fused))),
                        contours: Some(Arc::new(contours)),
                        context_image: Some(Arc::new(DynamicImage::ImageLuma8(smoothed_luma))),
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        transform: None,
                        metrics: None,
                        horiz_contour: None,
                        horiz_rect_metrics: None,
                        scale_factor: None,
                    })
                }
                Step::ScaleCalibration => {
                    // Step 3: Global Otsu Thresholding
                    let binary = self
                        .binary_image
                        .clone()
                        .ok_or(Error::General("Missing binary image".into()))?;

                    Ok(Intermediate {
                        current_step: Step::Binary,
                        preview: Preview::ready((*binary).clone(), Instant::now()),
                        pixels_per_mm: self.pixels_per_mm,
                        binary_image: self.binary_image.clone(),
                        fused_image: self.fused_image.clone(),
                        contours: self.contours.clone(),
                        context_image: self.context_image.clone(),
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        transform: None,
                        metrics: None,
                        horiz_contour: None,
                        horiz_rect_metrics: None,
                        scale_factor: None,
                    })
                }
                Step::Binary => {
                    // Step 4: Binary Fusion (Morphological Closing followed by Opening)
                    let fused = self
                        .fused_image
                        .clone()
                        .ok_or(Error::General("Missing fused image".into()))?;

                    Ok(Intermediate {
                        current_step: Step::BinaryFusion,
                        preview: Preview::ready((*fused).clone(), Instant::now()),
                        pixels_per_mm: self.pixels_per_mm,
                        binary_image: self.binary_image.clone(),
                        fused_image: self.fused_image.clone(),
                        contours: self.contours.clone(),
                        context_image: self.context_image.clone(), // Keep smoothed image for Step 5 crop
                        roi_image: None,
                        original_high_res: self.original_high_res.clone(),
                        transform: None,
                        metrics: None,
                        horiz_contour: None,
                        horiz_rect_metrics: None,
                        scale_factor: None,
                    })
                }
                Step::BinaryFusion => {
                    // Step 5: ROI Extraction & Unwrapping — delegated to unwrap_metrics
                    unwrap_metrics::process_binary_fusion(&self, &image)
                }
                Step::RoiExtraction => {
                    // Step 6: Fruitlet Eye Segmentation & Counting
                    fruitlet_counting::process_fruitlet_counting(&self, &image)
                }
                _ => Ok(self),
            }
        })
    }

    // UI Card rendering remains largely same but updated for Step enum
    pub(crate) fn card(&self, now: Instant) -> Element<'_, Message> {
        use iced::widget::{row, text};

        let image = {
            let thumbnail: Element<'_, _> = if let Preview::Ready { result_img, .. } = &self.preview
            {
                float(
                    image(dynamic_image_to_handle(&result_img.img))
                        .width(Fill)
                        .height(200)
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
                    .height(200)
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

        let mut title_col = iced::widget::column![
            text(self.current_step.label()).size(12),
        ].spacing(2);

        // RoiExtraction gets additional sub-column labels
        if matches!(self.current_step, Step::RoiExtraction) {
            title_col = title_col.push(
                row![
                    text("Vert. Unwrap").size(10).width(Fill).center(),
                    text("Orig. Rect").size(10).width(Fill).center(),
                    text("Horiz. Unwrap").size(10).width(Fill).center(),
                ]
                .width(Fill)
                .spacing(4),
            );
        }

        let title_bar = container(title_col)
            .padding(4)
            .style(container::dark);

        let decorated_card: Element<'_, Message> =
            iced::widget::column![title_bar, card].spacing(0).into();

        let is_result = matches!(self.preview, Preview::Ready { .. });

        button(decorated_card)
            .on_press_maybe(is_result.then_some(Message::Open(self.current_step.clone())))
            .padding(0)
            .style(theme::text_button_style)
            .into()
    }
}
