use std::{collections::HashSet, fmt, str::FromStr};

use icu_normalizer::ComposingNormalizerBorrowed;
use revision::revisioned;
use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

use crate::{
    Error, Result,
    component::{Component, ValidationContext},
};

use super::LanguageTag;

pub const GLOSSARY_TEXT_MAX_CHARS: usize = 256;
pub const GLOSSARY_MAX_EXAMPLES: usize = 3;
pub const GLOSSARY_EXAMPLE_MAX_CHARS: usize = 512;

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Glossary {
    pub enabled: bool,
    pub source_language: Option<LanguageTag>,
    pub target_language: Option<LanguageTag>,
    pub source_fingerprint: Option<String>,
    pub entries: Vec<GlossaryEntry>,
}

impl Component for Glossary {
    const KIND: &'static str = "dev.koharu.glossary";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        if let Some(language) = &self.source_language {
            language.validate()?;
        }
        if let Some(language) = &self.target_language {
            language.validate()?;
        }

        let mut ids = HashSet::with_capacity(self.entries.len());
        let mut sources = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            entry.validate()?;
            if !ids.insert(entry.id) {
                return Err(Error::invalid("glossary entry IDs contain duplicates"));
            }
            if !sources.insert((normalize_glossary_source(&entry.source), entry.kind)) {
                return Err(Error::invalid(
                    "glossary source and kind combinations contain duplicates",
                ));
            }
        }
        Ok(())
    }
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct GlossaryEntry {
    pub id: GlossaryEntryId,
    pub source: String,
    pub translation: Option<String>,
    pub kind: GlossaryKind,
    pub enabled: bool,
    pub confidence: Option<f32>,
    pub occurrence_count: u32,
    pub examples: Vec<String>,
    pub source_origin: GlossaryValueOrigin,
    pub translation_origin: Option<GlossaryValueOrigin>,
    pub present_in_last_scan: bool,
}

impl GlossaryEntry {
    fn validate(&self) -> Result<()> {
        validate_text(&self.source, GLOSSARY_TEXT_MAX_CHARS, false, "source")?;
        if let Some(translation) = &self.translation {
            validate_text(translation, GLOSSARY_TEXT_MAX_CHARS, true, "translation")?;
        }
        if self.examples.len() > GLOSSARY_MAX_EXAMPLES {
            return Err(Error::invalid("glossary entry has too many examples"));
        }
        for example in &self.examples {
            validate_text(example, GLOSSARY_EXAMPLE_MAX_CHARS, true, "example")?;
        }
        if self
            .confidence
            .is_some_and(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err(Error::invalid("glossary confidence is invalid"));
        }
        Ok(())
    }
}

#[revisioned(revision = 1)]
#[derive(
    Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
#[specta(transparent)]
pub struct GlossaryEntryId(#[specta(type = String)] Uuid);

impl GlossaryEntryId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for GlossaryEntryId {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for GlossaryEntryId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

impl fmt::Display for GlossaryEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[revisioned(revision = 1)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GlossaryKind {
    Person,
    Place,
    Organization,
    Item,
    Ability,
    Term,
    Other,
}

#[revisioned(revision = 1)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GlossaryValueOrigin {
    Detected,
    Automatic,
    User,
    Imported,
}

#[must_use]
pub fn normalize_glossary_source(source: &str) -> String {
    let normalized = ComposingNormalizerBorrowed::new_nfkc()
        .normalize_iter(source.chars())
        .collect::<String>();
    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn validate_text(value: &str, max_chars: usize, allow_empty: bool, field: &str) -> Result<()> {
    let valid_length = value.chars().count() <= max_chars;
    let has_prohibited_control = value.chars().any(char::is_control);
    let empty = normalize_glossary_source(value).is_empty();
    if valid_length && !has_prohibited_control && (allow_empty || !empty) {
        Ok(())
    } else {
        Err(Error::invalid(format!("glossary {field} is invalid")))
    }
}
