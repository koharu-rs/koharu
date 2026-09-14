use crate::{BlobId, DocumentId, Revision};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Codec(#[from] revision::Error),
    #[error("not a Koharu project")]
    NotAProject,
    #[error("project is already open for writing")]
    Locked,
    #[error("unsupported Koharu project format {0}")]
    UnsupportedFormat(u32),
    #[error("state belongs to document {state}, not {session}")]
    DocumentMismatch {
        state: DocumentId,
        session: DocumentId,
    },
    #[error("blob {0} was not found")]
    BlobNotFound(BlobId),
    #[error("state generation conflict: expected newer than {current}, got {proposed}")]
    RevisionConflict {
        current: Revision,
        proposed: Revision,
    },
    #[error(transparent)]
    Task(#[from] anyhow::Error),
    #[error("invalid storage data: {0}")]
    Invalid(String),
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}
