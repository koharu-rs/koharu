//! A text layer set in its frame, in stages that each take one type and
//! give the next: the text styled, its runs shaped once, its segments
//! between break opportunities, lines fitted to the frame's room, and glyphs
//! arranged into a `TextLayout`, which draws itself onto a drawing.

use std::ops::Range;

use anyhow::Result;
use koharu_scene::{Align, CharStyle, Color, Text};
use kurbo::Rect;

use self::{room::Room, shape::ShapedText, style::Styled};
use crate::fonts::Font;

mod arrange;
mod breaks;
mod draw;
mod fit;
mod itemize;
mod label;
mod lettering;
mod lines;
mod room;
mod shape;
mod style;

pub(crate) use self::{arrange::TextLayout, breaks::preload_dictionary, draw::draw};
pub use self::{label::Label, lettering::Lettering};

/// Laid-out text, as a run of one face at one size and paint.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// UTF-16 code units of the text.
    pub range: Range<usize>,
    pub post_script_name: String,
    /// The size it was drawn at.
    pub size: f32,
    pub fill: Color,
    /// The stroke colour and width.
    pub stroke: Option<(Color, f32)>,
}

/// What laying out a text resolved, for the inspector and PSD export.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedText {
    pub runs: Vec<TextRun>,
    /// The box the text was laid out in, in its own coordinates: the frame's
    /// box, a grown box, a path's bounds, or a point text's lines.
    pub frame: Rect,
    /// The scale fitting drew it at; 1 at its set size.
    pub scale: f32,
    /// UTF-16 offsets where a line ends with an added hyphen.
    pub hyphens: Vec<usize>,
    pub overflowed: bool,
    /// Where the first line's glyphs stand, in the text's own coordinates:
    /// the y of its baseline in horizontal text, and in vertical text the x
    /// of the column's centre line. Photoshop anchors point type there.
    pub baseline: f32,
    /// The alignment and line height it was set with, its style's or the
    /// defaults.
    pub align: Align,
    /// Whether its first paragraph runs right to left, which puts the start
    /// of its lines on the right.
    pub rtl: bool,
    pub line_height: f32,
    /// The characters no font of their span has, drawn as its first font's
    /// missing glyph, in code point order.
    pub uncovered: Vec<char>,
}

/// `text` laid out, each span set in the chain `fonts` gives for its
/// style and characters; `None` when it has no characters.
pub(crate) fn layout(
    text: &Text,
    fonts: impl FnMut(&CharStyle, &str) -> Result<Vec<Font>>,
) -> Result<Option<TextLayout>> {
    let styled = Styled::new(text, fonts)?;
    if styled.spans.is_empty() {
        return Ok(None);
    }
    let shaped = ShapedText::new(&styled)?;
    let room = Room::new(&styled.frame, styled.vertical);
    let (segments, placed) = fit::fit(&styled, &room, |breaking| {
        breaks::segments(&shaped, breaking)
    });
    arrange::arrange(&shaped, &segments, placed).map(Some)
}
