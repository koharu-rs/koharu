//! Ruby: each a text of its own, shaped beside its base. Whichever of the two
//! is shorter is spread to the other's length, so their centres line up and a
//! ruby never reaches over its base's neighbours, as W3C's rules for simple
//! ruby place them.

use std::ops::Range;

use anyhow::Result;
use icu_properties::{CodePointMapData, props::EastAsianWidth};
use koharu_scene::{Align, Frame, Orientation as Set, RichText};

use super::{
    itemize::{Orientation, Run},
    shape::ShapedText,
    style::{Span, Styled},
};

/// A ruby's size, of its base's: CSS's, JLREQ's, and InDesign's default.
const SIZE: f32 = 0.5;

/// A ruby styled after its base, the bytes of the text it's set over.
pub(super) struct Ruby {
    pub base: Range<usize>,
    pub text: Styled,
}

impl Ruby {
    /// How far it reaches above the centre of its base, which the line centres:
    /// half the base's em box, and its own on that.
    pub(super) fn rise(&self) -> f32 {
        self.size() / SIZE * 0.5 + self.size()
    }

    pub(super) fn size(&self) -> f32 {
        self.text.spans[0].size
    }
}

pub(super) struct ShapedRuby<'a> {
    pub ruby: &'a Ruby,
    pub text: ShapedText<'a>,
}

/// The ruby of `content`, which `styled` holds: in the fonts and paint its
/// base starts with.
pub(super) fn style(content: &RichText, styled: &Styled) -> Vec<Ruby> {
    content
        .ruby()
        .into_iter()
        .filter_map(|(base, ruby)| {
            let spans = || {
                styled
                    .spans
                    .iter()
                    .filter(|span| span.range.start < base.end && base.start < span.range.end)
            };
            let first = spans().next()?;
            let size = spans().map(|span| span.size).fold(0.0, f32::max);
            let span = Span {
                range: 0..ruby.len(),
                fonts: first.fonts.clone(),
                size: size * SIZE,
                fill: first.fill,
                stroke: first.stroke,
                // A ruby stands as its base does, but never as one cell.
                orientation: match first.orientation {
                    Set::TateChuYoko => Set::Mixed,
                    orientation => orientation,
                },
            };
            let text = Styled {
                text: ruby.to_owned(),
                spans: vec![span],
                vertical: styled.vertical,
                align: Align::Center,
                line_height: 1.0,
                hyphenation: None,
                chinese: styled.chinese,
                vietnamese: false,
                frame: Frame::Point,
                ruby: Vec::new(),
                marks: Vec::new(),
            };
            Some(Ruby { base, text })
        })
        .collect()
}

/// `shaped`'s ruby, each shaped, and spread over its base unless its base is
/// the shorter, which then spreads under it.
pub(super) fn shape<'a>(shaped: &mut ShapedText<'a>) -> Result<Vec<ShapedRuby<'a>>> {
    let styled = shaped.styled;
    let mut shaped_ruby = Vec::with_capacity(styled.ruby.len());
    for ruby in &styled.ruby {
        let mut text = ShapedText::new(&ruby.text)?;
        let whole = 0..ruby.text.text.len();
        let extra = length(&text, &whole) - length(shaped, &ruby.base);
        match extra > 0.0 {
            true => spread(shaped, &ruby.base, extra),
            false => spread(&mut text, &whole, -extra),
        }
        shaped_ruby.push(ShapedRuby { ruby, text });
    }
    Ok(shaped_ruby)
}

/// The length of `range` of `shaped`'s text, which starts and ends with runs,
/// set on one line.
pub(super) fn length(shaped: &ShapedText, range: &Range<usize>) -> f32 {
    shaped
        .runs
        .iter()
        .filter(|run| within(run, range))
        .map(|run| match run.orientation {
            Orientation::TateChuYoko => shaped.cell(run),
            _ => shaped.glyphs(run).iter().map(|glyph| glyph.advance).sum(),
        })
        .sum()
}

fn within(run: &Run, range: &Range<usize>) -> bool {
    range.start <= run.range.start && run.range.end <= range.end
}

/// A cluster of a run: its glyphs in `ShapedText::glyphs`, and whether it's
/// full-width.
struct Cluster {
    run: usize,
    glyphs: Range<usize>,
    wide: bool,
}

fn clusters(shaped: &ShapedText, range: &Range<usize>) -> Vec<Cluster> {
    let widths = CodePointMapData::<EastAsianWidth>::new();
    let mut clusters = Vec::new();
    for (index, run) in shaped.runs.iter().enumerate() {
        if !within(run, range) {
            continue;
        }
        if run.orientation == Orientation::TateChuYoko {
            clusters.push(Cluster {
                run: index,
                glyphs: run.glyphs.clone(),
                wide: true,
            });
            continue;
        }
        let mut start = run.glyphs.start;
        for glyphs in shaped.glyphs(run).chunk_by(|a, b| a.cluster == b.cluster) {
            let wide = shaped.styled.text[glyphs[0].cluster..]
                .chars()
                .next()
                .is_some_and(|ch| {
                    matches!(
                        widths.get(ch),
                        EastAsianWidth::Wide | EastAsianWidth::Fullwidth
                    )
                });
            clusters.push(Cluster {
                run: index,
                glyphs: start..start + glyphs.len(),
                wide,
            });
            start += glyphs.len();
        }
    }
    clusters
}

/// Lengthens `range` of `shaped`'s text by `extra`, which each full-width
/// character takes an equal share of, half before it and half after, as
/// CSS's `ruby-align: space-around` gives it. Other text stays together, as
/// simple ruby keeps Western letters, and takes one share.
fn spread(shaped: &mut ShapedText, range: &Range<usize>, extra: f32) {
    let clusters = clusters(shaped, range);
    let starts = |at: usize| at == 0 || clusters[at].wide || clusters[at - 1].wide;
    let shares = (0..clusters.len()).filter(|&at| starts(at)).count();
    let half = extra / shares.max(1) as f32 / 2.0;
    for (at, cluster) in clusters.iter().enumerate() {
        let before = if starts(at) { half } else { 0.0 };
        let after = match at + 1 == clusters.len() || starts(at + 1) {
            true => half,
            false => 0.0,
        };
        let run = &mut shaped.runs[cluster.run];
        if run.orientation == Orientation::TateChuYoko {
            run.space = (before, after);
            continue;
        }
        let down = shaped.styled.vertical && run.orientation == Orientation::Upright;
        let glyphs = &mut shaped.glyphs[cluster.glyphs.clone()];
        for glyph in &mut *glyphs {
            match down {
                true => glyph.y_offset -= before,
                false => glyph.x_offset += before,
            }
        }
        if let Some(last) = glyphs.last_mut() {
            last.advance += before + after;
        }
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Bounds, CharStyle, Direction, Frame, RichText, Text, TextStyle};

    use crate::{
        fonts::Library,
        text::{TextLayout, layout},
    };

    /// `content` in an installed sans-serif at 20 pixels, a line no taller
    /// than its letters.
    fn laid(content: RichText, frame: Frame, direction: Direction) -> TextLayout {
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
        let text = Text {
            content,
            style: TextStyle {
                chars: CharStyle {
                    size: Some(20.0),
                    ..CharStyle::default()
                },
                line_height: Some(1.0),
                ..TextStyle::default()
            },
            frame,
            direction,
            ..Text::default()
        };
        layout(&text, |_, _| Ok(vec![font.clone()]))
            .expect("a layout")
            .expect("characters")
    }

    #[test]
    fn a_ruby_is_centred_over_its_base_and_never_over_its_neighbours() {
        let text = RichText::from("a b c");
        let along = |laid: &TextLayout| -> Vec<(f32, f32)> {
            let clusters = &laid.lines[0].clusters;
            clusters
                .iter()
                .map(|c| (c.along.start, c.along.end))
                .collect()
        };
        let plain = laid(text.clone(), Frame::Point, Direction::Horizontal);
        // A longer ruby makes its base's room, which the base is centred in.
        let longer = text.annotated(2..3, Some("wwww"));
        let long = laid(longer.clone(), Frame::Point, Direction::Horizontal);
        let [run] = &long.resolved.ruby[..] else {
            panic!("{:?}", long.resolved.ruby);
        };
        assert_eq!((run.range.clone(), run.size), (2..3, 10.0));
        let (before, after) = (along(&plain), along(&long));
        let extra = (after[2].1 - after[2].0) - (before[2].1 - before[2].0);
        assert!(extra > 10.0, "{extra}");
        assert!((after[3].0 - before[3].0 - extra).abs() < 1e-3);
        let ruby = long.runs.last().expect("the ruby's glyphs");
        assert!((ruby.glyphs[0].x - after[2].0).abs() < 1e-3);
        let b = |laid: &TextLayout| laid.runs[1].glyphs[0].x;
        assert!((b(&long) - after[2].0 - extra / 2.0).abs() < 1e-3);
        // Its base stands on the line, and it on the base.
        assert!(ruby.glyphs[0].y < long.runs[0].glyphs[0].y);
        assert_eq!(long.lines[0].pitch, 20.0);
        // A shorter one moves nothing.
        let short = text.annotated(0..5, Some("i"));
        let short = laid(short, Frame::Point, Direction::Horizontal);
        assert_eq!(along(&short), before);
        // In a column it stands on the right.
        let column = laid(longer, Frame::Point, Direction::Vertical);
        let (ruby, letters) = (&column.runs[3].glyphs[0], &column.runs[0].glyphs[0]);
        assert!(ruby.x > letters.x, "{} {}", ruby.x, letters.x);
    }

    #[test]
    fn a_base_never_breaks_and_opens_only_its_own_line() {
        let frame = Frame::Box {
            bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 400.0,
            },
            fit: Default::default(),
            anchor: Default::default(),
        };
        let text = RichText::from("one two six ten");
        let lines = |laid: &TextLayout| -> Vec<String> {
            let lines = laid.lines.iter();
            lines
                .map(|line| text.text()[line.text.clone()].to_owned())
                .collect()
        };
        let plain = laid(text.clone(), frame.clone(), Direction::Horizontal);
        assert_eq!(lines(&plain), ["one two ", "six ten"]);
        let annotated = text.annotated(4..11, Some("2 6"));
        let annotated = laid(annotated, frame, Direction::Horizontal);
        assert_eq!(lines(&annotated), ["one ", "two six ", "ten"]);
        // A line with ruby holds it and its base, one on the other.
        let tops: Vec<f32> = annotated.lines.iter().map(|line| line.top).collect();
        assert_eq!(tops, [0.0, 30.0, 50.0]);
    }

    #[test]
    fn ruby_clears_the_line_above_beside_larger_letters() {
        let large = CharStyle {
            size: Some(30.0),
            ..CharStyle::default()
        };
        let text = RichText::from("up\nbig low").marked(0..6, &large);
        let text = text.annotated(7..10, Some("ow"));
        let laid = laid(text, Frame::Point, Direction::Horizontal);
        let lines = &laid.lines;
        // Half its base's 20 and its own 10, over the line's centre.
        let centre = lines[1].top + lines[1].pitch * 0.5;
        let above = lines[0].top + lines[0].pitch;
        assert!(centre - 20.0 >= above - 1e-3, "{centre} under {above}");
    }
}
