pub(crate) mod result_img;

use ::image::DynamicImage;
use iced::{
    Animation, animation,
    time::{Instant, milliseconds},
    widget::image,
};
pub use result_img::ResultImg;

use crate::EncodedImage;

#[derive(Clone, Debug)]
pub(crate) struct Blurhash {
    pub(crate) handle: image::Handle,
    pub(crate) fade_in: Animation<bool>,
}

/// The `Intermediate` preview state
#[derive(Clone, Debug)]
pub(crate) enum Preview {
    /// The `Intermediate` is still being processed
    Processing { blurhash: Blurhash },
    /// The `Intermediate` is ready to be displayed
    Ready {
        blurhash: Option<Blurhash>,
        result_img: Box<ResultImg>,
    },
}

// TODO: fix overflow issue
// impl From<Preview> for DynamicImage {
//     fn from(preview: Preview) -> Self {
//         unsafe {
//             let ptr = &preview as *const Preview as *const ResultImg;
//             (*ptr).img.clone()
//         }
//     }
// }

impl From<Preview> for DynamicImage {
    fn from(preview: Preview) -> Self {
        match preview {
            Preview::Ready { result_img, .. } => result_img.img.clone(),

            Preview::Processing { .. } => {
                panic!("Cannot convert Processing preview to DynamicImage");
            }
        }
    }
}

impl Preview {
    pub(crate) fn loading(img: EncodedImage, now: Instant) -> Self {
        Self::Processing {
            blurhash: Blurhash {
                fade_in: Animation::new(false)
                    .duration(milliseconds(700))
                    .easing(animation::Easing::EaseIn)
                    .go(true, now),
                handle: image::Handle::from_rgba(20, 20, img),
            },
        }
    }

    pub(crate) fn ready(img: DynamicImage, now: Instant) -> Self {
        Self::Ready {
            blurhash: None,
            result_img: Box::new(ResultImg::new(img, now)),
        }
    }

    pub(crate) fn toggle_zoom(&mut self, enabled: bool, now: Instant) {
        if let Self::Ready {
            result_img: thumbnail,
            ..
        } = self
        {
            thumbnail.zoom.go_mut(enabled, now);
        }
    }

    pub(crate) fn is_animating(&self, now: Instant) -> bool {
        match &self {
            Self::Processing { blurhash } => blurhash.fade_in.is_animating(now),
            Self::Ready {
                result_img: thumbnail,
                ..
            } => thumbnail.fade_in.is_animating(now) || thumbnail.zoom.is_animating(now),
        }
    }

    pub(crate) fn blurhash(&self, now: Instant) -> Option<&Blurhash> {
        match self {
            Self::Processing { blurhash, .. } => Some(blurhash),
            Self::Ready {
                blurhash: Some(blurhash),
                result_img: thumbnail,
                ..
            } if thumbnail.fade_in.is_animating(now) => Some(blurhash),
            Self::Ready { .. } => None,
        }
    }
}
