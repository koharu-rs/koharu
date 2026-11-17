//! Placed lines arranged into glyph positions: each line's runs in visual
//! order, aligned or justified in its room, trailing white space hanging
//! past its end, and vertical text's upright groups set in one cell.

use std::ops::Range;

use anyhow::Result;
use harfrust::Buffer;
use koharu_scene::Align;
use kurbo::Rect;
use unicode_bidi::{BidiInfo, Level};

use super::{
    ResolvedText, TextRun,
    breaks::{Break, Segment},
    fit::{Placed, PlacedLine},
    itemize::{Orientation, Run},
    lettering::{LineMap, PlacedGlyph},
    lines,
    room::physical,
    shape::{Glyph, ShapedText, shape_run},
};
use crate::fonts::Font;

/// A text's glyphs positioned in its own coordinates, with what laying it
/// out resolved.
pub(crate) struct TextLayout {
    /// One for each of `resolved.runs`, in order.
    pub runs: Vec<GlyphRun>,
    /// Where each line's characters lie, for typing into it.
    pub lines: Vec<LineMap>,
    pub resolved: ResolvedText,
    /// What the text covers, its strokes included, in its own coordinates.
    pub bounds: Rect,
}

/// The font and positioned glyphs of one of a layout's runs.
pub(crate) struct GlyphRun {
    pub font: Font,
    pub glyphs: Vec<glifo::Glyph>,
    /// Its glyphs are turned a quarter clockwise around their origins, as
    /// vertical text sets Latin.
    pub sideways: bool,
}

/// A glyph as positioned: what groups it into a run, its run's span, font,
/// and whether it's turned sideways; the glyph; and the text offset of its
/// cluster.
type Positioned = ((usize, usize, bool), glifo::Glyph, usize);

/// A run as one line holds it, with its glyphs there.
type Piece<'a> = (&'a Run, Vec<Glyph>);

/// The layout of `shaped` text set in `placed` lines of its `segments`:
/// each line's runs in the order they show, its glyphs positioned, and
/// what laying it out resolved.
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
    let mut lines = Vec::new();
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
        let mut clusters = Vec::new();
        let (first, start) = position(
            shaped,
            line,
            scale,
            used,
            visible.clone(),
            pieces,
            (&mut glyphs, &mut clusters),
        );
        baseline = baseline.or(first);
        lines.push(LineMap::new(
            text.clone(),
            visible.end,
            start,
            (line.top, line.pitch),
            clusters,
        ));
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
        lines,
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
            shape_run(shaped.styled, buffer, run, part)?
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
/// length and `visible` its text before trailing white space, and adds each
/// of its text's glyphs to `clusters`: its cluster offset, where it lies
/// along the line, and whether its run is right to left. Returns where its
/// first piece stands, its baseline's y in horizontal text and its column's
/// centre's x in vertical text, and where its pen started along it.
fn position(
    shaped: &ShapedText,
    line: &PlacedLine,
    scale: f32,
    used: f32,
    visible: Range<usize>,
    pieces: Vec<Piece<'_>>,
    (glyphs, clusters): (&mut Vec<Positioned>, &mut Vec<PlacedGlyph>),
) -> (Option<f32>, f32) {
    let styled = shaped.styled;
    let vertical = styled.vertical;
    let hang = visible.end;
    // The line is aligned and starts without the space an opening bracket
    // leads it with.
    let lead = opening_space(shaped, &pieces, scale, visible.start);
    let (pen, justify) = spacing(shaped, line, &pieces, used - lead, visible);
    let mut pen = pen - lead;
    let start = pen;
    let block_center = line.top + line.pitch * 0.5;
    let mut first = None;
    for (run, piece) in pieces {
        // A hyphen ending the line is no character of the text's.
        let text = shaped.runs.iter().any(|own| std::ptr::eq(own, run));
        let rtl = run.level.is_rtl();
        let mut placed = |cluster: usize, along: Range<f32>| {
            if text {
                clusters.push((cluster, along, rtl));
            }
        };
        let size = styled.spans[run.span].size * scale;
        let (ascent, descent) = shaped.font(run).extent(size);
        let baseline = line.top + (line.pitch - (ascent + descent)) * 0.5 + ascent;
        first.get_or_insert(if vertical { -block_center } else { baseline });
        let sideways = run.orientation == Orientation::Sideways;
        let mut push = |x: f32, y: f32, glyph: &Glyph| {
            let id = glyph.id;
            let drawn = glifo::Glyph { id, x, y };
            glyphs.push(((run.span, run.font, sideways), drawn, glyph.cluster));
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
                placed(glyph.cluster, pen..pen + size);
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
            // A mark on a column's centre line is centred across it by its
            // ink, whichever way it stands, as JLREQ centres `！`, `？`, `…`,
            // and dashes; its font centres its em box, which a Western `…`
            // or a Chinese `！` inks off the middle of.
            let ink = (vertical && centred(&styled.text, glyph.cluster, styled.chinese))
                .then(|| shaped.font(run).ink(glyph.id, size))
                .flatten();
            // Chinese centres a stop along its cell too, which its font's
            // vertical form sets in the corner.
            let middle = styled.chinese && stop(&styled.text, glyph.cluster);
            match (vertical, sideways) {
                (false, _) => push(pen + x_offset, baseline - y_offset, glyph),
                (true, false) => {
                    let x = ink.map_or(-block_center + x_offset, |ink| {
                        -block_center - ((ink.x0 + ink.x1) * 0.5) as f32
                    });
                    let y = match ink {
                        Some(ink) if middle => {
                            pen + glyph.advance * scale * 0.5 + ((ink.y0 + ink.y1) * 0.5) as f32
                        }
                        _ => pen - y_offset,
                    };
                    push(x, y, glyph);
                }
                // Turned clockwise, a glyph's up is the page's right.
                (true, true) => {
                    let x = ink.map_or(sideways_baseline + y_offset, |ink| {
                        -block_center - ((ink.y0 + ink.y1) * 0.5) as f32
                    });
                    push(x, pen + x_offset, glyph);
                }
            }
            placed(glyph.cluster, pen..pen + glyph.advance * scale);
            pen += glyph.advance * scale;
            if let Some((spaces_only, extra)) = justify
                && (!spaces_only || is_space(&styled.text, glyph.cluster))
            {
                pen += extra;
            }
        }
    }
    (first, start)
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
    for run in glyphs.chunk_by(|a, b| a.0 == b.0) {
        let (span, font, sideways) = run[0].0;
        let span = &styled.spans[span];
        let font = &span.fonts[font];
        let clusters = run.iter().map(|glyph| glyph.2);
        let start = clusters.clone().min().expect("a run has a glyph");
        let end = clusters.max().expect("a run has a glyph") + 1;
        text_runs.push(TextRun {
            range: utf16(&styled.text, start)..utf16(&styled.text, end),
            post_script_name: font.post_script_name.clone(),
            size: span.size * scale,
            fill: span.fill,
            stroke: span.stroke.map(|(color, width)| (color, width * scale)),
        });
        glyph_runs.push(GlyphRun {
            font: font.clone(),
            glyphs: run.iter().map(|glyph| glyph.1).collect(),
            sideways,
        });
    }
    (glyph_runs, text_runs)
}

/// Whether the character at `offset` sits on a vertical column's centre
/// line, as JLREQ's hyphens (cl-03), dividing punctuation (cl-04), middle
/// dots (cl-05), and inseparable characters (cl-08) do, and as CLREQ sets
/// Chinese's interpunct, midline ellipsis, and connector; in `chinese`, its
/// stops too.
///
/// Deliberately left out: cl-08's vertical kana repeat marks, whose halves
/// join only as their font draws them; ASCII `:`, `;`, and `~`, which JLREQ
/// classes as Western characters (cl-27) on a sideways run's baseline; and,
/// but in Chinese, full stops (cl-06) and commas (cl-07), which keep their
/// fonts' vertical forms in the column's top right corner. ASCII `!` and `?`
/// count as cl-04, since vertical text sets them upright and full width.
fn centred(text: &str, offset: usize, chinese: bool) -> bool {
    (chinese && stop(text, offset))
        || text[offset..].chars().next().is_some_and(|ch| {
            matches!(
                ch,
                // cl-03
                '‐' | '〜' | '゠' | '–'
                // cl-04
                | '!' | '?' | '！' | '？' | '‼' | '⁇' | '⁈' | '⁉'
                // cl-05
                | '・' | '：' | '；'
                // cl-08, with `―`, which JLREQ notes some systems treat as `—`
                | '—' | '―' | '…' | '‥'
                // CLREQ's interpunct, midline ellipsis, and connector
                | '·' | '⋯' | '～'
            )
        })
}

/// The space before the ink of an opening bracket (JLREQ's cl-01) that
/// starts a left-to-right or vertical line at `head`: JLREQ counts the
/// bracket half an em wide with half an em of space before it, and starts
/// every line with no space, as JIS X 4051 does, so a full-width bracket's
/// first half em goes. A proportional bracket has none.
fn opening_space(shaped: &ShapedText, pieces: &[Piece<'_>], scale: f32, head: usize) -> f32 {
    let Some((run, glyphs)) = pieces.first() else {
        return 0.0;
    };
    let opening = matches!(
        shaped.styled.text[head..].chars().next(),
        Some(
            '‘' | '“'
                | '('
                | '（'
                | '〔'
                | '['
                | '［'
                | '{'
                | '｛'
                | '〈'
                | '《'
                | '「'
                | '『'
                | '【'
                | '⦅'
                | '｟'
                | '〘'
                | '〖'
                | '«'
                | '〝'
        )
    );
    match glyphs.first() {
        Some(glyph) if opening && glyph.cluster == head && !shaped.rtl_at(head) => {
            let em = shaped.styled.spans[run.span].size * scale;
            (glyph.advance * scale - em * 0.5).clamp(0.0, em * 0.5)
        }
        _ => 0.0,
    }
}

/// Whether the character at `offset` is a full stop (cl-06) or a comma
/// (cl-07), which Chinese sets in the middle of its cell, as CLREQ describes
/// Taiwan and Hong Kong setting every stop mark.
fn stop(text: &str, offset: usize) -> bool {
    matches!(
        text[offset..].chars().next(),
        Some('，' | '、' | '。' | '．')
    )
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
    use koharu_scene::Frame;

    use super::*;
    use crate::{
        fonts::Library,
        text::style::{Span, Styled},
    };

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
                fill: koharu_scene::Color::BLACK,
                stroke: None,
            }],
            vertical: false,
            align: Align::Start,
            line_height: 1.2,
            hyphenation: None,
            chinese: false,
            frame: Frame::Point,
        }
    }

    #[test]
    fn a_right_to_left_line_shows_its_first_words_rightmost() -> Result<()> {
        // Hebrew, which makes the paragraph right to left, then English.
        let text = "\u{05e9}\u{05dc}\u{05d5}\u{05dd} \u{05e2}\u{05d5}\u{05dc}\u{05dd} and more";
        let styled = styled(text);
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
        let styled = styled("AVAV");
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
        let alone = shape_run(&styled, &mut buffer, run, 0..2)?;
        let advance = |glyphs: &[Glyph]| glyphs.iter().map(|glyph| glyph.advance).sum::<f32>();
        assert_eq!(advance(&held[0].1), advance(&alone));
        Ok(())
    }

    #[test]
    fn a_left_to_right_paragraph_after_a_right_to_left_one_starts_on_the_left() -> Result<()> {
        // Hebrew, then English on its own line.
        let text = "\u{05e9}\u{05dc}\u{05d5}\u{05dd}\nHello";
        let styled = styled(text);
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
    fn a_mark_on_the_centre_line_centres_its_ink_across_the_column() -> Result<()> {
        let font = [
            "Microsoft YaHei",
            "Yu Gothic",
            "MS Gothic",
            "Noto Sans CJK SC",
            "PingFang SC",
        ]
        .into_iter()
        .find_map(|family| Library::new().family(family).ok())
        .expect("an installed CJK font");
        let text = koharu_scene::Text {
            content: "已…！".into(),
            direction: koharu_scene::Direction::Vertical,
            ..koharu_scene::Text::default()
        };
        let layout = crate::text::layout(&text, |_, _| Ok(vec![font.clone()]))?.expect("a layout");
        let centre = layout.resolved.baseline;
        // The ellipsis turns sideways and the exclamation stands upright;
        // either way its ink sits across the column's centre line.
        for (glyphs, run) in layout.runs.iter().zip(&layout.resolved.runs) {
            if run.range.start == 0 {
                continue;
            }
            for glyph in &glyphs.glyphs {
                let ink = glyphs.font.ink(glyph.id, run.size).expect("a mark has ink");
                let across = match glyphs.sideways {
                    true => glyph.x + (ink.y0 + ink.y1) as f32 * 0.5,
                    false => glyph.x + (ink.x0 + ink.x1) as f32 * 0.5,
                };
                assert!((across - centre).abs() < 0.01, "{across} against {centre}");
            }
        }
        Ok(())
    }

    /// Where each glyph of `content` in `language`, set vertically in the
    /// renderer's default fonts, inks on the page, from the top; and the
    /// layout.
    fn inks(content: &str, language: &str) -> Result<(Vec<Rect>, TextLayout)> {
        let families = crate::RendererConfig::default().font_families;
        let text = koharu_scene::Text {
            content: content.into(),
            direction: koharu_scene::Direction::Vertical,
            language: language.parse()?,
            ..koharu_scene::Text::default()
        };
        let layout = crate::text::layout(&text, |style, characters| {
            crate::fonts::chain(style, &families, characters, &text.language)
        })?
        .expect("a layout");
        let mut inks = Vec::new();
        for (glyphs, run) in layout.runs.iter().zip(&layout.resolved.runs) {
            for glyph in &glyphs.glyphs {
                let Some(ink) = glyphs.font.ink(glyph.id, run.size) else {
                    continue;
                };
                // Font units run y up; a sideways glyph turns a quarter
                // clockwise about its origin.
                let (x, y) = (f64::from(glyph.x), f64::from(glyph.y));
                inks.push(match glyphs.sideways {
                    true => Rect::new(x + ink.y0, y + ink.x0, x + ink.y1, y + ink.x1),
                    false => Rect::new(x + ink.x0, y - ink.y1, x + ink.x1, y - ink.y0),
                });
            }
        }
        inks.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        Ok((inks, layout))
    }

    #[test]
    fn a_wave_dash_runs_down_its_column() -> Result<()> {
        let (inks, _) = inks("吗\u{301c}？", "zh-Hans")?;
        let wave = inks[1];
        assert!(wave.height() > wave.width(), "{wave:?}");
        Ok(())
    }

    #[test]
    fn a_japanese_comma_sits_in_the_top_right_of_its_cell() -> Result<()> {
        let (inks, layout) = inks("雨、雪", "ja")?;
        let (comma, cell) = (inks[1], &layout.lines[0].clusters[1].along);
        let centre = layout.resolved.baseline;
        let middle = f64::from(cell.start + cell.end) * 0.5;
        assert!(
            comma.center().x > f64::from(centre),
            "{comma:?} against {centre}"
        );
        assert!(comma.center().y < middle, "{comma:?} against {middle}");
        Ok(())
    }

    #[test]
    fn a_chinese_comma_sits_in_the_middle_of_its_cell() -> Result<()> {
        let (inks, layout) = inks("了，哥", "zh-Hans")?;
        let (comma, cell) = (inks[1], &layout.lines[0].clusters[1].along);
        let centre = layout.resolved.baseline;
        let middle = f64::from(cell.start + cell.end) * 0.5;
        assert!(
            (comma.center().x - f64::from(centre)).abs() < 0.01,
            "{comma:?} against {centre}"
        );
        assert!(
            (comma.center().y - middle).abs() < 0.01,
            "{comma:?} against {middle}"
        );
        Ok(())
    }

    #[test]
    fn a_column_opening_with_a_bracket_starts_level_with_the_others() -> Result<()> {
        let (inks, layout) = inks("能继承\n『毒师』", "zh-Hans")?;
        // The right column's first ink, 能, and the left one's, 『.
        let middle = inks.iter().map(|ink| ink.center().x).sum::<f64>() / inks.len() as f64;
        let top = |right: bool| {
            inks.iter()
                .filter(|ink| (ink.center().x > middle) == right)
                .map(|ink| ink.y0)
                .fold(f64::MAX, f64::min)
        };
        let em = f64::from(layout.lines[0].pitch) / 1.2;
        assert!(
            (top(false) - top(true)).abs() < em * 0.15,
            "{} against {}",
            top(false),
            top(true)
        );
        Ok(())
    }

    #[test]
    fn sideways_latin_runs_down_its_column_centred_across_it() -> Result<()> {
        let font = styled("").spans[0].fonts[0].clone();
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
