//! Placed lines arranged into glyph positions: each line's runs in visual
//! order, aligned or justified in its room, trailing white space hanging
//! past its end, and vertical text's upright groups set in one cell.

use std::ops::Range;

use anyhow::Result;
use harfrust::Buffer;
use koharu_scene::Align;
use unicode_bidi::{BidiInfo, Level};
use vello::kurbo::Rect;

use super::{
    ResolvedText, TextRun,
    breaks::{Break, Segment},
    fit::{Placed, PlacedLine},
    itemize::{Orientation, Run},
    lines,
    room::physical,
    shape::{Glyph, ShapedText},
};
use crate::fonts::Font;

/// A text's glyphs positioned in its own coordinates, with what laying it
/// out resolved.
pub(crate) struct TextLayout {
    /// One for each of `resolved.runs`, in order.
    pub runs: Vec<GlyphRun>,
    pub resolved: ResolvedText,
    /// What the text covers, its strokes included, in its own coordinates.
    pub bounds: Rect,
}

/// The font and positioned glyphs of one of a layout's runs.
pub(crate) struct GlyphRun {
    pub font: Font,
    pub glyphs: Vec<vello::Glyph>,
    /// Its glyphs are turned a quarter clockwise around their origins, as
    /// vertical text sets Latin.
    pub sideways: bool,
}

/// A glyph as positioned: its run's span, font, and whether it's turned
/// sideways; the glyph; and the text offset of its cluster.
type Positioned = (usize, usize, bool, vello::Glyph, usize);

/// A run as one line holds it, with its glyphs there.
type Piece<'a> = (&'a Run, Vec<Glyph>);

pub(super) fn arrange(
    shaped: &ShapedText,
    segments: &[Segment],
    placed: Placed,
) -> Result<TextLayout> {
    let styled = shaped.styled;
    let scale = placed.scale;
    let mut buffer = Buffer::new();
    let mut glyphs = Vec::new();
    let mut hyphens = Vec::new();
    let mut bounds: Option<Rect> = None;
    let mut baseline = None;
    for line in &placed.lines {
        let Some(last) = line.segments.end.checked_sub(1).map(|at| &segments[at]) else {
            continue;
        };
        if last.after == Break::Hyphen && !line.last {
            hyphens.push(last.range.end);
        }
        let used = lines::length(segments, line.segments.clone(), scale);
        let text = segments[line.segments.start].range.start..last.range.end;
        let pieces = pieces(shaped, &mut buffer, line, last, text.clone())?;
        // Trailing white space hangs past the line's end.
        let visible = text.start..last.visible_end;
        let first = position(shaped, line, scale, used, visible, pieces, &mut glyphs);
        baseline = baseline.or(first);
        let line_box = physical(
            styled.vertical,
            f64::from(line.start),
            f64::from(line.top),
            f64::from(line.start + line.room.max(used)),
            f64::from(line.top + line.pitch),
        );
        bounds = Some(bounds.map_or(line_box, |b| b.union(line_box)));
    }

    let (runs, text_runs) = runs(shaped, &glyphs, scale);
    let stroke = styled
        .spans
        .iter()
        .filter_map(|span| span.stroke.map(|(_, width)| width * scale))
        .fold(0.0, f32::max);
    let utf16 = |at: &usize| utf16(&styled.text, *at);
    Ok(TextLayout {
        runs,
        resolved: ResolvedText {
            runs: text_runs,
            frame: placed.frame,
            scale,
            hyphens: hyphens.iter().map(utf16).collect(),
            overflowed: placed.overflowed,
            baseline: baseline.unwrap_or_default(),
            align: styled.align,
            rtl: shaped.rtl_at(0),
            line_height: styled.line_height,
            uncovered: shaped.uncovered.clone(),
        },
        bounds: bounds
            .unwrap_or(placed.frame)
            .inflate(f64::from(stroke), f64::from(stroke)),
    })
}

/// The runs a line holds, in the order they show, with the hyphen it ends
/// with.
fn pieces<'a>(
    shaped: &'a ShapedText,
    buffer: &mut Buffer,
    line: &PlacedLine,
    last: &Segment,
    text: Range<usize>,
) -> Result<Vec<Piece<'a>>> {
    let mut pieces = line_runs(shaped, buffer, text)?;
    if !shaped.styled.vertical {
        pieces = visual_order(pieces);
    }
    if last.after == Break::Hyphen && !line.last {
        let at = last.visible_end.saturating_sub(1);
        if let Some(hyphen) = &shaped.hyphens[shaped.styled.span_at(at)] {
            let glyphs = shaped
                .glyphs(hyphen)
                .iter()
                .map(|glyph| Glyph {
                    cluster: at,
                    ..*glyph
                })
                .collect();
            pieces.push((hyphen, glyphs));
        }
    }
    Ok(pieces)
}

/// The runs in `range`, in logical order, each with its glyphs there. A run
/// the range cuts where HarfBuzz marks breaking unsafe is shaped again on
/// that side, as a line's end or start shapes on its own.
fn line_runs<'a>(
    shaped: &'a ShapedText,
    buffer: &mut Buffer,
    range: Range<usize>,
) -> Result<Vec<Piece<'a>>> {
    let mut pieces = Vec::new();
    for run in &shaped.runs {
        if run.range.end <= range.start || run.range.start >= range.end {
            continue;
        }
        let glyphs = shaped.glyphs(run);
        let unsafe_cut = |at: usize| {
            run.range.start < at
                && at < run.range.end
                && glyphs
                    .iter()
                    .any(|glyph| glyph.cluster == at && glyph.unsafe_to_break)
        };
        let piece = if unsafe_cut(range.start) || unsafe_cut(range.end) {
            let part = run.range.start.max(range.start)..run.range.end.min(range.end);
            shaped.reshape(buffer, run, part)?
        } else {
            glyphs
                .iter()
                .filter(|glyph| range.contains(&glyph.cluster))
                .copied()
                .collect()
        };
        pieces.push((run, piece));
    }
    Ok(pieces)
}

/// A line's pieces in the order they show, by their bidi levels (the
/// Unicode Bidirectional Algorithm's rule L2). A right-to-left piece's
/// glyphs are already in visual order. Trailing white space, whose level
/// rule L1 resets, hangs past the line's end and isn't drawn.
fn visual_order(pieces: Vec<Piece<'_>>) -> Vec<Piece<'_>> {
    let levels: Vec<Level> = pieces.iter().map(|(run, _)| run.level).collect();
    let mut pieces: Vec<Option<Piece<'_>>> = pieces.into_iter().map(Some).collect();
    BidiInfo::reorder_visual(&levels)
        .into_iter()
        .map(|at| pieces[at].take().expect("each piece shows once"))
        .collect()
}

/// Positions a line's glyphs at `scale` into `glyphs`, `used` being its
/// length and `visible` its text before trailing white space. Returns where
/// its first piece stands: its baseline's y in horizontal text, and its
/// column's centre's x in vertical text.
fn position(
    shaped: &ShapedText,
    line: &PlacedLine,
    scale: f32,
    used: f32,
    visible: Range<usize>,
    pieces: Vec<Piece<'_>>,
    glyphs: &mut Vec<Positioned>,
) -> Option<f32> {
    let styled = shaped.styled;
    let vertical = styled.vertical;
    let hang = visible.end;
    let (mut pen, justify) = spacing(shaped, line, &pieces, used, visible);
    let block_center = line.top + line.pitch * 0.5;
    let mut first = None;
    for (run, piece) in pieces {
        let size = styled.spans[run.span].size * scale;
        let (ascent, descent) = shaped.font(run).extent(size);
        let baseline = line.top + (line.pitch - (ascent + descent)) * 0.5 + ascent;
        first.get_or_insert(if vertical { -block_center } else { baseline });
        let sideways = run.orientation == Orientation::Sideways;
        let mut push = |x: f32, y: f32, glyph: &Glyph| {
            let id = glyph.id;
            let drawn = vello::Glyph { id, x, y };
            glyphs.push((run.span, run.font, sideways, drawn, glyph.cluster));
        };
        if run.orientation == Orientation::TateChuYoko {
            // An upright group shares one cell, its glyphs side by side
            // across the column.
            let width: f32 = piece.iter().map(|g| g.advance * scale).sum();
            let mut across = -width * 0.5;
            let cell_baseline = pen + (size - (ascent + descent)) * 0.5 + ascent;
            for glyph in &piece {
                let x = -block_center + across + glyph.x_offset * scale;
                push(x, cell_baseline - glyph.y_offset * scale, glyph);
                across += glyph.advance * scale;
            }
            pen += size;
            continue;
        }
        // A sideways run's baseline runs down the column, as far left of its
        // centre line as halfway between the ascent and descent is above the
        // baseline, which centres the run across the column.
        let sideways_baseline = -block_center - (ascent - descent) * 0.5;
        for glyph in &piece {
            if glyph.cluster >= hang {
                continue;
            }
            let (x_offset, y_offset) = (glyph.x_offset * scale, glyph.y_offset * scale);
            match (vertical, sideways) {
                (false, _) => push(pen + x_offset, baseline - y_offset, glyph),
                (true, false) => push(-block_center + x_offset, pen - y_offset, glyph),
                // Turned clockwise, a glyph's up is the page's right.
                (true, true) => push(sideways_baseline + y_offset, pen + x_offset, glyph),
            }
            pen += glyph.advance * scale;
            if let Some((spaces_only, extra)) = justify
                && (!spaces_only || is_space(&styled.text, glyph.cluster))
            {
                pen += extra;
            }
        }
    }
    first
}

/// Where a line's pen starts, and when the line is justified, whether only
/// its spaces widen and by how much each gap does. `visible` is the line's
/// text before its trailing white space; its paragraph's direction says
/// which side the line starts on.
fn spacing(
    shaped: &ShapedText,
    line: &PlacedLine,
    pieces: &[Piece<'_>],
    used: f32,
    visible: Range<usize>,
) -> (f32, Option<(bool, f32)>) {
    let align = shaped.styled.align;
    let free = (line.room - used).max(0.0);
    let justify = align == Align::Justify && !line.last;
    let offset = match (align, shaped.rtl_at(visible.start)) {
        (Align::Center, _) => free * 0.5,
        (Align::Start | Align::Justify, false) | (Align::End, true) => 0.0,
        (Align::End, false) | (Align::Start | Align::Justify, true) => {
            if justify {
                0.0
            } else {
                free
            }
        }
    };
    let gaps = justify.then(|| {
        let text = &shaped.styled.text;
        let spaces = pieces
            .iter()
            .flat_map(|(_, glyphs)| glyphs)
            .filter(|glyph| glyph.cluster < visible.end && is_space(text, glyph.cluster))
            .count();
        // Without spaces, every glyph spreads apart.
        let (spaces_only, count) = match spaces {
            0 => (
                false,
                pieces
                    .iter()
                    .map(|(_, glyphs)| glyphs.len())
                    .sum::<usize>()
                    .saturating_sub(1),
            ),
            spaces => (true, spaces),
        };
        (
            spaces_only,
            if count > 0 { free / count as f32 } else { 0.0 },
        )
    });
    (line.start + offset, gaps)
}

/// `glyphs` grouped into runs of one span, face and orientation, with those
/// runs as `ResolvedText` keeps them.
fn runs(shaped: &ShapedText, glyphs: &[Positioned], scale: f32) -> (Vec<GlyphRun>, Vec<TextRun>) {
    let styled = shaped.styled;
    let mut glyph_runs = Vec::new();
    let mut text_runs = Vec::new();
    for run in glyphs.chunk_by(|a, b| (a.0, a.1, a.2) == (b.0, b.1, b.2)) {
        let span = &styled.spans[run[0].0];
        let font = &span.fonts[run[0].1];
        let clusters = run.iter().map(|glyph| glyph.4);
        let start = clusters.clone().min().expect("a run has a glyph");
        let end = clusters.max().expect("a run has a glyph") + 1;
        text_runs.push(TextRun {
            range: utf16(&styled.text, start)..utf16(&styled.text, end),
            post_script_name: font.post_script_name().to_owned(),
            size: span.size * scale,
            fill: span.fill,
            stroke: span.stroke.map(|(color, width)| (color, width * scale)),
        });
        glyph_runs.push(GlyphRun {
            font: font.clone(),
            glyphs: run.iter().map(|glyph| glyph.3).collect(),
            sideways: run[0].2,
        });
    }
    (glyph_runs, text_runs)
}

fn is_space(text: &str, offset: usize) -> bool {
    text[offset..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
}

/// The UTF-16 offset of byte `offset`, rounded up to a character boundary.
fn utf16(text: &str, offset: usize) -> usize {
    text[..text.ceil_char_boundary(offset)]
        .encode_utf16()
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fonts::FontSystem,
        text::testing::{primary, styled},
    };

    #[test]
    fn a_right_to_left_line_shows_its_first_words_rightmost() -> Result<()> {
        // Hebrew, which makes the paragraph right to left, then English.
        let text = "\u{05e9}\u{05dc}\u{05d5}\u{05dd} \u{05e2}\u{05d5}\u{05dc}\u{05dd} and more";
        let styled = styled(text, vec![primary(&mut FontSystem::new())]);
        let shaped = ShapedText::new(&styled)?;
        let shown = visual_order(line_runs(&shaped, &mut Buffer::new(), 0..text.len())?);
        // The English shows leftmost, and the Hebrew it follows rightmost.
        assert!(!shown[0].0.level.is_rtl());
        assert_eq!(shown.last().map(|(run, _)| run.range.start), Some(0));
        Ok(())
    }

    #[test]
    fn a_line_cut_where_breaking_is_unsafe_shapes_that_side_alone() -> Result<()> {
        // A kerning pair, V and A, crosses the cut.
        let styled = styled("AVAV", vec![primary(&mut FontSystem::new())]);
        let shaped = ShapedText::new(&styled)?;
        let run = &shaped.runs[0];
        assert!(
            shaped
                .glyphs(run)
                .iter()
                .any(|glyph| glyph.cluster == 2 && glyph.unsafe_to_break)
        );
        let mut buffer = Buffer::new();
        let held = line_runs(&shaped, &mut buffer, 0..2)?;
        let alone = shaped.reshape(&mut buffer, run, 0..2)?;
        let advance = |glyphs: &[Glyph]| glyphs.iter().map(|glyph| glyph.advance).sum::<f32>();
        assert_eq!(advance(&held[0].1), advance(&alone));
        Ok(())
    }

    #[test]
    fn a_left_to_right_paragraph_after_a_right_to_left_one_starts_on_the_left() -> Result<()> {
        // Hebrew, then English on its own line.
        let text = "\u{05e9}\u{05dc}\u{05d5}\u{05dd}\nHello";
        let styled = styled(text, vec![primary(&mut FontSystem::new())]);
        let shaped = ShapedText::new(&styled)?;
        let english = text.find('H').expect("English");
        let line = PlacedLine {
            segments: 1..2,
            start: 0.0,
            room: 200.0,
            top: 0.0,
            pitch: 20.0,
            last: true,
        };
        let pieces = line_runs(&shaped, &mut Buffer::new(), english..text.len())?;
        let (pen, _) = spacing(&shaped, &line, &pieces, 40.0, english..text.len());
        assert_eq!(pen, 0.0);
        Ok(())
    }

    #[test]
    fn sideways_latin_runs_down_its_column_centred_across_it() -> Result<()> {
        let font = primary(&mut FontSystem::new());
        let text = koharu_scene::Text {
            content: "AV".into(),
            direction: koharu_scene::Direction::Vertical,
            ..koharu_scene::Text::default()
        };
        let layout = crate::text::layout(&text, |_, _| Ok(vec![font.clone()]))?.expect("a layout");
        let run = &layout.runs[0];
        assert!(run.sideways);
        let [a, v] = run.glyphs[..] else {
            panic!("two glyphs");
        };
        // Down the column, its baseline left of the column's centre line by
        // half of what the ascent passes the descent.
        assert!(v.y > a.y && a.x == v.x);
        let (ascent, descent) = font.extent(24.0);
        let centre = layout.resolved.baseline;
        assert!((a.x - (centre - (ascent - descent) * 0.5)).abs() < 1e-3);
        Ok(())
    }
}
