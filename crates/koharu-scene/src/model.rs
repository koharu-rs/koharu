//! Public types: ids, values, layers, provenance, and views. Every type here
//! serializes the way commands and tools see it.

use std::collections::{BTreeMap, BTreeSet};

use color::{AlphaColor, Srgb};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use uuid::Uuid;

/// An sRGB colour with straight alpha, serialized as `[r, g, b, a]`, each
/// from 0 to 1, as Figma's `Color` is.
pub type Color = AlphaColor<Srgb>;

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

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

macro_rules! checked_strings {
    ($($(#[$doc:meta])* $name:ident => $check:expr),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Type, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                let check: fn(&str) -> Result<(), String> = $check;
                check(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    )*};
}

checked_strings! {
    /// An existing page or layer id, or `$label` for one created earlier in
    /// the same edit.
    Ref => |s| {
        let valid = match s.strip_prefix('$') {
            Some(label) => {
                !label.is_empty()
                    && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            }
            None => Uuid::parse_str(s).is_ok(),
        };
        if valid { Ok(()) } else { Err(format!("{s:?} is neither an id nor a $label")) }
    },
    /// A JSON Pointer (RFC 6901) into a layer's properties; `""` is the
    /// whole layer.
    Pointer => |s| jsonptr::Pointer::parse(s).map(|_| ()).map_err(|e| e.to_string()),
    /// SVG path data.
    Path => |s| kurbo::BezPath::from_svg(s).map(|_| ()).map_err(|e| format!("invalid path data: {e}")),
    /// A BCP 47 language tag; `und` when undetermined.
    LanguageTag => |s| {
        icu_locale_core::Locale::try_from_str(s)
            .map(|_| ())
            .map_err(|e| format!("{s:?} is not a BCP 47 tag: {e}"))
    },
    /// A BLAKE3 hash, as 64 lowercase hex digits.
    Hash => check_hex,
    /// The BLAKE3 hash of an imported file's bytes.
    FileHash => check_hex,
}

fn check_hex(s: &str) -> Result<(), String> {
    if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(format!("{s:?} is not a BLAKE3 hash"))
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

impl Pointer {
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    pub(crate) fn content() -> Self {
        Self("/content".to_owned())
    }

    pub(crate) fn pixels() -> Self {
        Self("/pixels".to_owned())
    }

    fn parsed(&self) -> &jsonptr::Pointer {
        jsonptr::Pointer::parse(&self.0).expect("checked on construction")
    }

    #[must_use]
    pub fn tokens(&self) -> Vec<String> {
        self.parsed()
            .tokens()
            .map(|token| token.decoded().into_owned())
            .collect()
    }

    #[must_use]
    pub fn child(&self, token: &str) -> Self {
        Self(self.parsed().with_trailing_token(token).as_str().to_owned())
    }

    /// True when `self` is `other` or lies inside it.
    #[must_use]
    pub fn within(&self, other: &Self) -> bool {
        self.parsed().starts_with(other.parsed())
    }

    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.within(other) || other.within(self)
    }
}

impl LanguageTag {
    #[must_use]
    pub fn und() -> Self {
        Self("und".to_owned())
    }
}

impl From<blake3::Hash> for Hash {
    fn from(hash: blake3::Hash) -> Self {
        Self(hash.to_hex().to_string())
    }
}

impl FileHash {
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes).to_hex().to_string())
    }
}

/// The document's version, as opaque Loro frontiers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
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

/// What a mask is made of. Pixel tiles are read at `/pixels`.
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
    #[serde(with = "color_serde")]
    #[specta(type = [f32; 4])]
    #[schemars(with = "[f32; 4]")]
    pub color: Color,
    pub width: f64,
}

/// Character values; each is also a mark key on a run, and a mark wins for
/// its run.
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
    #[serde(default, with = "option_color_serde")]
    #[specta(type = Option<[f32; 4]>)]
    #[schemars(with = "Option<[f32; 4]>")]
    pub fill: Option<Color>,
    #[serde(default, with = "option_color_serde")]
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

/// Text with styled runs. Deserializes from a plain string too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(from = "RichTextInput")]
pub struct RichText {
    pub text: String,
    pub runs: Vec<Run>,
}

/// A styled run over Unicode scalar indices `start..end`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub style: CharStyle,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum RichTextInput {
    Plain(String),
    Rich(RichTextFields),
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RichTextFields {
    text: String,
    #[serde(default)]
    runs: Vec<Run>,
}

impl From<RichTextInput> for RichText {
    fn from(input: RichTextInput) -> Self {
        match input {
            RichTextInput::Plain(text) => Self {
                text,
                runs: Vec::new(),
            },
            RichTextInput::Rich(RichTextFields { text, runs }) => Self { text, runs },
        }
    }
}

impl From<&str> for RichText {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            runs: Vec::new(),
        }
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

fn normal() -> Blend {
    Blend::Normal
}

fn pass_through() -> Blend {
    Blend::PassThrough
}

macro_rules! layers {
    ($(
        $(#[$doc:meta])*
        $variant:ident => $name:ident ($blend:literal) {
            $( $(#[$field_meta:meta])* $field:ident : $ty:ty ),* $(,)?
        }
    )*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
            #[serde(deny_unknown_fields)]
            pub struct $name {
                #[serde(default)]
                pub name: String,
                #[serde(default = "visible")]
                pub visible: bool,
                #[serde(default)]
                pub locked: bool,
                /// 0 to 1.
                #[serde(default = "opaque")]
                pub opacity: f32,
                #[serde(default = $blend)]
                pub blend: Blend,
                #[serde(default)]
                pub attributes: Attributes,
                $( $(#[$field_meta])* pub $field: $ty, )*
            }
        )*

        /// A layer: the common properties plus its kind's own. In JSON it is
        /// one flat object tagged by `kind`.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum Layer {
            $( $variant($name), )*
        }

        /// What kind a layer is, without its properties.
        #[derive(
            Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema,
            strum::AsRefStr, strum::EnumString,
        )]
        #[serde(rename_all = "snake_case")]
        #[strum(serialize_all = "snake_case")]
        pub enum LayerKind {
            $( $variant, )*
        }

        impl Layer {
            #[must_use]
            pub fn kind(&self) -> LayerKind {
                match self { $( Self::$variant(_) => LayerKind::$variant, )* }
            }

            #[must_use]
            pub fn name(&self) -> &str {
                match self { $( Self::$variant(layer) => &layer.name, )* }
            }

            fn common(&self) -> (f32, Blend) {
                match self { $( Self::$variant(layer) => (layer.opacity, layer.blend), )* }
            }
        }
    };
}

layers! {
    /// A folder of layers, drawn first to last.
    Group => GroupLayer ("pass_through") {}
    /// An imported file, kept byte for byte.
    Image => ImageLayer ("normal") {
        file: FileHash,
        #[serde(default)]
        transform: Transform,
    }
    /// Editable RGBA pixels, read at `/pixels`.
    Raster => RasterLayer ("normal") {
        #[serde(default)]
        offset: Offset,
    }
    /// A vector path with an optional fill and stroke, in page coordinates.
    Shape => ShapeLayer ("normal") {
        path: Path,
        #[serde(default)]
        fill_rule: FillRule,
        #[serde(default, with = "option_color_serde")]
        #[specta(type = Option<[f32; 4]>)]
        #[schemars(with = "Option<[f32; 4]>")]
        fill: Option<Color>,
        #[serde(default)]
        stroke: Option<Stroke>,
    }
    /// Styled text in a frame. `content` is set when the layer is added;
    /// after that only the text ops change it.
    Text => TextLayer ("normal") {
        #[serde(default)]
        content: RichText,
        #[serde(default)]
        style: TextStyle,
        #[serde(default)]
        frame: Frame,
        #[serde(default)]
        direction: Direction,
        #[serde(default = "LanguageTag::und")]
        language: LanguageTag,
        #[serde(default)]
        transform: Transform,
    }
    /// Limits its parent's visibility; never drawn. Masks apply in order.
    Mask => MaskLayer ("normal") {
        #[serde(default)]
        mode: MaskMode,
        #[serde(default)]
        content: MaskContent,
    }
}

impl Layer {
    /// Checks what serde can't: ranges, finite numbers, and runs inside the
    /// text. Whether a named file exists is the planner's to check.
    pub fn check(&self) -> Result<(), String> {
        let (opacity, blend) = self.common();
        if !(0.0..=1.0).contains(&opacity) {
            return Err(format!("opacity {opacity} is outside 0 to 1"));
        }
        if blend == Blend::PassThrough && !matches!(self, Self::Group(_)) {
            return Err("only a group can pass through".to_owned());
        }
        match self {
            Self::Group(_) | Self::Raster(_) | Self::Mask(_) => Ok(()),
            Self::Image(image) => finite_transform(&image.transform),
            Self::Shape(shape) => {
                shape.fill.iter().try_for_each(check_color)?;
                match &shape.stroke {
                    Some(stroke) => {
                        check_color(&stroke.color)?;
                        at_least_zero(stroke.width, "a stroke width")
                    }
                    None => Ok(()),
                }
            }
            Self::Text(text) => {
                finite_transform(&text.transform)?;
                text.style.chars.check()?;
                let length = text.content.text.chars().count();
                for run in &text.content.runs {
                    if run.start >= run.end || run.end > length {
                        return Err(format!(
                            "run {}..{} is outside the text",
                            run.start, run.end
                        ));
                    }
                    run.style.check()?;
                }
                if let Frame::Box { bounds } = &text.frame {
                    if !(bounds.x.is_finite() && bounds.y.is_finite()) {
                        return Err("a frame must be finite".to_owned());
                    }
                    at_least_zero(bounds.width, "a frame width")?;
                    at_least_zero(bounds.height, "a frame height")?;
                }
                Ok(())
            }
        }
    }
}

impl CharStyle {
    pub fn check(&self) -> Result<(), String> {
        if let Some(size) = self.size
            && !(size.is_finite() && size > 0.0)
        {
            return Err("a font size must be above 0".to_owned());
        }
        if let Some(weight) = self.weight
            && !(1..=1000).contains(&weight)
        {
            return Err("a font weight must be from 1 to 1000".to_owned());
        }
        if let Some(width) = self.stroke_width {
            at_least_zero(width, "a stroke width")?;
        }
        self.fill
            .iter()
            .chain(&self.stroke)
            .try_for_each(check_color)
    }
}

fn check_color(color: &Color) -> Result<(), String> {
    if color.components.iter().all(|c| (0.0..=1.0).contains(c)) {
        Ok(())
    } else {
        Err("a colour component is outside 0 to 1".to_owned())
    }
}

fn at_least_zero(value: f64, what: &str) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{what} must be a finite number of at least 0"))
    }
}

fn finite_transform(transform: &Transform) -> Result<(), String> {
    if transform.0.iter().flatten().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err("a transform must be finite".to_owned())
    }
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
    pub path: Pointer,
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
    pub writes: Vec<Pointer>,
    pub ingredients: Vec<Read>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ProjectView {
    pub id: ProjectId,
    /// A SemVer version.
    pub version: String,
    pub pages: Vec<PageSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct PageSummary {
    pub id: PageId,
    pub index: usize,
    pub size: Size,
    pub layers: usize,
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
    pub overrides: Vec<Pointer>,
    /// Paths a model wrote whose ingredients have changed since.
    pub stale: Vec<Pointer>,
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

/// A translation paired with the source it was translated from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct StoryEntry {
    pub translation: LayerId,
    pub translation_hash: Hash,
    pub text: String,
    pub language: LanguageTag,
    pub source: LayerId,
    pub source_hash: Hash,
    pub source_text: String,
    pub source_language: LanguageTag,
    pub stale: bool,
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

mod color_serde {
    use super::Color;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(color: &Color, serializer: S) -> Result<S::Ok, S::Error> {
        color.components.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
        <[f32; 4]>::deserialize(deserializer).map(Color::new)
    }
}

mod option_color_serde {
    use super::Color;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        color: &Option<Color>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        color.map(|color| color.components).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Color>, D::Error> {
        Option::<[f32; 4]>::deserialize(deserializer).map(|c| c.map(Color::new))
    }
}
