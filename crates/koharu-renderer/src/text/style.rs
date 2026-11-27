//! A scene text as layout reads it: its characters, and each span's size,
//! paint and font chain, with the defaults applied.

use std::ops::Range;

use anyhow::Result;
use hypher::Lang;
use koharu_scene::{Align, CharStyle, Color, Direction, Emphasis, Frame, Orientation, Text};

use super::{
    emphasis::Mark,
    ruby::{self, Ruby},
};
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
    /// Its language is Chinese, whose vertical stop marks sit in the middle
    /// of their cells.
    pub chinese: bool,
    /// Its language is Vietnamese, whose words its lines keep whole where
    /// they can.
    pub vietnamese: bool,
    /// Where the text sets, in its own coordinates.
    pub frame: Frame,
    /// In text order.
    pub ruby: Vec<Ruby>,
    /// The emphasis marks its spans set, in span order.
    pub marks: Vec<Mark>,
}

pub(super) struct Span {
    pub range: Range<usize>,
    /// The font chain: the face asked for, then fallbacks.
    pub fonts: Vec<Font>,
    pub size: f32,
    pub fill: Color,
    /// The colour and width of a stroke.
    pub stroke: Option<(Color, f32)>,
    /// How it stands in a column.
    pub orientation: Orientation,
}

impl Styled {
    /// `text` with each span's style over the text's, set in the chain
    /// `fonts` gives for that style and the span's characters, its ruby's and
    /// its emphasis mark among them.
    pub(super) fn new(
        text: &Text,
        mut fonts: impl FnMut(&CharStyle, &str) -> Result<Vec<Font>>,
    ) -> Result<Self> {
        let mut plain = String::new();
        let mut spans = Vec::new();
        let mut marked = Vec::new();
        for span in &text.content.0 {
            if span.insert.is_empty() {
                continue;
            }
            let style = span.attributes.or(&text.style.chars);
            let ruby = span.ruby.as_deref().unwrap_or_default();
            let mark = style.emphasis.and_then(Emphasis::mark);
            if let Some(mark) = mark {
                marked.push((spans.len(), mark));
            }
            let start = plain.len();
            plain.push_str(&span.insert);
            let mark = mark.map(String::from).unwrap_or_default();
            spans.push(Span {
                range: start..plain.len(),
                fonts: fonts(&style, &[&span.insert, ruby, &mark].concat())?,
                size: style.size.unwrap_or(24.0) as f32,
                fill: style.fill.map_or(Color::BLACK, Color::from),
                stroke: match (style.stroke, style.stroke_width) {
                    (Some(color), Some(width)) if width > 0.0 => {
                        Some((Color::from(color), width as f32))
                    }
                    _ => None,
                },
                orientation: style.orientation.unwrap_or_default(),
            });
        }
        let mut styled = Self {
            text: plain,
            spans,
            vertical: text.direction == Direction::Vertical,
            align: text.style.align.unwrap_or_default(),
            line_height: text.style.line_height.unwrap_or(1.2) as f32,
            // By the language's two-letter code.
            hyphenation: text
                .language
                .language
                .as_str()
                .as_bytes()
                .try_into()
                .ok()
                .and_then(Lang::from_iso),
            chinese: text.language.language.as_str() == "zh",
            vietnamese: text.language.language.as_str() == "vi",
            frame: text.frame.clone(),
            ruby: Vec::new(),
            marks: Vec::new(),
        };
        styled.ruby = ruby::style(&text.content, &styled);
        styled.marks = marked
            .into_iter()
            .map(|(span, mark)| Mark::new(&styled, span, mark))
            .collect();
        Ok(styled)
    }

    /// The span holding byte `offset`, or the last at the text's end.
    pub(super) fn span_at(&self, offset: usize) -> usize {
        self.spans
            .iter()
            .position(|span| span.range.contains(&offset))
            .unwrap_or(self.spans.len().saturating_sub(1))
    }
}
