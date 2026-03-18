use std::sync::Arc;

use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub(crate) enum Error {
    #[error("Cannot determine the image format")]
    Decode(Arc<image::ImageError>),
    #[error("General error: {0}")]
    General(String),
}

impl From<image::ImageError> for Error {
    fn from(error: image::ImageError) -> Self {
        Self::Decode(Arc::new(error))
    }
}
