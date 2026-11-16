//! A short line the canvas draws over the page, laid out and drawn as a
//! text layer's lettering is.

use anyhow::Result;
use koharu_scene::{CharStyle, Rgba, Text, TextStyle};
use kurbo::{Affine, Rect};

use super::{TextLayout, draw, layout};
use crate::{Drawing, fonts};

/// A short line in Arial, or the platform's sans-serif without it, as the
/// canvas draws a text's number over the page: laid out once, drawn at any
/// transform.
pub struct Label(TextLayout);

impl Label {
    /// `text` at `size` pixels and `weight`, filled with `fill`; `None` when
    /// it has nothing to draw.
    pub fn new(text: &str, size: f64, weight: u16, fill: Rgba) -> Result<Option<Self>> {
        let text = Text {
            content: text.into(),
            style: TextStyle {
                chars: CharStyle {
                    size: Some(size),
                    weight: Some(weight),
                    fill: Some(fill),
                    ..CharStyle::default()
                },
                ..TextStyle::default()
            },
            ..Text::default()
        };
        let layout = layout(&text, |style, run| {
            fonts::chain(style, &[], run, &text.language)
        })?;
        Ok(layout.map(Self))
    }

    /// What it covers, its first baseline's start at the origin.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.0.bounds
    }

    /// Fills it onto `drawing` through `transform`.
    pub fn draw(&self, drawing: &mut Drawing, transform: Affine) -> Result<()> {
        draw(drawing, &self.0, transform, false)
    }
}
