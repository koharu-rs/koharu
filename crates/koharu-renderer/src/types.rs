//! Shared public value types for renderer entry points.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontSource {
    System,
    Bundled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontFamily {
    pub name: String,
    pub metadata: FontMetadata,
    pub sources: Vec<FontSource>,
    pub faces: Vec<FontFace>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontMetadata {
    pub primary_script: Option<String>,
    pub scripts: Vec<String>,
    pub languages: Vec<String>,
    pub category: Option<String>,
    pub classifications: Vec<String>,
    pub use_cases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontRange {
    pub minimum: u16,
    pub maximum: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontFace {
    pub post_script_name: String,
    pub weight: u16,
    pub weight_range: Option<FontRange>,
    pub style: FontStyle,
}
