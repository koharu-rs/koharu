//! Colours as `[r, g, b, a]`, as Figma's `Color` is. `color`'s own serde writes
//! `{"components": [...]}` and has no schemars or specta support, so layers
//! hold the components and convert at the edge.

use color::{AlphaColor, Srgb};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

/// An sRGB colour with straight alpha.
pub type Color = AlphaColor<Srgb>;

/// An sRGB colour with straight alpha, each component from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(transparent)]
pub struct Rgba(pub [f32; 4]);

impl From<Rgba> for Color {
    fn from(Rgba(components): Rgba) -> Self {
        Self::new(components)
    }
}

impl From<Color> for Rgba {
    fn from(color: Color) -> Self {
        Self(color.components)
    }
}
