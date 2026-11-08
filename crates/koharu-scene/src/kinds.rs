//! What each kind of layer holds: Photoshop's layer kinds, and the parts of a
//! manga page as Clip Studio Paint keeps its frames and balloons.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned, de::Error as _};
use serde_json::Value;
use specta::Type;

use crate::{
    FillRule, Hash, Layer, MaskMode, Offset, Path, Rgba, Stroke, Text, Transform, layer::part,
};

/// A folder of layers, drawn first to last.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Group {}

/// An imported file, kept byte for byte.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub file: Hash,
    #[serde(default)]
    pub transform: Transform,
}

/// Editable RGBA pixels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Raster {
    #[serde(default)]
    pub offset: Offset,
}

/// A vector path with an optional fill and stroke, in page coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub path: Path,
    #[serde(default)]
    pub fill_rule: FillRule,
    #[serde(default)]
    pub fill: Option<Rgba>,
    #[serde(default)]
    pub stroke: Option<Stroke>,
}

/// Grey pixels that limit its parent's visibility, as Photoshop's and Clip
/// Studio Paint's layer masks do; never drawn. Masks apply in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    #[serde(default)]
    pub mode: MaskMode,
    #[serde(default)]
    pub offset: Offset,
}

/// A panel of the page. Its children show only inside its outline, which is
/// never drawn, as a Clip Studio Paint frame border folder with Draw border
/// off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    /// A closed path in page coordinates.
    pub outline: Path,
}

/// A balloon of the page, holding its texts. Its outline is never drawn or
/// filled, because the page's background already shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Balloon {
    /// A closed path in page coordinates.
    pub outline: Path,
}

/// What a block's text is, as a PAGE-XML text region's `type` gives the
/// nature of its text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Text,
    Sfx,
}

/// One text of the page, in a fixed shape: its source and its translation,
/// both text values, as an XLIFF segment's source and target share one
/// content model, and two layers of its own, its `mask` and its `cleanup`, as
/// Photoshop's Generative Fill puts its result on a layer with a layer mask.
/// A patch on the block reaches its parts; painting them takes their own ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Block {
    #[serde(default)]
    pub role: Role,
    /// The original text, never drawn, in a box or a closed path, as a
    /// PAGE-XML text region records it.
    pub source: Text,
    /// As an XLIFF segment holds zero or one target.
    #[serde(default)]
    pub translation: Option<Text>,
    /// Where the source's glyphs are: the clean-up shows only inside it while
    /// it's visible.
    #[serde(default = "part::<Mask>", deserialize_with = "strict")]
    pub mask: Layer<Mask>,
    /// The scan redrawn without the source's glyphs, under the translation.
    #[serde(default = "part::<Raster>", deserialize_with = "strict")]
    pub cleanup: Layer<Raster>,
}

/// A block's part, refusing a field it doesn't have. A flattened struct takes
/// only the fields it knows, unlike a layer's tagged kind, so serde alone
/// would drop a misspelt one.
fn strict<'de, D, C>(deserializer: D) -> Result<Layer<C>, D::Error>
where
    D: Deserializer<'de>,
    C: Serialize + DeserializeOwned,
{
    let given = Value::deserialize(deserializer)?;
    let part: Layer<C> = serde_json::from_value(given.clone()).map_err(D::Error::custom)?;
    let read = serde_json::to_value(&part).map_err(D::Error::custom)?;
    if let (Value::Object(given), Value::Object(read)) = (&given, &read)
        && let Some(key) = given.keys().find(|key| !read.contains_key(*key))
    {
        return Err(D::Error::custom(format!("unknown field `{key}`")));
    }
    Ok(part)
}

/// A layer's kind and what that kind holds. In JSON its fields sit beside the
/// layer's own, tagged by `kind`.
#[derive(
    Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema, strum::EnumDiscriminants,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[strum_discriminants(
    name(LayerKind),
    doc = "What kind a layer is, without its values.",
    derive(strum::AsRefStr, Serialize, Deserialize),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Content {
    Group(Group),
    Image(Image),
    Raster(Raster),
    Shape(Shape),
    Text(Box<Text>),
    Mask(Mask),
    Panel(Panel),
    Balloon(Balloon),
    Block(Box<Block>),
}
