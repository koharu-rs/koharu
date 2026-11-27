//! Each run shaped once with HarfRust, through one reused buffer, and each
//! span's hyphen.

use std::ops::Range;

use anyhow::{Context, Result};
use harfrust::{Buffer, Direction, Feature, ShapeOptions, ShaperFont, Tag};
use icu_properties::props::Script as IcuScript;
use icu_segmenter::GraphemeClusterSegmenter;
use unicode_bidi::{BidiInfo, Level, ParagraphInfo};

use super::{
    emphasis::{self, ShapedMark},
    itemize::{Orientation, Run, itemize},
    ruby::{self, ShapedRuby},
    style::Styled,
};
use crate::{fonts::Font, script::harfrust_script};

/// A shaped glyph, at the set size.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glyph {
    pub id: u32,
    /// The text offset of its cluster's first character.
    pub cluster: usize,
    /// Along the inline axis.
    pub advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    /// Breaking the text where its cluster starts changes how both sides
    /// shape, as HarfBuzz's `UNSAFE_TO_BREAK` says.
    pub unsafe_to_break: bool,
}

/// The text as it lays out at any scale: its runs, shaped once.
pub(super) struct ShapedText<'a> {
    pub styled: &'a Styled,
    /// The bidi paragraphs, whose levels say which side lines start on.
    pub paragraphs: Vec<ParagraphInfo>,
    /// In logical order, leaving out the characters that break lines.
    pub runs: Vec<Run>,
    pub glyphs: Vec<Glyph>,
    /// Each span's hyphen, when the text's language hyphenates.
    pub hyphens: Vec<Option<Run>>,
    /// Its ruby, in text order.
    pub ruby: Vec<ShapedRuby<'a>>,
    /// Its emphasis marks, as `Styled::marks` has them.
    pub marks: Vec<ShapedMark<'a>>,
    /// Characters no font of their span has, in code point order.
    pub uncovered: Vec<char>,
}

impl<'a> ShapedText<'a> {
    pub(super) fn new(styled: &'a Styled) -> Result<Self> {
        let BidiInfo {
            levels, paragraphs, ..
        } = BidiInfo::new(&styled.text, None);
        let (mut runs, uncovered) = itemize(styled, &levels);
        let mut buffer = Buffer::new();
        let mut glyphs = Vec::new();
        for run in &mut runs {
            let shaped = shape_run(styled, &mut buffer, run, run.range.clone())?;
            run.glyphs = glyphs.len()..glyphs.len() + shaped.len();
            if run.orientation == Orientation::TateChuYoko {
                let width: f32 = shaped.iter().map(|glyph| glyph.advance).sum();
                run.squeeze = (styled.spans[run.span].size / width).min(1.0);
            }
            glyphs.extend(shaped);
        }
        // Each span's hyphen, shaped in the first of its fonts that has one.
        let mut hyphens = Vec::with_capacity(styled.spans.len());
        for (index, span) in styled.spans.iter().enumerate() {
            if styled.hyphenation.is_none() || span.fonts.is_empty() {
                hyphens.push(None);
                continue;
            }
            let mut run = Run {
                range: 0..0,
                span: index,
                font: span
                    .fonts
                    .iter()
                    .position(|font| font.covers("-"))
                    .unwrap_or(0),
                level: Level::ltr(),
                script: harfrust_script(IcuScript::Common),
                orientation: Orientation::Upright,
                glyphs: 0..0,
                space: (0.0, 0.0),
                squeeze: 1.0,
            };
            let shaped = shape_text(styled, &mut buffer, "-", &run, None, |_| 0)?;
            run.glyphs = glyphs.len()..glyphs.len() + shaped.len();
            glyphs.extend(shaped);
            hyphens.push(Some(run));
        }
        let mut shaped = Self {
            styled,
            paragraphs,
            runs,
            glyphs,
            hyphens,
            ruby: Vec::new(),
            marks: Vec::new(),
            uncovered,
        };
        shaped.ruby = ruby::shape(&mut shaped)?;
        shaped.marks = emphasis::shape(&shaped)?;
        Ok(shaped)
    }

    pub(super) fn glyphs(&self, run: &Run) -> &[Glyph] {
        &self.glyphs[run.glyphs.clone()]
    }

    pub(super) fn font(&self, run: &Run) -> &Font {
        &self.styled.spans[run.span].fonts[run.font]
    }

    /// The advance of a tate-chu-yoko run's cell, as long as its size is.
    pub(super) fn cell(&self, run: &Run) -> f32 {
        run.space.0 + self.styled.spans[run.span].size + run.space.1
    }

    /// The advance of span `span`'s hyphen.
    pub(super) fn hyphen(&self, span: usize) -> f32 {
        self.hyphens[span].as_ref().map_or(0.0, |run| {
            self.glyphs(run).iter().map(|glyph| glyph.advance).sum()
        })
    }

    /// Whether the paragraph holding `offset` runs right to left, which
    /// starts its lines on the right. Vertical text never does.
    pub(super) fn rtl_at(&self, offset: usize) -> bool {
        !self.styled.vertical
            && self
                .paragraphs
                .iter()
                .find(|paragraph| paragraph.range.contains(&offset))
                .or(self.paragraphs.last())
                .is_some_and(|paragraph| paragraph.level.is_rtl())
    }
}

/// `range` of `run` shaped on its own: a run, or a line's end or start
/// where breaking changes its shaping.
pub(super) fn shape_run(
    styled: &Styled,
    buffer: &mut Buffer,
    run: &Run,
    range: Range<usize>,
) -> Result<Vec<Glyph>> {
    let source = &styled.text[range.clone()];
    // Vertical text sets `!` and `?` in their full-width forms, which stand
    // upright in a column, and a group of them in their proportional forms,
    // side by side in one cell. A font without the full-width forms, such as
    // a Latin comic font, stands its own up: the run's font was chosen for
    // the characters the text has.
    let font = &styled.spans[run.span].fonts[run.font];
    let (text, clusters) = match styled.vertical {
        true => exclamations(
            source,
            run.orientation == Orientation::TateChuYoko || !font.covers("！"),
        ),
        false => (source.to_owned(), (0..source.len()).collect()),
    };
    let cluster = |cluster| range.start + clusters.get(cluster).copied().unwrap_or(0);
    let glyphs = shape_text(styled, buffer, &text, run, None, cluster)?;
    // A tate-chu-yoko cell wider than an em takes its font's half-, third-, or
    // quarter-width forms for two, three, or four characters where they
    // narrow it, as CSS fits combined text into an em; `ShapedText` squeezes
    // what's still too wide.
    let width = |glyphs: &[Glyph]| glyphs.iter().map(|glyph| glyph.advance).sum::<f32>();
    if run.orientation != Orientation::TateChuYoko || width(&glyphs) <= styled.spans[run.span].size
    {
        return Ok(glyphs);
    }
    let tag = match GraphemeClusterSegmenter::new().segment_str(&text).count() - 1 {
        2 => b"hwid",
        3 => b"twid",
        4 => b"qwid",
        _ => return Ok(glyphs),
    };
    let narrowed = shape_text(styled, buffer, &text, run, Some(Tag::new(tag)), cluster)?;
    Ok(match width(&narrowed) < width(&glyphs) {
        true => narrowed,
        false => glyphs,
    })
}

/// `text` shaped as `run` says, with the feature `width` on, each glyph's
/// cluster mapped by `cluster` from a byte of `text` to one of the text.
fn shape_text(
    styled: &Styled,
    buffer: &mut Buffer,
    text: &str,
    run: &Run,
    width: Option<Tag>,
    cluster: impl Fn(usize) -> usize,
) -> Result<Vec<Glyph>> {
    // Sideways runs and tate-chu-yoko shape as horizontal text.
    let vertical = styled.vertical && run.orientation == Orientation::Upright;
    let direction = if vertical {
        Direction::TopToBottom
    } else if run.level.is_rtl() {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    };
    // `vert` alone, as CSS sets upright text: `vrt2` also swaps proportional
    // Latin for glyphs its font has already turned, which stood upright Latin
    // sideways, and runs are turned here, by Unicode's orientations.
    let columns = [Feature::new(Tag::new(b"vert"), 1, ..)];
    let narrow = width.map(|width| [Feature::new(width, 1, ..)]);
    let features: &[Feature] = match (vertical, &narrow) {
        (true, _) => &columns,
        (false, Some(narrow)) => narrow,
        (false, None) => &[],
    };
    let span = &styled.spans[run.span];
    let font = &span.fonts[run.font];
    buffer.clear();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer.set_direction(direction);
    if let Some(script) = run.script {
        buffer.set_script(Some(script));
    }
    let plan = font.shape_plan(
        buffer.direction(),
        buffer.script(),
        buffer.language(),
        features,
    );
    harfrust::shape(
        &ShaperFont::new(&font.shaper),
        buffer,
        ShapeOptions::new()
            .plan(Some(&plan))
            .features(features)
            .point_size(Some(span.size)),
    )
    .context("failed to shape text with HarfRust")?;
    let scale = font.scale(span.size);
    let mut glyphs: Vec<Glyph> = buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .map(|(info, position)| Glyph {
            id: info.glyph_id,
            cluster: cluster(info.cluster as usize),
            advance: match vertical {
                true => (position.y_advance as f32 * scale).abs(),
                false => position.x_advance as f32 * scale,
            },
            x_offset: position.x_offset as f32 * scale,
            y_offset: position.y_offset as f32 * scale,
            unsafe_to_break: info.unsafe_to_break(),
        })
        .collect();
    // A faux bold takes the room its glyphs grow into, as HarfBuzz's synthetic
    // bold does. A full em keeps its width, as it does in a CJK family's own
    // bold, and so do a column's cells.
    let growth = font.growth(span.size);
    if growth > 0.0 && !vertical && run.orientation != Orientation::TateChuYoko {
        for glyphs in glyphs.chunk_by_mut(|a, b| a.cluster == b.cluster) {
            let advance: f32 = glyphs.iter().map(|glyph| glyph.advance).sum();
            if advance <= 0.0 || (advance - span.size).abs() < span.size * 1e-3 {
                continue;
            }
            for glyph in &mut *glyphs {
                glyph.x_offset += growth;
            }
            if let Some(last) = glyphs.last_mut() {
                last.advance += growth * 2.0;
            }
        }
    }
    Ok(glyphs)
}

/// `text` with `!` and `?` in their full-width forms, or in their
/// `proportional` ones, and the source byte of each byte of the result.
fn exclamations(text: &str, proportional: bool) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut clusters = Vec::with_capacity(text.len());
    for (offset, ch) in text.char_indices() {
        out.push(match (ch, proportional) {
            ('!', false) => '！',
            ('?', false) => '？',
            ('！', true) => '!',
            ('？', true) => '?',
            (other, _) => other,
        });
        clusters.resize(out.len(), offset);
    }
    (out, clusters)
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Align, Color, Frame};

    use super::*;
    use crate::{fonts::Library, text::style::Span};

    /// `text` in one 16-pixel span of an installed sans-serif, as point
    /// text.
    fn styled(text: &str) -> Styled {
        let font = [
            "Arial",
            "Segoe UI",
            "Noto Sans",
            "DejaVu Sans",
            "Liberation Sans",
        ]
        .into_iter()
        .find_map(|family| Library::new().family(family).ok())
        .expect("an installed sans-serif");
        Styled {
            text: text.to_owned(),
            spans: vec![Span {
                range: 0..text.len(),
                fonts: vec![font],
                size: 16.0,
                fill: Color::BLACK,
                stroke: None,
                orientation: koharu_scene::Orientation::Mixed,
            }],
            vertical: false,
            align: Align::Start,
            line_height: 1.2,
            hyphenation: None,
            chinese: false,
            frame: Frame::Point,
            ruby: Vec::new(),
            marks: Vec::new(),
        }
    }

    #[test]
    fn a_vertical_exclamation_stands_in_its_font_without_a_full_width_form() -> Result<()> {
        let mut styled = styled("Dogo!");
        let font = &styled.spans[0].fonts[0];
        assert!(font.covers("!") && !font.covers("！"));
        styled.vertical = true;
        let shaped = ShapedText::new(&styled)?;
        let mark = shaped
            .glyphs
            .iter()
            .find(|glyph| glyph.cluster == 4)
            .context("the mark is shaped")?;
        assert_ne!(mark.id, 0);
        Ok(())
    }
}
