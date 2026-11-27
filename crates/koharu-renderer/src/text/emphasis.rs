//! Emphasis marks: a span's mark beside each of its characters, as CSS sets
//! one "as if each character was assigned the mark as its ruby", centred on
//! it, at half its size, and upright in a column.

use std::ops::Range;

use anyhow::Result;
use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use icu_segmenter::GraphemeClusterSegmenter;
use koharu_scene::{Align, Frame, Orientation as Set};

use super::{
    itemize::Orientation,
    ruby::Ruby,
    shape::ShapedText,
    style::{Span, Styled},
};

/// A mark's size, of its character's: CSS's.
const SIZE: f32 = 0.5;

/// The punctuation CSS still marks: what decomposes to one of these.
const SYMBOLS: [&str; 15] = [
    "#", "%", "‰", "‱", "٪", "؉", "؊", "&", "⁊", "@", "§", "¶", "⁋", "⁓", "〽",
];

/// A span's mark, as a text of its own: its one character in the span's
/// fonts and paint at half its size, set as a row, which stands upright in a
/// column too.
pub(super) struct Mark {
    pub span: usize,
    pub text: Styled,
}

impl Mark {
    pub(super) fn new(styled: &Styled, span: usize, mark: char) -> Self {
        let base = &styled.spans[span];
        let text = mark.to_string();
        Self {
            span,
            text: Styled {
                spans: vec![Span {
                    range: 0..text.len(),
                    fonts: base.fonts.clone(),
                    size: base.size * SIZE,
                    fill: base.fill,
                    stroke: base.stroke,
                    orientation: Set::Mixed,
                }],
                text,
                vertical: false,
                align: Align::Center,
                line_height: 1.0,
                hyphenation: None,
                chinese: false,
                vietnamese: false,
                frame: Frame::Point,
                ruby: Vec::new(),
                marks: Vec::new(),
            },
        }
    }
}

/// A mark shaped, with the bytes of each character it marks, in text order.
pub(super) struct ShapedMark<'a> {
    pub mark: &'a Mark,
    pub text: ShapedText<'a>,
    pub bases: Vec<Range<usize>>,
}

impl ShapedMark<'_> {
    pub(super) fn size(&self) -> f32 {
        self.mark.text.spans[0].size
    }

    /// How far from its line's centre the near side of `base`'s mark lies:
    /// past its character's em box, or past a ruby on its side, as CSS sets
    /// marks outside ruby.
    pub(super) fn near(&self, styled: &Styled, base: &Range<usize>) -> f32 {
        let half = styled.spans[self.mark.span].size * 0.5;
        if under(styled) {
            return half;
        }
        styled
            .ruby
            .iter()
            .filter(|ruby| ruby.base.start < base.end && base.start < ruby.base.end)
            .map(Ruby::rise)
            .fold(half, f32::max)
    }
}

/// Whether marks go under a row, as CSS prefers for Chinese; otherwise over
/// a row or right of a column, on ruby's side, as it prefers for Japanese
/// and Korean.
pub(super) fn under(styled: &Styled) -> bool {
    styled.chinese && !styled.vertical
}

/// `shaped`'s marks, each shaped once, over each character of its span but
/// those CSS leaves out; a tate-chu-yoko cell, one character to CSS, takes
/// one.
pub(super) fn shape<'a>(shaped: &ShapedText<'a>) -> Result<Vec<ShapedMark<'a>>> {
    let styled = shaped.styled;
    styled
        .marks
        .iter()
        .map(|mark| {
            let mut bases = Vec::new();
            for run in shaped.runs.iter().filter(|run| run.span == mark.span) {
                if run.orientation == Orientation::TateChuYoko {
                    bases.push(run.range.clone());
                    continue;
                }
                let text = &styled.text[run.range.clone()];
                let boundaries: Vec<usize> =
                    GraphemeClusterSegmenter::new().segment_str(text).collect();
                bases.extend(
                    boundaries
                        .windows(2)
                        .filter(|pair| marked(&text[pair[0]..pair[1]]))
                        .map(|pair| run.range.start + pair[0]..run.range.start + pair[1]),
                );
            }
            Ok(ShapedMark {
                mark,
                text: ShapedText::new(&mark.text)?,
                bases,
            })
        })
        .collect()
}

/// Whether CSS marks `grapheme`: anything but a lone separator (Z*), control,
/// format, or unassigned character (Cc, Cf, Cn), or punctuation (P*) that
/// doesn't decompose to one of `SYMBOLS`, as `＃` does.
fn marked(grapheme: &str) -> bool {
    let mut chars = grapheme.chars();
    let (Some(first), None) = (chars.next(), chars.next()) else {
        return true;
    };
    let category = CodePointMapData::<GeneralCategory>::new().get(first);
    if GeneralCategoryGroup::Separator.contains(category)
        || matches!(
            category,
            GeneralCategory::Control | GeneralCategory::Format | GeneralCategory::Unassigned
        )
    {
        return false;
    }
    !GeneralCategoryGroup::Punctuation.contains(category) || {
        let decomposed = DecomposingNormalizerBorrowed::new_nfkd().normalize(grapheme);
        SYMBOLS.contains(&decomposed.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{CharStyle, Direction, Emphasis, RichText, Text, TextStyle};
    use kurbo::Rect;

    use crate::{
        fonts::Library,
        text::{TextLayout, layout},
    };

    /// `content` in `language` with dots on every character, set in an
    /// installed CJK font at 24 pixels: where each mark inks, from the top
    /// and then the left, and the layout.
    fn marks(content: RichText, language: &str, direction: Direction) -> (Vec<Rect>, TextLayout) {
        let font = [
            "Yu Gothic",
            "Microsoft YaHei",
            "MS Gothic",
            "Noto Sans CJK JP",
        ]
        .into_iter()
        .find_map(|family| Library::new().family(family).ok())
        .expect("an installed CJK font");
        let text = Text {
            content,
            style: TextStyle {
                chars: CharStyle {
                    emphasis: Some(Emphasis::Dot),
                    ..CharStyle::default()
                },
                ..TextStyle::default()
            },
            direction,
            language: language.parse().expect("a language"),
            ..Text::default()
        };
        let layout = layout(&text, |_, _| Ok(vec![font.clone()]))
            .expect("it lays out")
            .expect("a layout");
        let first = layout.runs.len() - layout.resolved.emphasis.len();
        let mut inks: Vec<Rect> = (layout.runs[first..].iter())
            .zip(&layout.resolved.emphasis)
            .flat_map(|(glyphs, run)| {
                glyphs.glyphs.iter().filter_map(move |glyph| {
                    let ink = glyphs.font.ink(glyph.id, run.size)?;
                    let (x, y) = (f64::from(glyph.x), f64::from(glyph.y));
                    Some(Rect::new(x + ink.x0, y - ink.y1, x + ink.x1, y - ink.y0))
                })
            })
            .collect();
        inks.sort_by(|a, b| {
            (a.center().y, a.center().x)
                .partial_cmp(&(b.center().y, b.center().x))
                .expect("numbers")
        });
        (inks, layout)
    }

    #[test]
    fn a_column_s_marks_sit_right_of_each_character_outside_its_ruby() {
        let content = RichText(vec![
            koharu_scene::Span {
                insert: "漢字".into(),
                ruby: Some("かんじ".into()),
                ..koharu_scene::Span::default()
            },
            koharu_scene::Span {
                insert: "、ね12".into(),
                ..koharu_scene::Span::default()
            },
        ]);
        let text = content.text();
        let (inks, layout) = marks(content, "ja", Direction::Vertical);
        // None on the comma; one on the two digits' cell.
        assert_eq!(inks.len(), 4, "{inks:?}");
        let centre = f64::from(layout.resolved.baseline);
        // Past half the em, or past the ruby's em box on that, by half a mark.
        let besides = [("漢", 30.0), ("字", 30.0), ("ね", 18.0), ("12", 18.0)];
        for (ink, (part, away)) in inks.iter().zip(besides) {
            let at = text.find(part).expect("in the text");
            let cell = &(layout.lines[0].clusters.iter())
                .find(|cluster| cluster.text.contains(&at))
                .expect("a cluster")
                .along;
            let middle = f64::from(cell.start + cell.end) * 0.5;
            assert!(
                (ink.center().y - middle).abs() < 0.01,
                "{part}: {ink:?} against {middle}"
            );
            let right = ink.center().x - centre;
            assert!(
                (right - away).abs() < 0.01,
                "{part}: {right} against {away}"
            );
        }
    }

    #[test]
    fn a_chinese_row_s_marks_sit_under_it() {
        let (inks, layout) = marks("中文".into(), "zh-Hans", Direction::Horizontal);
        let line = &layout.lines[0];
        let middle = f64::from(line.top + line.pitch * 0.5);
        assert_eq!(inks.len(), 2);
        for ink in inks {
            assert!(
                (ink.center().y - middle - 18.0).abs() < 0.01,
                "{ink:?} against {middle}"
            );
        }
    }
}
