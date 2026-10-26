//! Colours as `[r, g, b, a]`. `color`'s own serde writes
//! `{"components": [r, g, b, a]}`, and it has no specta or schemars support,
//! so colour fields name these modules and their schema type.

use color::{AlphaColor, Srgb};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with straight alpha, serialized as `[r, g, b, a]`, each
/// from 0 to 1, as Figma's `Color` is.
pub type Color = AlphaColor<Srgb>;

pub(crate) mod array {
    use super::{Color, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(color: &Color, serializer: S) -> Result<S::Ok, S::Error> {
        color.components.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
        <[f32; 4]>::deserialize(deserializer).map(Color::new)
    }
}

pub(crate) mod option {
    use super::{Color, Deserialize, Deserializer, Serialize, Serializer};

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
