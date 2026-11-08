//! Values that layers hold: geometry, paths, blending, and text frames.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Rgba;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Offset {
    pub x: i32,
    pub y: i32,
}

/// A pixel rectangle, for pixel writes and reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A rectangle in page pixels, for text frames.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
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

/// An opacity, from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(try_from = "f32", into = "f32")]
#[specta(transparent)]
pub struct Opacity(f32);

impl Opacity {
    pub const OPAQUE: Self = Self(1.0);
}

impl Default for Opacity {
    fn default() -> Self {
        Self::OPAQUE
    }
}

impl TryFrom<f32> for Opacity {
    type Error = String;

    fn try_from(opacity: f32) -> Result<Self, String> {
        if (0.0..=1.0).contains(&opacity) {
            Ok(Self(opacity))
        } else {
            Err(format!("an opacity of {opacity} isn't between 0 and 1"))
        }
    }
}

impl From<Opacity> for f32 {
    fn from(opacity: Opacity) -> Self {
        opacity.0
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
    /// Containers only: children blend with what is below the container.
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

/// How a box's text meets the box.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Text that would overflow shrinks just enough to fit, down to half its
    /// size.
    #[default]
    Shrink,
    /// The set size always.
    None,
    /// The box keeps its inline size, and its block size follows the text,
    /// growing away from its anchor.
    Grow,
}

/// How a path's text meets the path: as a box's, but a path never grows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathFit {
    #[default]
    Shrink,
    None,
}

/// Where the lines sit along a frame's block axis, which follows the writing
/// direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    Start,
    Center,
    End,
}

/// Where a text lays out. Externally tagged, so a merge patch switches
/// variants by clearing the old one: `{"frame": {"path": {...}, "box": null}}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    /// Point text: the line grows as it is typed.
    #[default]
    Point,
    /// Paragraph text wrapping inside a box, in the text's own coordinates.
    Box {
        bounds: Bounds,
        #[serde(default)]
        fit: Fit,
        #[serde(default)]
        anchor: Anchor,
    },
    /// Text filling a closed path, in the text's own coordinates.
    Path {
        path: Path,
        #[serde(default)]
        fit: PathFit,
        #[serde(default)]
        anchor: Anchor,
        /// The air between the text and the path in ems, as CSS's
        /// `shape-padding: 1em`, so it shrinks with the text.
        #[serde(default = "one_em", deserialize_with = "inset")]
        inset: f64,
    },
}

impl Frame {
    /// A path frame's inset unless one is given: Comicraft's letter width of
    /// air.
    pub const DEFAULT_INSET: f64 = 1.0;
}

fn one_em() -> f64 {
    Frame::DEFAULT_INSET
}

/// An inset, which as CSS's `shape-padding` can't be negative.
fn inset<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let inset = f64::deserialize(deserializer)?;
    if inset >= 0.0 {
        Ok(inset)
    } else {
        Err(serde::de::Error::custom(format!(
            "an inset of {inset} is negative"
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f64,
}
