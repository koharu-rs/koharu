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

/// A reader's decision about the detector's ink. The source measurement remains evidence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InkChoice {
    Keep,
    Black,
    White,
}

impl InkChoice {
    pub(crate) fn replacement(self, detected: Rgba) -> Option<Rgba> {
        let level = match self {
            Self::Keep => return None,
            Self::Black => 0.0,
            Self::White => 1.0,
        };
        Some(Rgba([level, level, level, detected.0[3]]))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct InkSuggestion {
    pub location: crate::TermLocation,
    pub block_number: Option<usize>,
    pub detected: Rgba,
    pub translation_fill: Option<Rgba>,
}

impl crate::Layer {
    fn ink_choice(&self, detected: Rgba) -> Option<InkChoice> {
        let value = self.attributes.get("koharu")?.get("ink_review")?;
        let (reviewed, choice): (Rgba, InkChoice) = serde_json::from_value(value.clone()).ok()?;
        (reviewed == detected).then_some(choice)
    }

    /// A correction made before translation also sets its initial fill.
    pub fn reviewed_ink(&self, detected: Rgba) -> Rgba {
        self.ink_choice(detected)
            .and_then(|choice| choice.replacement(detected))
            .unwrap_or(detected)
    }

    pub(crate) fn pending_ink(&self) -> Option<(Rgba, Option<Rgba>)> {
        let crate::Content::Block(block) = &self.content else {
            return None;
        };
        self.attributes.get("c2pa")?.get("detection")?;
        let detected = block.source.style.chars.fill?;
        let [r, g, b, _] = detected.0;
        if [r, g, b] == [0.0; 3] || [r, g, b] == [1.0; 3] || self.ink_choice(detected).is_some() {
            return None;
        }
        Some((
            detected,
            block.translation.as_ref().and_then(|t| t.style.chars.fill),
        ))
    }
}

impl crate::Project {
    pub fn ink_suggestions(&self) -> crate::Result<Vec<InkSuggestion>> {
        let mut pending = Vec::new();
        for (i, page) in self.pages().enumerate() {
            let numbers: std::collections::HashMap<_, _> = self
                .story(page.id)?
                .into_iter()
                .enumerate()
                .map(|(i, id)| (id, i + 1))
                .collect();
            for layer in self.layers_of(page.id)? {
                let Some((detected, translation_fill)) = layer.pending_ink() else {
                    continue;
                };
                let crate::Content::Block(block) = &layer.content else {
                    continue;
                };
                pending.push(InkSuggestion {
                    location: crate::TermLocation {
                        page: page.id,
                        number: i + 1,
                        layer: layer.id,
                        text: block.source.content.text(),
                    },
                    block_number: numbers.get(&layer.id).copied(),
                    detected,
                    translation_fill,
                });
            }
        }
        Ok(pending)
    }
}
