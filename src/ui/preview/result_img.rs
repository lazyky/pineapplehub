use ::image::DynamicImage;
use iced::{Animation, animation, time::Instant};

/// The process result image with animation states.
#[derive(Clone, Debug)]
pub struct ResultImg {
    pub img: DynamicImage,
    pub fade_in: Animation<bool>,
    pub zoom: Animation<bool>,
}

impl ResultImg {
    pub fn new(img: DynamicImage, now: Instant) -> Self {
        Self {
            img,
            fade_in: Animation::new(false).slow().go(true, now),
            zoom: Animation::new(false)
                .quick()
                .easing(animation::Easing::EaseInOut),
        }
    }
}
