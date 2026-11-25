//! Where lines may break: Unicode line breaking, with Chinese words kept
//! whole, Korean broken between words, and words at hyphenation points; and
//! the text between two breaks measured from its glyphs.

use std::{ops::Range, sync::LazyLock};

use hypher::{Lang, hyphenate_bounded};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup, LineBreak, Script as IcuScript},
};
use icu_segmenter::{
    LineSegmenter,
    options::{LineBreakOptions, LineBreakWordOption},
};
use jieba_rs::Jieba;

use super::{itemize::Orientation, shape::ShapedText};

static JIEBA: LazyLock<Jieba> = LazyLock::new(Jieba::new);

/// Loads jieba's dictionary, which takes about 50 ms, so the first Chinese
/// text needn't wait for it.
pub(crate) fn preload_dictionary() {
    LazyLock::force(&JIEBA);
}

/// Photoshop's hyphenation default keeps two letters on each side.
const HYPHENATION_KEEPS: usize = 2;
/// What a mandatory break ends with, which belongs to no line.
pub(super) const LINE_ENDS: [char; 5] = ['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'];

/// Where lines may break: between words, then also at each word's middle
/// hyphenation point, so no word takes two hyphens, then at all of them.
/// Never elsewhere in a word, as a letterer never splits one without a
/// hyphen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Breaking {
    Words,
    Halves,
    Hyphens,
}

/// What may end a line after a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Break {
    /// A line break in the text.
    Mandatory,
    /// Between words, or between CJK characters.
    Normal,
    /// Inside a word at a hyphenation point; a hyphen shows if a line ends here.
    Hyphen,
    /// The end of the text.
    End,
}

/// Text between two break opportunities, measured along the inline axis at
/// the set size.
#[derive(Clone, Debug)]
pub(super) struct Segment {
    /// Its text, without a line break it ends with.
    pub range: Range<usize>,
    /// Where its trailing white space starts.
    pub visible_end: usize,
    pub advance: f32,
    /// Trailing white space, which hangs past a line's end.
    pub trailing: f32,
    /// The hyphen drawn when a line ends after a `Hyphen` break.
    pub hyphen: f32,
    pub after: Break,
    /// It ends with punctuation, the best place for a break.
    pub punctuation: bool,
    /// Its largest span's size.
    pub size: f32,
}

/// A piece of text between break opportunities: its range, without a line
/// break it ends with, where the next piece starts, and what may end a line
/// after it.
type Cut = (Range<usize>, usize, Break);

/// The text cut at its break opportunities, as `breaking` allows with the
/// hyphenation points of `lang`, when it has some.
fn opportunities(text: &str, breaking: Breaking, lang: Option<Lang>) -> Vec<Cut> {
    let scripts = CodePointMapData::<IcuScript>::new();
    let mut options = LineBreakOptions::default();
    // Korean breaks between words, not syllables.
    if text.chars().any(|ch| scripts.get(ch) == IcuScript::Hangul) {
        options.word_option = Some(LineBreakWordOption::KeepAll);
    }
    let line_breaks = CodePointMapData::<LineBreak>::new();
    let mandatory = |offset: usize| {
        text[..offset].chars().next_back().is_some_and(|c| {
            matches!(
                line_breaks.get(c),
                LineBreak::MandatoryBreak
                    | LineBreak::CarriageReturn
                    | LineBreak::LineFeed
                    | LineBreak::NextLine
            )
        })
    };
    // Chinese breaks between jieba's words rather than every character: text
    // with Han or Bopomofo and no kana or Hangul, which Japanese and Korean
    // break by their own rules.
    let chinese = |text: &str| {
        text.chars()
            .map(|ch| scripts.get(ch))
            .try_fold(false, |chinese, script| match script {
                IcuScript::Han | IcuScript::Bopomofo => Some(true),
                IcuScript::Hiragana | IcuScript::Katakana | IcuScript::Hangul => None,
                _ => Some(chinese),
            })
            .unwrap_or(false)
    };
    let mut offset = 0;
    let words: Vec<Range<usize>> = match chinese(text) {
        true => JIEBA
            .cut(text, true)
            .into_iter()
            .filter_map(|token| {
                let start = offset;
                offset += token.word.len();
                chinese(token.word).then_some(start..offset)
            })
            .collect(),
        false => Vec::new(),
    };
    let offsets: Vec<usize> = LineSegmenter::new_auto(options)
        .segment_str(text)
        .filter(|&at| mandatory(at) || !words.iter().any(|w| at > w.start && at < w.end))
        .collect();
    let mut pieces = Vec::new();
    for pair in offsets.windows(2) {
        let (start, next) = (pair[0], pair[1]);
        let after = if next >= text.len() {
            Break::End
        } else if mandatory(next) {
            Break::Mandatory
        } else {
            Break::Normal
        };
        let end = match mandatory(next) {
            true => start + text[start..next].trim_end_matches(LINE_ENDS).len(),
            false => next,
        };
        let points = match lang {
            Some(lang) if !mandatory(next) => points(text, start..end, breaking, lang),
            _ => Vec::new(),
        };
        let mut start = start;
        for point in points {
            pieces.push((start..point, point, Break::Hyphen));
            start = point;
        }
        pieces.push((start..end, next, after));
    }
    pieces
}

/// Where `breaking` lets a hyphen end a line inside the piece `range`.
fn points(text: &str, range: Range<usize>, breaking: Breaking, lang: Lang) -> Vec<usize> {
    // A piece's word is its letters, between whatever else starts or ends
    // it. Photoshop hyphenates words of more than five letters.
    let piece = &text[range.clone()];
    let not_letter = |ch: char| !ch.is_alphabetic();
    let start = range.start + piece.len() - piece.trim_start_matches(not_letter).len();
    let word = start..start + piece.trim_matches(not_letter).len();
    let letters = &text[word.clone()];
    if breaking == Breaking::Words
        || !letters.chars().all(char::is_alphabetic)
        || letters.chars().count() < 6
    {
        return Vec::new();
    }
    let mut end = word.start;
    let points = hyphenate_bounded(letters, lang, HYPHENATION_KEEPS, HYPHENATION_KEEPS)
        .map(|syllable| {
            end += syllable.len();
            end
        })
        .filter(|point| *point < word.end);
    match breaking {
        Breaking::Halves => points
            .min_by_key(|point| (2 * point).abs_diff(word.start + word.end))
            .into_iter()
            .collect(),
        _ => points.collect(),
    }
}

/// The text's segments, broken as `breaking` allows and measured from the
/// glyphs it shaped into.
pub(super) fn segments(shaped: &ShapedText, breaking: Breaking) -> Vec<Segment> {
    let styled = shaped.styled;
    // Where the text may break, where a run starts or ends or a cluster
    // starts but not inside an upright group; and the advance of the text
    // before each byte, each glyph's at its cluster and an upright group's
    // cell at its start.
    let mut breakable = vec![false; styled.text.len() + 1];
    let mut advances = vec![0.0; styled.text.len() + 1];
    for run in &shaped.runs {
        breakable[run.range.start] = true;
        breakable[run.range.end] = true;
        if run.orientation == Orientation::TateChuYoko {
            advances[run.range.start] += f64::from(styled.spans[run.span].size);
            continue;
        }
        for glyph in shaped.glyphs(run) {
            breakable[glyph.cluster] = true;
            advances[glyph.cluster] += f64::from(glyph.advance);
        }
    }
    let mut sum = 0.0;
    for value in &mut advances {
        (*value, sum) = (sum, sum + *value);
    }
    // A cut inside a cluster, such as a ligature, or inside an upright group
    // joins the next piece.
    let mut segments = Vec::new();
    let mut joined = None;
    for (range, next, after) in opportunities(&styled.text, breaking, styled.hyphenation) {
        let start = joined.take().unwrap_or(range.start);
        if matches!(after, Break::Mandatory | Break::End) || breakable[next] {
            segments.push(measure(shaped, &advances, start..range.end, after));
        } else {
            joined = Some(start);
        }
    }
    segments
}

/// The segment `range` makes, measured by `advances`.
fn measure(shaped: &ShapedText, advances: &[f64], range: Range<usize>, after: Break) -> Segment {
    let styled = shaped.styled;
    let advance = |range: Range<usize>| (advances[range.end] - advances[range.start]) as f32;
    let visible_end = range.start + styled.text[range.clone()].trim_end().len();
    let categories = CodePointMapData::<GeneralCategory>::new();
    let punctuation = styled.text[range.start..visible_end]
        .chars()
        .next_back()
        .is_some_and(|ch| GeneralCategoryGroup::Punctuation.contains(categories.get(ch)));
    let size = shaped
        .runs
        .iter()
        .filter(|run| run.range.start < range.end && run.range.end > range.start)
        .map(|run| styled.spans[run.span].size)
        .reduce(f32::max)
        // An empty paragraph has no runs, and keeps its span's leading, as
        // Photoshop's does.
        .unwrap_or_else(|| styled.spans[styled.span_at(range.start)].size);
    Segment {
        visible_end,
        advance: advance(range.clone()),
        trailing: advance(visible_end..range.end),
        hyphen: match after {
            Break::Hyphen => shaped.hyphen(styled.span_at(range.end.saturating_sub(1))),
            _ => 0.0,
        },
        after,
        punctuation,
        size,
        range,
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Align, Color, Frame};

    use super::*;
    use crate::{
        fonts::Library,
        text::{
            shape::shape_run,
            style::{Span, Styled},
        },
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
                fill: Color::BLACK,
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

    /// The text of each piece.
    fn pieces(text: &str, breaking: Breaking) -> Vec<&str> {
        opportunities(text, breaking, Some(Lang::English))
            .into_iter()
            .map(|(range, ..)| &text[range])
            .collect()
    }

    #[test]
    fn a_line_break_ends_its_piece_and_belongs_to_no_line() {
        let text = "Hello, \nWorld!";
        assert_eq!(
            opportunities(text, Breaking::Words, None),
            [(0..7, 8, Break::Mandatory), (8..14, 14, Break::End)]
        );
    }

    #[test]
    fn chinese_word_segmentation_keeps_jieba_words_together() {
        let text = "\u{5357}\u{4eac}\u{5e02}\u{957f}\u{6c5f}\u{5927}\u{6865}";
        assert_eq!(
            pieces(text, Breaking::Words),
            vec![
                "\u{5357}\u{4eac}\u{5e02}",
                "\u{957f}\u{6c5f}\u{5927}\u{6865}",
            ]
        );
    }

    #[test]
    fn chinese_word_segmentation_does_not_resegment_kana_text() {
        let text = "\u{543e}\u{8f29}\u{306f}\u{732b}";
        assert_eq!(
            pieces(text, Breaking::Words),
            vec!["\u{543e}", "\u{8f29}", "\u{306f}", "\u{732b}",]
        );
    }

    #[test]
    fn hyphenation_does_not_invent_breaks_missing_from_the_patterns() {
        assert_eq!(pieces("strength", Breaking::Hyphens), ["strength"]);
    }

    #[test]
    fn hyphenation_keeps_two_letters_each_side_and_skips_short_words() {
        // English's own bounds keep three letters at a word's end, which
        // would leave "within" whole; "tower" has fewer than six letters.
        assert_eq!(
            pieces("tower within", Breaking::Hyphens),
            ["tower ", "with", "in"]
        );
    }

    #[test]
    fn a_halved_word_breaks_only_at_the_point_nearest_its_middle() {
        assert_eq!(
            pieces("Um... actually!", Breaking::Hyphens),
            ["Um... ", "ac", "tu", "al", "ly!"]
        );
        assert_eq!(
            pieces("Um... actually!", Breaking::Halves),
            ["Um... ", "actu", "ally!"]
        );
    }

    #[test]
    fn a_hyphenated_word_is_measured_as_shaped_once() -> anyhow::Result<()> {
        // A kerning pair, A and V, crosses its hyphenation point: car-a-van.
        let styled = Styled {
            hyphenation: Some(Lang::English),
            chinese: false,
            ..styled("CARAVAN")
        };
        let shaped = ShapedText::new(&styled)?;
        let total = |segments: Vec<Segment>| segments.iter().map(|s| s.advance).sum::<f32>();
        let word = total(segments(&shaped, Breaking::Words));
        let pieces = segments(&shaped, Breaking::Hyphens);
        assert!(pieces.len() > 1, "{pieces:?}");
        let hyphenated = total(pieces);
        assert!(
            (word - hyphenated).abs() < 1e-3,
            "{word} against {hyphenated}"
        );
        // Letter by letter, the pairs would lose their kerning.
        let mut buffer = harfrust::Buffer::new();
        let run = &shaped.runs[0];
        let mut alone = 0.0;
        for at in 0..7 {
            alone += shape_run(&styled, &mut buffer, run, at..at + 1)?
                .iter()
                .map(|glyph| glyph.advance)
                .sum::<f32>();
        }
        assert!(alone > word + 1.0, "{alone} against {word}");
        Ok(())
    }

    #[test]
    fn an_empty_paragraph_keeps_its_span_s_size() -> anyhow::Result<()> {
        let styled = styled("a\n\nb");
        let shaped = ShapedText::new(&styled)?;
        let sizes: Vec<f32> = segments(&shaped, Breaking::Words)
            .iter()
            .map(|segment| segment.size)
            .collect();
        assert_eq!(sizes, [16.0; 3]);
        Ok(())
    }
}
