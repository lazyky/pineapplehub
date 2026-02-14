use crate::Message;
use iced::{
    Animation, Element, Fill, animation, color,
    time::Instant,
    widget::{container, image, mouse_area, opaque, space},
};

#[derive(Clone, Debug)]
pub(crate) struct Viewer {
    image: Option<image::Handle>,
    background_fade_in: Animation<bool>,
    image_fade_in: Animation<bool>,
}

impl Viewer {
    pub(crate) fn new() -> Self {
        Self {
            image: None,
            background_fade_in: Animation::new(false)
                .very_slow()
                .easing(animation::Easing::EaseInOut),
            image_fade_in: Animation::new(false)
                .very_slow()
                .easing(animation::Easing::EaseInOut),
        }
    }

    pub(crate) fn show(&mut self, img: image::Handle, now: Instant) {
        self.image = Some(img);
        self.background_fade_in.go_mut(true, now);
        self.image_fade_in.go_mut(true, now);
    }

    pub(crate) fn close(&mut self, now: Instant) {
        self.background_fade_in.go_mut(false, now);
        self.image_fade_in.go_mut(false, now);
    }

    pub(crate) fn is_animating(&self, now: Instant) -> bool {
        self.background_fade_in.is_animating(now) || self.image_fade_in.is_animating(now)
    }

    pub(crate) fn view(&self, now: Instant) -> Element<'_, Message> {
        let opacity = self.background_fade_in.interpolate(0.0, 0.8, now);

        let image: Element<'_, _> = if let Some(handle) = &self.image {
            image(handle)
                .width(Fill)
                .height(Fill)
                .opacity(self.image_fade_in.interpolate(0.0, 1.0, now))
                .scale(self.image_fade_in.interpolate(1.5, 1.0, now))
                .into()
        } else {
            space::horizontal().into()
        };

        if opacity > 0.0 {
            opaque(
                mouse_area(
                    container(image)
                        .center(Fill)
                        .style(move |_theme| {
                            container::Style::default().background(color!(0x000000, opacity))
                        })
                        .padding(20),
                )
                .on_press(Message::Close),
            )
        } else {
            space::horizontal().into()
        }
    }
}
