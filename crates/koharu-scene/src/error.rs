use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::{LayerId, PointerBuf, ProjectId, Read};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Kind {
    Page,
    Layer,
    Style,
    File,
    Label,
}

/// Its code is the variant's name in snake case.
#[derive(Debug, thiserror::Error, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Error {
    #[error("{kind} {id} was not found")]
    NotFound { kind: Kind, id: String },
    #[error("{}{}{reason}", op.map(|op| format!("op {op}: ")).unwrap_or_default(), path.as_ref().map(|path| format!("at {:?}: ", path.as_str())).unwrap_or_default())]
    Invalid {
        op: Option<usize>,
        path: Option<PointerBuf>,
        reason: String,
    },
    #[error("op {op}: layer {layer} is locked")]
    Locked { op: usize, layer: LayerId },
    #[error("layer {} changed at {:?} since it was read", read.layer, read.path.as_str())]
    Stale { read: Read },
    #[error("this edit is for project {got}, but project {expected} is open")]
    WrongProject { expected: ProjectId, got: ProjectId },
    #[error("this Koharu cannot open project version {0}")]
    Version(String),
    #[error("the project document is invalid: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Storage(#[from] koharu_storage::Error),
    #[error(transparent)]
    Loro(#[from] loro::LoroError),
    #[error(transparent)]
    #[strum(serialize = "loro")]
    Export(#[from] loro::LoroEncodeError),
    #[error("an earlier edit failed partway, so the project can't be edited or saved; reopen it")]
    Poisoned,
}

impl Error {
    pub(crate) fn invalid(op: usize, reason: impl Into<String>) -> Self {
        Self::Invalid {
            op: Some(op),
            path: None,
            reason: reason.into(),
        }
    }

    /// Input that is wrong but not tied to one op, such as a tool call.
    pub(crate) fn rejected(reason: impl Into<String>) -> Self {
        Self::Invalid {
            op: None,
            path: None,
            reason: reason.into(),
        }
    }

    pub(crate) fn not_found(kind: Kind, id: impl ToString) -> Self {
        Self::NotFound {
            kind,
            id: id.to_string(),
        }
    }

    pub(crate) fn corrupt(reason: impl std::fmt::Display) -> Self {
        Self::Corrupt(reason.to_string())
    }
}

/// Errors reach the UI and agents as `{ "error": code, "message": ..., ... }`
/// with the fields an agent needs to correct itself.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("error", <&str>::from(self))?;
        map.serialize_entry("message", &self.to_string())?;
        match self {
            Self::NotFound { kind, id } => {
                map.serialize_entry("kind", &kind.to_string())?;
                map.serialize_entry("id", id)?;
            }
            Self::Invalid { op, path, .. } => {
                map.serialize_entry("op", op)?;
                map.serialize_entry("path", path)?;
            }
            Self::Locked { op, layer } => {
                map.serialize_entry("op", op)?;
                map.serialize_entry("layer", layer)?;
            }
            Self::Stale { read } => map.serialize_entry("read", read)?,
            _ => {}
        }
        map.end()
    }
}
