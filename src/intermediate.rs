use ::image::{DynamicImage, EncodableLayout};
use iced::{
    Color, ContentFit, Element, Fill, Shadow,
    time::Instant,
    widget::{button, container, float, horizontal_space, image, mouse_area, stack},
};
use sipper::{Straw, sipper};

use crate::{Message, Preview, error::Error};

pub(crate) type EncodedImage = Vec<u8>;

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    Original,
    Gray,
    Final,
}

#[derive(Clone, Debug)]
pub(crate) struct Intermediate {
    pub(crate) current_step: Step,
    pub(crate) preview: Preview,
}

impl Intermediate {
    pub(crate) fn process(self) -> impl Straw<Self, Vec<u8>, Error> {
        sipper(async move |mut sender| {
            let image: DynamicImage = self.preview.into();
            let blurhash = blurhash::encode(
                4,
                3,
                image.width(),
                image.height(),
                &image.to_rgba8().as_bytes(),
            )
            .unwrap();
            let _ = sender
                .send(blurhash::decode(&blurhash, 20, 20, 1.0).unwrap())
                .await;

            match self.current_step {
                Step::Original => Ok(Intermediate {
                    current_step: Step::Gray,
                    preview: Preview::ready(
                        DynamicImage::ImageLuma8(image.clone().to_luma8()),
                        Instant::now(),
                    ),
                }),
                Step::Gray => todo!(),
                Step::Final => todo!(),
            }
        })
    }
    pub(crate) fn card(&self, now: Instant) -> Element<'_, Message> {
        let image = {
            let thumbnail: Element<'_, _> = if let Preview::Ready { result_img, .. } = &self.preview
            {
                float(
                    image(&result_img.get_handle())
                        .width(Fill)
                        .content_fit(ContentFit::Cover)
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
                horizontal_space().into()
            };

            if let Some(blurhash) = self.preview.blurhash(now) {
                let blurhash = image(&blurhash.handle)
                    .width(Fill)
                    .content_fit(ContentFit::Cover)
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

// impl Intermediate {
//     pub(crate) fn gray(&mut self) {
//         self.gray =
//             DynamicImage::ImageLuma8(image::load_from_memory(&self.original).unwrap().to_luma8())
//     }

//     fn blur(&mut self) {
//         // https://docs.rs/image/0.25.6/src/image/imageops/sample.rs.html#1004
//         self.blur = DynamicImage::ImageLuma8(blur(&self.gray.to_luma8(), 7.0));
//     }
// }
