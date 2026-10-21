use crate::Id;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
    #[error(transparent)]
    Loro(#[from] loro::LoroError),
    #[error("{kind} record is invalid: {source}")]
    Record {
        kind: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("project document is invalid: {0}")]
    Schema(String),
    #[error("{kind} {id} was not found")]
    NotFound { kind: &'static str, id: Id },
    #[error("{kind} {id} is still referenced")]
    Referenced { kind: &'static str, id: Id },
    #[error("a generation cannot change the user's {kind} {id}")]
    Authorship { kind: &'static str, id: Id },
    #[error("{0}")]
    Invalid(String),
    #[error("the project changed since this edit was prepared")]
    Stale,
    #[error("a failed edit left the project inconsistent; reopen it")]
    Poisoned,
}
