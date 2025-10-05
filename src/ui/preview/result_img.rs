use ::image::DynamicImage;
use iced::{Animation, animation, time::Instant, widget::image};

/// The process result image with animation states.
#[derive(Clone, Debug)]
pub(crate) struct ResultImg {
    pub(crate) img: DynamicImage,
    pub(crate) fade_in: Animation<bool>,
    pub(crate) zoom: Animation<bool>,
}

impl ResultImg {
    pub fn new(img: DynamicImage, now: Instant) -> Self {
        Self {
            img: img,
            fade_in: Animation::new(false).slow().go(true, now),
            zoom: Animation::new(false)
                .quick()
                .easing(animation::Easing::EaseInOut),
        }
    }

    // TODO: use utils function
    pub(crate) fn get_handle(&self) -> image::Handle {
        image::Handle::from_rgba(
            self.img.width(),
            self.img.height(),
            self.img.to_rgba8().into_raw(),
        )
    }
}
