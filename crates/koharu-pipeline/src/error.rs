use bytesize::ByteSize;
use thiserror::Error;

use crate::Stage;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Scene(#[from] koharu_scene::Error),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error(
        "couldn't load {model} with {free} free on {device}, libtorch holding {reserved} with \
         {active} of it in use"
    )]
    Load {
        model: String,
        device: String,
        free: ByteSize,
        reserved: ByteSize,
        active: ByteSize,
        #[source]
        source: anyhow::Error,
    },

    #[error("{stage} failed")]
    Stage {
        stage: Stage,
        #[source]
        source: anyhow::Error,
    },
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}
