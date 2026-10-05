use std::{fmt, str::FromStr};

use revision::{DeserializeRevisioned, Revisioned, SerializeRevisioned, revisioned};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    Error, Result,
    component::{Component, ValidationContext},
    id::validate_namespaced,
};

use super::{Authored, Origin};

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct LanguageTag(String);

impl LanguageTag {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let valid = (2..=63).contains(&value.len())
            && !value.starts_with('-')
            && !value.ends_with('-')
            && value.split('-').all(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(Error::invalid(format!("invalid language tag: {value}")))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<()> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

impl FromStr for LanguageTag {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct SourceText {
    pub text: Authored<String>,
    pub language: Option<LanguageTag>,
}

/// Marks an entity as semantic text content, independent from source analysis
/// and visual presentation.
#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct TextContent {
    pub origin: Origin,
}

impl Component for TextContent {
    const KIND: &'static str = "dev.koharu.text.content";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        self.origin.validate()
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }
}

impl Revisioned for SourceText {
    fn revision() -> u16 {
        1
    }
}

impl SerializeRevisioned for SourceText {
    fn serialize_revisioned<W: std::io::Write>(
        &self,
        writer: &mut W,
    ) -> std::result::Result<(), revision::Error> {
        self.text.serialize_revisioned(writer)?;
        self.language.serialize_revisioned(writer)
    }
}

impl DeserializeRevisioned for SourceText {
    fn deserialize_revisioned<R: std::io::Read>(
        reader: &mut R,
    ) -> std::result::Result<Self, revision::Error> {
        Ok(Self {
            text: Authored::deserialize_revisioned(reader)?,
            language: Option::deserialize_revisioned(reader)?,
        })
    }
}

impl Component for SourceText {
    const KIND: &'static str = "dev.koharu.text.source";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        if let Some(language) = &self.language {
            language.validate()?;
        }
        validate_authored_text(&self.text)
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.text.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.text.origin = origin;
        true
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Translation {
    pub text: Authored<String>,
    pub language: Option<LanguageTag>,
}

impl Revisioned for Translation {
    fn revision() -> u16 {
        1
    }
}

impl SerializeRevisioned for Translation {
    fn serialize_revisioned<W: std::io::Write>(
        &self,
        writer: &mut W,
    ) -> std::result::Result<(), revision::Error> {
        self.text.serialize_revisioned(writer)?;
        self.language.serialize_revisioned(writer)
    }
}

impl DeserializeRevisioned for Translation {
    fn deserialize_revisioned<R: std::io::Read>(
        reader: &mut R,
    ) -> std::result::Result<Self, revision::Error> {
        Ok(Self {
            text: Authored::deserialize_revisioned(reader)?,
            language: Option::deserialize_revisioned(reader)?,
        })
    }
}

impl Component for Translation {
    const KIND: &'static str = "dev.koharu.text.translation";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        validate_authored_text(&self.text)?;
        if let Some(language) = &self.language {
            language.validate()?;
        }
        Ok(())
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.text.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.text.origin = origin;
        true
    }
}

fn validate_authored_text(value: &Authored<String>) -> Result<()> {
    if value.value.len() > 16 * 1024 * 1024 || value.value.contains('\0') {
        return Err(Error::invalid("text is too large or contains NUL"));
    }
    value.origin.validate()
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct TextRole {
    pub origin: Origin,
    pub role: String,
}

impl TextRole {
    /// A bubble holds the text: someone in the scene is saying it.
    pub const DIALOGUE: &'static str = "dev.koharu.text.dialogue";

    /// No bubble holds the text: narration boxes, signs, and lettering drawn as art.
    pub const FREE_TEXT: &'static str = "dev.koharu.text.free-text";
}

impl Component for TextRole {
    const KIND: &'static str = "dev.koharu.text.role";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        self.origin.validate()?;
        validate_namespaced(&self.role, "text role")
    }

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }
}
