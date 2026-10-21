//! The project schema. Each record is stored as a Loro map with one key per
//! field, so edits and undo work field by field. The same types are the views
//! readers receive and the payloads the interface is sent.

use std::{collections::BTreeMap, fmt, str::FromStr};

use loro::LoroBinaryValue;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use specta::Type;
use uuid::Uuid;

pub(crate) trait Record: Serialize + DeserializeOwned {
    const KIND: &'static str;
    /// Fields stored as Loro text instead of a plain value.
    const TEXT_FIELDS: &'static [&'static str] = &[];
}

/// Who produced a value. A generated value may be replaced by any later
/// generation; a user's value only by the user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    User,
    Generated(Generation),
}

impl Origin {
    #[must_use]
    pub const fn is_user(&self) -> bool {
        matches!(self, Self::User)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    /// Namespaced producer, such as `dev.koharu.pipeline.ocr`.
    pub producer: String,
    pub model: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub name: String,
    pub width: f64,
    pub height: f64,
    /// The page's own artwork, an image of this page.
    pub source: Id,
    pub text_mask: Option<Id>,
    pub bubble_mask: Option<Id>,
}

impl Record for Page {
    const KIND: &'static str = "page";
}

/// A detected area of a page. Analysis only: it is never drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub kind: RegionKind,
    pub polygon: Vec<Point>,
    pub label: Option<String>,
    pub detection: Vec<DetectionLabel>,
    pub ocr: Option<Ocr>,
    /// Regions this one lies inside, such as the bubble around a text region.
    pub inside: Vec<Id>,
    pub origin: Origin,
}

impl Record for Region {
    const KIND: &'static str = "region";
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    Text,
    Bubble,
    Panel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct DetectionLabel {
    pub label: String,
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Ocr {
    pub direction: TextDirection,
    pub confidence: Option<f64>,
    pub lines: Vec<[Point; 4]>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TextDirection {
    Auto,
    Horizontal,
    Vertical,
}

/// The words of one text block. Translation never replaces the recognized
/// source, and each keeps who produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Text {
    /// The region the source was recognized from.
    pub region: Option<Id>,
    pub source: String,
    pub source_origin: Origin,
    pub source_language: Option<String>,
    pub translation: String,
    pub translation_origin: Origin,
    pub translation_language: Option<String>,
    pub role: Option<String>,
    pub origin: Origin,
}

impl Record for Text {
    const KIND: &'static str = "text";
    const TEXT_FIELDS: &'static [&'static str] = &["source", "translation"];
}

/// One entry of a page's visible layer stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub opacity: f64,
    pub content: LayerContent,
    pub origin: Origin,
}

impl Record for Layer {
    const KIND: &'static str = "layer";
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerContent {
    Group,
    /// The group that holds a page's text layers.
    TextGroup,
    Text {
        /// The text this layer presents.
        text: Id,
        layout: TextLayout,
        angle: Option<f64>,
        /// The authored frame; without one the layer follows `flows_in`, then
        /// `fit_to`.
        frame: Option<Vec<Point>>,
        fit_to: Option<Id>,
        flows_in: Option<Id>,
        typography: Typography,
    },
    Raster {
        role: RasterRole,
        image: Id,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TextLayout {
    Point,
    Paragraph,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RasterRole {
    Cleanup,
    Paint,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Typography {
    pub font: Option<String>,
    pub weight: Option<u16>,
    pub style: Option<FontStyle>,
    pub size: Option<f64>,
    pub auto_fit: bool,
    pub color: Option<[u8; 4]>,
    pub stroke_color: Option<[u8; 4]>,
    pub stroke_width: Option<f64>,
    pub alignment: Option<TextAlignment>,
    pub writing_mode: Option<WritingMode>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    Start,
    Center,
    End,
    Justify,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WritingMode {
    Horizontal,
    Vertical,
}

/// Stable identity of a page, layer, text, region, or image. It is stored in
/// the document, so it survives undo re-creating the Loro containers behind
/// it.
#[derive(
    Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct Id(Uuid);

impl Id {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for Id {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

impl fmt::Display for Id {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Version of the project document: the number of operations it holds. It
/// only grows, including across reopening.
#[derive(
    Copy, Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Type,
)]
#[serde(transparent)]
pub struct Revision(#[specta(type = f64)] u64);

impl Revision {
    pub(crate) fn of(doc: &loro::LoroDoc) -> Self {
        Self(
            doc.oplog_vv()
                .values()
                .map(|counter| u64::try_from(*counter).unwrap_or_default())
                .sum(),
        )
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One page as readers see it: an owned copy, safe to use away from the
/// session. Images are read separately, so a view never decodes them.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct PageView {
    pub id: Id,
    pub page: Page,
    pub layers: Vec<LayerNode>,
    pub texts: BTreeMap<Id, Text>,
    pub regions: BTreeMap<Id, Region>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct LayerNode {
    pub id: Id,
    pub layer: Layer,
    pub children: Vec<LayerNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct PageSummary {
    pub id: Id,
    pub page: Page,
}

/// An encoded image. Cloning shares the bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: LoroBinaryValue,
}
