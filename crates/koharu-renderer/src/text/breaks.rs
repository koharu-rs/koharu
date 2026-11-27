//! Where lines may break: Unicode line breaking, with Chinese words kept
//! whole, Vietnamese ones together where they can be, Korean broken between
//! words, words at hyphenation points, and a drawn-out sound in its run of
//! one letter; and the text between two breaks measured from its glyphs.

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

use super::{emphasis, itemize::Orientation, ruby::Ruby, shape::ShapedText};

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
/// hyphen, but in a drawn-out sound, which breaks with one in the middle of
/// its run of one letter in any language and as a word does, since any length
/// of the run reads alike and dictionaries have no points in it.
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
    /// It ends inside a word, between two of its syllables, where a line had
    /// better not break: Vietnamese spaces every syllable.
    pub inside_word: bool,
    /// Its largest span's size.
    pub size: f32,
    /// How far its ruby and emphasis marks reach above its line's centre,
    /// or right of a column's; 0 without any.
    pub rise: f32,
    /// How far its emphasis marks reach below its line's centre; 0 without
    /// any.
    pub sink: f32,
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
    // Unicode 17's rules, whose LB19a lets a line break before an opening
    // quotation mark and after a closing one between East Asian characters,
    // as Chinese sets “” as brackets; `new_auto`'s Unicode 15.1 data breaks on
    // neither side of one, so 位于“霍尔”中心地带 had run on as one piece down
    // and past its column. The LSTM data breaks Thai, Khmer, and Myanmar, as
    // `new_auto` does.
    let mut segmenter = LineSegmenter::new_17_for_non_complex_scripts(options);
    segmenter.load_lstm();
    // A middle dot never starts a line, as CLREQ and JLREQ keep it, but
    // Unicode leaves U+00B7 ambiguous, which broke a translated name before
    // the dot between its parts.
    let offsets: Vec<usize> = segmenter
        .segment_str(text)
        .filter(|&at| mandatory(at) || !words.iter().any(|w| at > w.start && at < w.end))
        .filter(|&at| at == 0 || mandatory(at) || !text[at..].starts_with('\u{B7}'))
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
        let points = match mandatory(next) {
            true => Vec::new(),
            false => points(text, start..end, breaking, lang),
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

/// Where `breaking` lets a hyphen end a line inside the piece `range`, at
/// `lang`'s hyphenation points, or in a drawn-out sound's run of one letter.
fn points(text: &str, range: Range<usize>, breaking: Breaking, lang: Option<Lang>) -> Vec<usize> {
    // A piece's word is its letters, between whatever else starts or ends
    // it. Photoshop hyphenates words of more than five letters.
    let piece = &text[range.clone()];
    let not_letter = |ch: char| !ch.is_alphabetic();
    let start = range.start + piece.len() - piece.trim_start_matches(not_letter).len();
    let word = start..start + piece.trim_matches(not_letter).len();
    let letters = &text[word.clone()];
    if !letters.chars().all(char::is_alphabetic) {
        return Vec::new();
    }
    if let Some(point) = drawn_out(letters) {
        return vec![word.start + point];
    }
    let Some(lang) = lang.filter(|_| breaking != Breaking::Words) else {
        return Vec::new();
    };
    if letters.chars().count() < 6 {
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

/// Where the drawn-out sound `letters`, such as "Gwaaaaah", breaks: in the
/// middle of its longest run of one letter four or more long, keeping two of
/// it on each side.
fn drawn_out(letters: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = letters.char_indices().collect();
    let same = |a: char, b: char| a.to_lowercase().eq(b.to_lowercase());
    let mut longest: Option<(usize, usize)> = None;
    let mut at = 0;
    while let Some(&(_, first)) = chars.get(at) {
        let length = (chars[at..].iter())
            .take_while(|(_, ch)| same(*ch, first))
            .count();
        if length >= 4 && longest.is_none_or(|(_, most)| length > most) {
            longest = Some((at, length));
        }
        at += length;
    }
    longest.map(|(start, length)| chars[start + length / 2].0)
}

/// The text's segments, broken as `breaking` allows and measured from the
/// glyphs it shaped into.
pub(super) fn segments(shaped: &ShapedText, breaking: Breaking) -> Vec<Segment> {
    let styled = shaped.styled;
    // Where the text may break, where a run starts or ends or a cluster
    // starts but not inside an upright group or under a ruby; and the advance
    // of the text before each byte, each glyph's at its cluster and an upright
    // group's cell at its start.
    let mut breakable = vec![false; styled.text.len() + 1];
    let mut advances = vec![0.0; styled.text.len() + 1];
    for run in &shaped.runs {
        breakable[run.range.start] = true;
        breakable[run.range.end] = true;
        if run.orientation == Orientation::TateChuYoko {
            advances[run.range.start] += f64::from(shaped.cell(run));
            continue;
        }
        for glyph in shaped.glyphs(run) {
            breakable[glyph.cluster] = true;
            advances[glyph.cluster] += f64::from(glyph.advance);
        }
    }
    for ruby in &styled.ruby {
        breakable[ruby.base.start + 1..ruby.base.end].fill(false);
    }
    let mut sum = 0.0;
    for value in &mut advances {
        (*value, sum) = (sum, sum + *value);
    }
    // Vietnamese words, whose syllables a line had better not break between.
    let words = match styled.vietnamese {
        true => pyvi::tokenize(&styled.text),
        false => Vec::new(),
    };
    // A cut inside a cluster, such as a ligature, inside an upright group, or
    // under a ruby joins the next piece.
    let mut segments = Vec::new();
    let mut joined = None;
    for (range, next, after) in opportunities(&styled.text, breaking, styled.hyphenation) {
        let start = joined.take().unwrap_or(range.start);
        if matches!(after, Break::Mandatory | Break::End) || breakable[next] {
            let inside_word = after == Break::Normal
                && (words.iter()).any(|word| next > word.start && next < word.end);
            let segment = measure(shaped, &advances, start..range.end, after, inside_word);
            segments.push(segment);
        } else {
            joined = Some(start);
        }
    }
    segments
}

/// The segment `range` makes, measured by `advances`.
fn measure(
    shaped: &ShapedText,
    advances: &[f64],
    range: Range<usize>,
    after: Break,
    inside_word: bool,
) -> Segment {
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
    let mut rise = styled
        .ruby
        .iter()
        .filter(|ruby| ruby.base.start < range.end && range.start < ruby.base.end)
        .map(Ruby::rise)
        .fold(0.0, f32::max);
    let mut sink = 0.0;
    for mark in &shaped.marks {
        let bases = mark
            .bases
            .iter()
            .filter(|base| base.start < range.end && range.start < base.end);
        for base in bases {
            let reach = mark.near(styled, base) + mark.size();
            match emphasis::under(styled) {
                true => sink = f32::max(sink, reach),
                false => rise = rise.max(reach),
            }
        }
    }
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
        inside_word,
        size,
        rise,
        sink,
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
                orientation: koharu_scene::Orientation::Mixed,
            }],
            vertical: false,
            align: Align::Start,
            line_height: 1.2,
            hyphenation: None,
            chinese: false,
            vietnamese: false,
            frame: Frame::Point,
            ruby: Vec::new(),
            marks: Vec::new(),
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
    fn chinese_breaks_before_an_opening_quote_and_after_a_closing_one() {
        // 位于“霍尔”中心地带, a translation on page 36 of a volume, whose
        // column ran past its box when it broke nowhere around the quotes.
        let text =
            "\u{4f4d}\u{4e8e}\u{201c}\u{970d}\u{5c14}\u{201d}\u{4e2d}\u{5fc3}\u{5730}\u{5e26}";
        assert_eq!(
            pieces(text, Breaking::Words),
            vec![
                "\u{4f4d}\u{4e8e}",
                "\u{201c}\u{970d}\u{5c14}\u{201d}",
                "\u{4e2d}\u{5fc3}\u{5730}\u{5e26}",
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
    fn a_drawn_out_sound_breaks_in_its_run_as_a_word_does_in_any_language() {
        let text = "Gwaaaaah! Hmmm";
        let unknown: Vec<&str> = opportunities(text, Breaking::Words, None)
            .into_iter()
            .map(|(range, ..)| &text[range])
            .collect();
        assert_eq!(unknown, ["Gwaa", "aaah! ", "Hmmm"]);
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
