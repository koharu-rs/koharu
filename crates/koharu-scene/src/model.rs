//! Public types: ids, values, layers, provenance, and views. Every type here
//! serializes the way commands and tools see it.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub use crate::color::Color;
pub use icu_locale_core::LanguageIdentifier;
pub use jsonptr::{Pointer, PointerBuf};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use uuid::Uuid;

/// A text layer's text, which is kept apart from its other properties.
pub(crate) const CONTENT: &Pointer = Pointer::from_static("/content");

macro_rules! uuid_ids {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd,
            Serialize, Deserialize, Type, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse().map(Self)
            }
        }
    )*};
}

uuid_ids! {
    /// A project, stored once when it is created.
    ProjectId,
    /// A page; stored, so it survives undo.
    PageId,
    /// A layer; stored, so it survives undo.
    LayerId,
}

/// An existing page or layer id, or `$label` for one created earlier in the
/// same edit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
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
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
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

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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

/// SVG path data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(try_from = "String")]
pub struct Path(String);

impl TryFrom<String> for Path {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        kurbo::BezPath::from_svg(&s).map_err(|e| format!("invalid path data: {e}"))?;
        Ok(Self(s))
    }
}

impl Path {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A BLAKE3 hash, as 64 lowercase hex digits: of a value read, or of an
/// imported file's bytes, which names the file.
#[derive(
    Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, Type, JsonSchema,
)]
#[serde(try_from = "String")]
pub struct Hash(String);

impl TryFrom<String> for Hash {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(s))
        } else {
            Err(format!("{s:?} is not a BLAKE3 hash"))
        }
    }
}

impl Hash {
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        blake3::hash(bytes).into()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<blake3::Hash> for Hash {
    fn from(hash: blake3::Hash) -> Self {
        Self(hash.to_hex().to_string())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The document's version, as opaque Loro frontiers. Equal revisions are the
/// same document; frontiers have no order, so revisions are not compared.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
pub struct Revision(pub(crate) String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Offset {
    pub x: i32,
    pub y: i32,
}

/// A pixel rectangle, for pixel writes and reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A rectangle in page pixels, for text frames.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A 2D affine `[[a, b, c], [d, e, f]]`, mapping `(x, y)` to
/// `(a x + b y + c, d x + e y + f)`, like Figma's `relativeTransform`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
pub struct Transform(pub [[f64; 3]; 2]);

impl Default for Transform {
    fn default() -> Self {
        Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
    }
}

impl From<Transform> for kurbo::Affine {
    fn from(Transform([[a, b, c], [d, e, f]]): Transform) -> Self {
        Self::new([a, d, b, e, c, f])
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// Groups only: children blend with what is below the group.
    PassThrough,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    #[default]
    Add,
    Subtract,
    Intersect,
}

/// Where a text lays out. Externally tagged, so a merge patch can switch
/// variants: `{"frame": {"path": {"path": "M…"}, "box": null}}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Frame {
    /// Point text: the line grows as it is typed.
    #[default]
    Point,
    /// Paragraph text wrapping inside a box, in the text's own coordinates.
    Box { bounds: Bounds },
    /// Text filling a closed path, in the text's own coordinates.
    Path { path: Path },
}

/// What a mask is made of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskContent {
    Pixels { offset: Offset },
    Path { path: Path },
}

impl Default for MaskContent {
    fn default() -> Self {
        Self::Pixels {
            offset: Offset::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    #[serde(with = "crate::color::array")]
    #[specta(type = [f32; 4])]
    #[schemars(with = "[f32; 4]")]
    pub color: Color,
    pub width: f64,
}

/// Character values; each is also a mark key on a span, and a mark wins for
/// its span.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CharStyle {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub weight: Option<u16>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default, with = "crate::color::option")]
    #[specta(type = Option<[f32; 4]>)]
    #[schemars(with = "Option<[f32; 4]>")]
    pub fill: Option<Color>,
    #[serde(default, with = "crate::color::option")]
    #[specta(type = Option<[f32; 4]>)]
    #[schemars(with = "Option<[f32; 4]>")]
    pub stroke: Option<Color>,
    #[serde(default)]
    pub stroke_width: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    #[serde(default)]
    pub chars: CharStyle,
    /// A paragraph key: style only, never a mark.
    #[serde(default)]
    pub align: Align,
}

/// Text as the inserts of a Loro text delta: spans in order, each with its
/// marks, so no mark can fall outside the text. Deserializes from a plain
/// string too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(from = "RichTextInput")]
pub struct RichText(pub Vec<Span>);

/// A string with its character marks: one insert of a Loro text delta.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub insert: String,
    #[serde(default, skip_serializing_if = "unmarked")]
    pub attributes: CharStyle,
}

fn unmarked(style: &CharStyle) -> bool {
    *style == CharStyle::default()
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum RichTextInput {
    Plain(String),
    Spans(Vec<Span>),
}

impl From<RichTextInput> for RichText {
    fn from(input: RichTextInput) -> Self {
        match input {
            RichTextInput::Plain(text) => Self::from(text.as_str()),
            RichTextInput::Spans(spans) => Self(spans),
        }
    }
}

impl RichText {
    /// The plain text.
    #[must_use]
    pub fn text(&self) -> String {
        self.0.iter().map(|span| span.insert.as_str()).collect()
    }
}

impl From<&str> for RichText {
    fn from(text: &str) -> Self {
        Self(match text.is_empty() {
            true => Vec::new(),
            false => vec![Span {
                insert: text.to_owned(),
                attributes: CharStyle::default(),
            }],
        })
    }
}

/// Namespace, then key, then value. The renderer never reads attributes.
pub type Attributes = BTreeMap<String, BTreeMap<String, Value>>;

fn visible() -> bool {
    true
}

fn opaque() -> f32 {
    1.0
}

fn pass_through() -> Blend {
    Blend::PassThrough
}

fn und() -> LanguageIdentifier {
    LanguageIdentifier::UNKNOWN
}

/// A folder of layers, drawn first to last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// 0 to 1.
    #[serde(default = "opaque")]
    pub opacity: f32,
    #[serde(default = "pass_through")]
    pub blend: Blend,
    #[serde(default)]
    pub attributes: Attributes,
}

/// An imported file, kept byte for byte.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// 0 to 1.
    #[serde(default = "opaque")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: Blend,
    #[serde(default)]
    pub attributes: Attributes,
    pub file: Hash,
    #[serde(default)]
    pub transform: Transform,
}

/// Editable RGBA pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RasterLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// 0 to 1.
    #[serde(default = "opaque")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: Blend,
    #[serde(default)]
    pub attributes: Attributes,
    #[serde(default)]
    pub offset: Offset,
}

/// A vector path with an optional fill and stroke, in page coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// 0 to 1.
    #[serde(default = "opaque")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: Blend,
    #[serde(default)]
    pub attributes: Attributes,
    pub path: Path,
    #[serde(default)]
    pub fill_rule: FillRule,
    #[serde(default, with = "crate::color::option")]
    #[specta(type = Option<[f32; 4]>)]
    #[schemars(with = "Option<[f32; 4]>")]
    pub fill: Option<Color>,
    #[serde(default)]
    pub stroke: Option<Stroke>,
}

/// Styled text in a frame. `content` is set when the layer is added; after
/// that only the text ops change it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// 0 to 1.
    #[serde(default = "opaque")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: Blend,
    #[serde(default)]
    pub attributes: Attributes,
    #[serde(default)]
    pub content: RichText,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default)]
    pub frame: Frame,
    #[serde(default)]
    pub direction: Direction,
    /// A BCP 47 tag; `und` when undetermined.
    #[serde(default = "und")]
    #[specta(type = String)]
    #[schemars(with = "String")]
    pub language: LanguageIdentifier,
    #[serde(default)]
    pub transform: Transform,
}

/// Limits its parent's visibility; never drawn, so of the common properties
/// it takes only the name, visibility, and attributes. Masks apply in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub attributes: Attributes,
    #[serde(default)]
    pub mode: MaskMode,
    #[serde(default)]
    pub content: MaskContent,
}

/// A layer: the common properties plus its kind's own. In JSON it is one
/// flat object tagged by `kind`.
#[derive(
    Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema, strum::EnumDiscriminants,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[strum_discriminants(
    name(LayerKind),
    doc = "What kind a layer is, without its properties.",
    derive(strum::AsRefStr, strum::EnumString),
    strum(serialize_all = "snake_case")
)]
pub enum Layer {
    Group(GroupLayer),
    Image(ImageLayer),
    Raster(RasterLayer),
    Shape(ShapeLayer),
    Text(TextLayer),
    Mask(MaskLayer),
}

/// The program that made a model's output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoftwareAgent {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// A C2PA action: what a model did and with what.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Action {
    /// Such as `c2pa.created` or `c2pa.translated`.
    pub action: String,
    pub software_agent: SoftwareAgent,
    /// An IPTC digital source type URI.
    #[serde(default)]
    pub digital_source_type: Option<String>,
    /// For `c2pa.translated`: `sourceLanguage` and `targetLanguage`.
    #[serde(default)]
    pub parameters: Map<String, Value>,
}

/// How an ingredient relates to an action, as in C2PA.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Relationship {
    /// The one source the action derives from.
    ParentOf,
    /// Context the action also read.
    InputTo,
    /// A part placed into the result.
    ComponentOf,
}

/// What an author read: a layer's value at a path, by hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub layer: LayerId,
    #[specta(type = String)]
    #[schemars(with = "String")]
    pub path: PointerBuf,
    pub hash: Hash,
    pub relationship: Relationship,
}

impl Read {
    #[must_use]
    pub fn parent_of(self) -> Self {
        Self {
            relationship: Relationship::ParentOf,
            ..self
        }
    }

    #[must_use]
    pub fn component_of(self) -> Self {
        Self {
            relationship: Relationship::ComponentOf,
            ..self
        }
    }
}

/// One model action recorded on a layer: the paths it wrote and what it
/// read. Each path belongs to the latest action that wrote it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionRecord {
    pub action: Action,
    #[specta(type = Vec<String>)]
    #[schemars(with = "Vec<String>")]
    pub writes: Vec<PointerBuf>,
    pub ingredients: Vec<Read>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ProjectView {
    pub id: ProjectId,
    /// A SemVer version.
    pub version: String,
    /// In order.
    pub pages: Vec<PageView>,
}

/// A page's layer tree, first drawn first, without pixels or file bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct PageView {
    pub id: PageId,
    pub size: Size,
    pub layers: Vec<LayerView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct LayerView {
    pub id: LayerId,
    /// The hash of `""`, sent back as a read to edit safely.
    pub hash: Hash,
    pub layer: Layer,
    pub origin: Vec<ActionRecord>,
    /// Paths where the user's value replaced a model's.
    #[specta(type = Vec<String>)]
    #[schemars(with = "Vec<String>")]
    pub overrides: Vec<PointerBuf>,
    /// Paths a model wrote whose ingredients have changed since.
    #[specta(type = Vec<String>)]
    #[schemars(with = "Vec<String>")]
    pub stale: Vec<PointerBuf>,
    /// Drawn children, first to last.
    pub children: Vec<LayerView>,
    /// Masks, applied in order; not drawn.
    pub masks: Vec<LayerView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct LayerAnswer {
    pub page: PageId,
    pub parent: Option<LayerId>,
    pub view: LayerView,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct History {
    pub undo: Option<String>,
    pub redo: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Rgba8,
    Gray8,
}

/// Decoded 64×64 tiles at tile coordinates relative to `offset`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Tiles {
    pub offset: Offset,
    pub format: PixelFormat,
    pub tiles: Vec<Tile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct File {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Created {
    Page { id: PageId, label: Option<String> },
    Layer { id: LayerId, label: Option<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct PageChanges {
    pub added: Vec<PageId>,
    pub removed: Vec<PageId>,
    pub reordered: bool,
    pub changed: BTreeSet<PageId>,
}

/// What one edit, undo, or redo changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Change {
    pub revision: Revision,
    pub created: Vec<Created>,
    pub pages: PageChanges,
    /// Changed or removed layers.
    pub layers: BTreeSet<LayerId>,
    /// Layers a model's removal skipped because the user changed them.
    pub kept: Vec<LayerId>,
    pub history: History,
}
