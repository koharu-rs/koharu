//! Each run shaped once with HarfRust, through one reused buffer, and each
//! span's hyphen.

use std::ops::Range;

use anyhow::{Context, Result};
use harfrust::{Buffer, Direction, Feature, ShapeOptions, ShaperFont, Tag};
use icu_properties::props::Script as IcuScript;
use unicode_bidi::{BidiInfo, Level, ParagraphInfo};

use super::{
    itemize::{Orientation, Run, itemize},
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
            glyphs.extend(shaped);
        }
        let mut hyphens = Vec::with_capacity(styled.spans.len());
        for span in 0..styled.spans.len() {
            hyphens.push(match styled.hyphenation {
                Some(_) => hyphen(styled, &mut buffer, span, &mut glyphs)?,
                None => None,
            });
        }
        Ok(Self {
            styled,
            paragraphs,
            runs,
            glyphs,
            hyphens,
            uncovered,
        })
    }

    pub(super) fn glyphs(&self, run: &Run) -> &[Glyph] {
        &self.glyphs[run.glyphs.clone()]
    }

    pub(super) fn font(&self, run: &Run) -> &Font {
        &self.styled.spans[run.span].fonts[run.font]
    }

    /// The advance of span `span`'s hyphen.
    pub(super) fn hyphen(&self, span: usize) -> f32 {
        self.hyphens[span].as_ref().map_or(0.0, |run| {
            self.glyphs(run).iter().map(|glyph| glyph.advance).sum()
        })
    }

    /// `range` of `run` shaped on its own, as a line's end or start where
    /// breaking changes its shaping.
    pub(super) fn reshape(
        &self,
        buffer: &mut Buffer,
        run: &Run,
        range: Range<usize>,
    ) -> Result<Vec<Glyph>> {
        shape_run(self.styled, buffer, run, range)
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

/// `range` of `run` shaped on its own.
fn shape_run(
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
    shape_text(styled, buffer, &text, run, |cluster| {
        range.start + clusters.get(cluster).copied().unwrap_or(0)
    })
}

/// `text` shaped as `run` says, each glyph's cluster mapped by `cluster` from
/// a byte of `text` to one of the text.
fn shape_text(
    styled: &Styled,
    buffer: &mut Buffer,
    text: &str,
    run: &Run,
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
    let features = [
        Feature::new(Tag::new(b"vert"), 1, ..),
        Feature::new(Tag::new(b"vrt2"), 1, ..),
    ];
    let features: &[Feature] = if vertical { &features } else { &[] };
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
        &ShaperFont::new(font.shaper()),
        buffer,
        ShapeOptions::new()
            .plan(Some(&plan))
            .features(features)
            .point_size(Some(span.size)),
    )
    .context("failed to shape text with HarfRust")?;
    let scale = font.scale(span.size);
    Ok(buffer
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
        .collect())
}

/// Span `span`'s hyphen, shaped in the first of its fonts that has one, its
/// glyphs added to `glyphs`; `None` when the span has no fonts.
fn hyphen(
    styled: &Styled,
    buffer: &mut Buffer,
    span: usize,
    glyphs: &mut Vec<Glyph>,
) -> Result<Option<Run>> {
    let fonts = &styled.spans[span].fonts;
    if fonts.is_empty() {
        return Ok(None);
    }
    let mut run = Run {
        range: 0..0,
        span,
        font: fonts.iter().position(|font| font.covers("-")).unwrap_or(0),
        level: Level::ltr(),
        script: harfrust_script(IcuScript::Common),
        orientation: Orientation::Upright,
        glyphs: 0..0,
    };
    let shaped = shape_text(styled, buffer, "-", &run, |_| 0)?;
    run.glyphs = glyphs.len()..glyphs.len() + shaped.len();
    glyphs.extend(shaped);
    Ok(Some(run))
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
    use super::*;
    use crate::{
        fonts::FontSystem,
        text::testing::{primary, styled},
    };

    #[test]
    fn arabic_shapes_as_one_run() -> Result<()> {
        // "مرحبا" (Marhaba)
        let styled = styled(
            "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627}",
            vec![primary(&mut FontSystem::new())],
        );
        let shaped = ShapedText::new(&styled)?;
        assert_eq!(shaped.runs.len(), 1);
        assert!(shaped.runs[0].level.is_rtl());
        Ok(())
    }

    #[test]
    fn scripts_shape_apart_in_logical_order() -> Result<()> {
        // Arabic, then Hebrew.
        let styled = styled(
            "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} \u{05e9}\u{05dc}\u{05d5}\u{05dd}",
            vec![primary(&mut FontSystem::new())],
        );
        let shaped = ShapedText::new(&styled)?;
        assert!(shaped.runs.len() >= 2);
        assert!(shaped.runs[0].range.start < shaped.runs[shaped.runs.len() - 1].range.start);
        Ok(())
    }

    #[test]
    fn a_vertical_exclamation_stands_in_its_font_without_a_full_width_form() -> Result<()> {
        let font = primary(&mut FontSystem::new());
        assert!(font.has_glyph('!') && !font.has_glyph('！'));
        let mut styled = styled("Dogo!", vec![font]);
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
