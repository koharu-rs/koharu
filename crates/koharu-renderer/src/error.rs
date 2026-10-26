use koharu_scene::{Hash, LayerId};
use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Scene(#[from] koharu_scene::Error),

    #[error("invalid renderer input: {0}")]
    Invalid(String),

    #[error("file {file} is not a supported image")]
    Image {
        file: Hash,
        #[source]
        source: image::ImageError,
    },

    #[error("text layer {layer} couldn't be laid out")]
    Text {
        layer: LayerId,
        #[source]
        source: anyhow::Error,
    },

    #[error("font resources failed")]
    Fonts(#[source] anyhow::Error),

    #[error("the GPU renderer failed")]
    Backend(#[source] anyhow::Error),
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}
