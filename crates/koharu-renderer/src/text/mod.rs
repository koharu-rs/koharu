//! A text layer set in its frame, in stages that each take one type and
//! give the next: the text styled, its runs shaped once, its segments
//! between break opportunities, lines fitted to the frame's room, and glyphs
//! arranged into a `TextLayout`, which paints into vello scenes.

use std::ops::Range;

use anyhow::Result;
use koharu_scene::{Align, CharStyle, Color, Text};
use vello::kurbo::Rect;

use self::{room::Room, shape::ShapedText, style::Styled};
use crate::fonts::Font;

mod arrange;
mod breaks;
mod fit;
mod itemize;
mod lines;
mod paint;
mod room;
mod shape;
mod style;

pub(crate) use self::{arrange::TextLayout, breaks::preload_dictionary, paint::paint};

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

/// `text` laid out, each span set in the chain `fonts` gives for its style
/// and characters; `None` when it has no characters.
pub(crate) fn layout(
    text: &Text,
    fonts: impl FnMut(&CharStyle, &str) -> Result<Vec<Font>>,
) -> Result<Option<TextLayout>> {
    let styled = Styled::new(text, fonts)?;
    if styled.spans.is_empty() {
        return Ok(None);
    }
    let shaped = ShapedText::new(&styled)?;
    let room = Room::new(&styled.frame, styled.vertical)?;
    let (segments, placed) = fit::fit(&styled, &room, |breaking| {
        breaks::segments(&shaped, breaking)
    });
    arrange::arrange(&shaped, &segments, placed).map(Some)
}

/// Texts set in installed fonts, for the stages' tests.
#[cfg(test)]
mod testing {
    use koharu_scene::{Align, Color, Frame};

    use super::style::{Span, Styled};
    use crate::fonts::{Font, FontSystem};

    const PRIMARY_FAMILIES: &[&str] = &[
        "Arial",
        "Segoe UI",
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
        "Yu Gothic",
        "MS Gothic",
        "Times New Roman",
    ];

    /// The first of a few common text fonts that is installed.
    pub(super) fn primary(fonts: &mut FontSystem) -> Font {
        PRIMARY_FAMILIES
            .iter()
            .find_map(|name| fonts.query_family(name).ok())
            .expect("no primary font available")
    }

    /// `text` in one 16-pixel span set in `fonts`, as point text.
    pub(super) fn styled(text: &str, fonts: Vec<Font>) -> Styled {
        Styled {
            text: text.to_owned(),
            spans: vec![Span {
                range: 0..text.len(),
                fonts,
                size: 16.0,
                fill: Color::BLACK,
                stroke: None,
            }],
            vertical: false,
            align: Align::Start,
            line_height: 1.2,
            hyphenation: None,
            frame: Frame::Point,
        }
    }
}
