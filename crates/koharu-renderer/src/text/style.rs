//! A scene text as layout reads it: its characters, and each span's size,
//! paint and font chain, with the defaults applied.

use std::ops::Range;

use anyhow::Result;
use hypher::Lang;
use koharu_scene::{Align, CharStyle, Color, Direction, Frame, Text};

use super::breaks::hyphenation;
use crate::fonts::Font;

pub(super) struct Styled {
    pub text: String,
    /// The text's non-empty spans, which cover it in order.
    pub spans: Vec<Span>,
    pub vertical: bool,
    pub align: Align,
    /// A multiple of each line's largest size.
    pub line_height: f32,
    /// The patterns that hyphenate the text's language, when it has some.
    pub hyphenation: Option<Lang>,
    /// Where the text sets, in its own coordinates.
    pub frame: Frame,
}

pub(super) struct Span {
    pub range: Range<usize>,
    /// The font chain: the face asked for, then fallbacks.
    pub fonts: Vec<Font>,
    pub size: f32,
    pub fill: Color,
    /// The colour and width of a stroke.
    pub stroke: Option<(Color, f32)>,
}

impl Styled {
    /// `text` with each span's style over the text's, set in the chain
    /// `fonts` gives for that style and the span's characters.
    pub(super) fn new(
        text: &Text,
        mut fonts: impl FnMut(&CharStyle, &str) -> Result<Vec<Font>>,
    ) -> Result<Self> {
        let mut plain = String::new();
        let mut spans = Vec::new();
        for span in &text.content.0 {
            if span.insert.is_empty() {
                continue;
            }
            let style = span.attributes.or(&text.style.chars);
            let start = plain.len();
            plain.push_str(&span.insert);
            spans.push(Span {
                range: start..plain.len(),
                fonts: fonts(&style, &span.insert)?,
                size: style.size.unwrap_or(24.0) as f32,
                fill: style.fill.map_or(Color::BLACK, Color::from),
                stroke: match (style.stroke, style.stroke_width) {
                    (Some(color), Some(width)) if width > 0.0 => {
                        Some((Color::from(color), width as f32))
                    }
                    _ => None,
                },
            });
        }
        Ok(Self {
            text: plain,
            spans,
            vertical: text.direction == Direction::Vertical,
            align: text.style.align.unwrap_or_default(),
            line_height: text.style.line_height.unwrap_or(1.2) as f32,
            hyphenation: hyphenation(&text.language),
            frame: text.frame.clone(),
        })
    }

    /// The span holding byte `offset`, or the last at the text's end.
    pub(super) fn span_at(&self, offset: usize) -> usize {
        self.spans
            .iter()
            .position(|span| span.range.contains(&offset))
            .unwrap_or(self.spans.len().saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Rgba, RichText, Span as SceneSpan, TextStyle};

    use super::*;

    #[test]
    fn a_span_takes_its_own_style_over_the_text_s_and_empty_ones_drop() -> Result<()> {
        let red = Rgba([1.0, 0.0, 0.0, 1.0]);
        let span = |insert: &str, attributes| SceneSpan {
            insert: insert.to_owned(),
            attributes,
        };
        let text = Text {
            content: RichText(vec![
                span(
                    "big ",
                    CharStyle {
                        size: Some(40.0),
                        ..CharStyle::default()
                    },
                ),
                span("", CharStyle::default()),
                span("red", CharStyle::default()),
            ]),
            style: TextStyle {
                chars: CharStyle {
                    fill: Some(red),
                    ..CharStyle::default()
                },
                ..TextStyle::default()
            },
            ..Text::default()
        };
        let styled = Styled::new(&text, |_, _| Ok(Vec::new()))?;
        assert_eq!(styled.text, "big red");
        assert_eq!(styled.spans.len(), 2);
        assert_eq!(styled.spans[0].size, 40.0);
        assert_ne!(styled.spans[1].size, 40.0);
        assert!(
            styled
                .spans
                .iter()
                .all(|span| span.fill == Color::from(red))
        );
        Ok(())
    }
}
