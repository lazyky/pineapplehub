use iced::widget::image::Handle;
use image::DynamicImage;

pub(crate) fn dynamic_image_to_handle(img: &DynamicImage) -> Handle {
    Handle::from_rgba(img.width(), img.height(), img.to_rgba8().into_raw())
}
