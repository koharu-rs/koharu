//! Ids and hashes.

use derive_more::{Display, From, FromStr};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use specta::Type;
pub use uuid::Uuid;

/// A project, stored once when it is created.
#[derive(Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
#[derive(Clone, Copy, Debug, Display, Eq, FromStr, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectId(pub Uuid);

/// A page; stored, so it survives undo.
#[derive(Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
#[derive(Clone, Copy, Debug, Display, Eq, FromStr, Hash, Ord, PartialEq, PartialOrd)]
pub struct PageId(pub Uuid);

/// A layer; stored, so it survives undo.
#[derive(Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
#[derive(Clone, Copy, Debug, Display, Eq, FromStr, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayerId(pub Uuid);

/// An existing page or layer id, or `$label` for one created earlier in the
/// same edit.
#[derive(Clone, Debug, Display, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(try_from = "String")]
pub struct Ref(String);

impl TryFrom<String> for Ref {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        let valid = match s.strip_prefix('$') {
            Some(label) => {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            }
            None => Uuid::parse_str(&s).is_ok(),
        };
        if valid {
            Ok(Self(s))
        } else {
            Err(format!("{s:?} is neither an id nor a $label"))
        }
    }
}

impl Ref {
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.0.strip_prefix('$')
    }

    #[must_use]
    pub fn uuid(&self) -> Option<Uuid> {
        Uuid::parse_str(&self.0).ok()
    }
}

impl From<PageId> for Ref {
    fn from(id: PageId) -> Self {
        Self(id.to_string())
    }
}

impl From<LayerId> for Ref {
    fn from(id: LayerId) -> Self {
        Self(id.to_string())
    }
}

/// A BLAKE3 hash, as 64 hex digits: of an imported file's bytes, which names
/// the file, or of a tile's.
#[derive(
    Clone,
    Copy,
    Debug,
    Display,
    Eq,
    From,
    FromStr,
    Hash,
    PartialEq,
    SerializeDisplay,
    DeserializeFromStr,
    Type,
    JsonSchema,
)]
pub struct Hash(
    #[specta(type = String)]
    #[schemars(with = "String")]
    blake3::Hash,
);

impl Hash {
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(blake3::Hash::from_bytes(bytes))
    }
}

/// The document's version: new with each edit, and restored by undo and redo.
/// Within a session, equal revisions are the same document; they have no order,
/// so they are not compared.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
pub struct Revision(pub(crate) String);

impl Revision {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7().to_string())
    }
}
