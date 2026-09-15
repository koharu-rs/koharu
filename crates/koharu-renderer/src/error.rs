use koharu_scene::{BlobId, EntityId};
use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Scene(#[from] koharu_scene::Error),

    #[error("invalid renderer input: {0}")]
    InvalidInput(String),

    #[error("failed to decode image blob {blob}")]
    Image {
        blob: BlobId,
        #[source]
        source: image::ImageError,
    },

    #[error("font resolution failed for entity {entity}")]
    Font {
        entity: EntityId,
        #[source]
        source: anyhow::Error,
    },

    #[error("font resource operation failed")]
    FontResource(#[source] anyhow::Error),

    #[error("text layout failed for entity {entity}")]
    Layout {
        entity: EntityId,
        #[source]
        source: anyhow::Error,
    },

    #[error("renderer backend failed: {0:#}")]
    Backend(#[source] anyhow::Error),
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }
}
