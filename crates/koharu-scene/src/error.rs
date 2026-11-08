use std::{io, path::PathBuf};

use crate::LayerId;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// What is missing, such as `layer 01a1…`.
    #[error("{0} was not found")]
    NotFound(String),
    #[error("op {op}: {reason}")]
    Invalid { op: usize, reason: String },
    /// Input that is wrong but not tied to one op, such as a tool call.
    #[error("{0}")]
    Rejected(String),
    #[error("op {op}: layer {layer} is locked")]
    Locked { op: usize, layer: LayerId },
    #[error("this Koharu cannot open project version {0}")]
    Version(String),
    #[error("the project file is invalid: {0}")]
    Corrupt(String),
    #[error("{} is open in another window", .0.display())]
    ProjectLocked(PathBuf),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

impl Error {
    pub(crate) fn invalid(op: usize, reason: impl Into<String>) -> Self {
        Self::Invalid {
            op,
            reason: reason.into(),
        }
    }

    pub(crate) fn corrupt(reason: impl std::fmt::Display) -> Self {
        Self::Corrupt(reason.to_string())
    }
}
