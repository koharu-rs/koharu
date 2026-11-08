//! Where lines may break: Unicode line breaking, with Chinese words kept
//! whole, Korean broken between words, and words at hyphenation points; and
//! the text between two breaks measured from its glyphs.

use std::{
    ops::Range,
    sync::{LazyLock, Once},
};

use hypher::{Lang, hyphenate_bounded};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup, LineBreak, Script as IcuScript},
};
use icu_segmenter::{
    GraphemeClusterSegmenter, LineSegmenter,
    options::{LineBreakOptions, LineBreakWordOption},
};
use jieba_rs::Jieba;
use koharu_scene::LanguageIdentifier;

use super::{itemize::Orientation, shape::ShapedText};

static JIEBA: LazyLock<Jieba> = LazyLock::new(Jieba::new);

/// Starts loading jieba's dictionary, which takes about 50 ms, so the first
/// Chinese text needn't wait for it: once, on a thread of its own rather than
/// rayon's global pool, which the app may not have set up yet. Should the
/// thread fail to start, the first Chinese text loads it.
pub(crate) fn preload_dictionary() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("jieba".to_owned())
            .spawn(|| {
                LazyLock::force(&JIEBA);
            });
    });
}

/// Photoshop's hyphenation default keeps two letters on each side.
const HYPHENATION_KEEPS: usize = 2;
/// What a mandatory break ends with, which belongs to no line.
pub(super) const LINE_ENDS: [char; 5] = ['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'];

/// Where lines may break: between words, then also at hyphenation points,
/// then anywhere in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Breaking {
    Words,
    Hyphens,
    Anywhere,
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
    /// Inside a word with no hyphenation point.
    Anywhere,
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

/// The text cut at its break opportunities. With `hyphenate`, a word also
/// breaks at its hyphenation points.
fn opportunities(text: &str, hyphenate: Option<Lang>) -> Vec<Cut> {
    let mut options = LineBreakOptions::default();
    // Korean breaks between words, not syllables.
    if contains_korean(text) {
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
    let words = match contains_chinese(text) {
        true => chinese_word_ranges(text),
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
        match hyphenate {
            Some(lang) if !mandatory(next) => {
                hyphenated(text, start..end, next, after, lang, &mut pieces);
            }
            _ => pieces.push((start..end, next, after)),
        }
    }
    pieces
}

/// Pushes a piece split at its word's hyphenation points, each part but the
/// last ending in a hyphen break.
fn hyphenated(
    text: &str,
    range: Range<usize>,
    next: usize,
    after: Break,
    lang: Lang,
    pieces: &mut Vec<Cut>,
) {
    // Photoshop hyphenates words of more than five letters.
    let word = hyphenatable_word_bounds(&text[range.clone()])
        .map(|(start, end)| range.start + start..range.start + end)
        .filter(|word| text[word.clone()].chars().count() >= 6);
    let Some(word) = word else {
        pieces.push((range, next, after));
        return;
    };
    let syllables = hyphenate_bounded(
        &text[word.clone()],
        lang,
        HYPHENATION_KEEPS,
        HYPHENATION_KEEPS,
    );
    let mut start = range.start;
    let mut end = word.start;
    for syllable in syllables {
        end += syllable.len();
        if end < word.end {
            pieces.push((start..end, end, Break::Hyphen));
            start = end;
        }
    }
    pieces.push((start..range.end, next, after));
}

/// The patterns that hyphenate `language`, by its two-letter code.
pub(super) fn hyphenation(language: &LanguageIdentifier) -> Option<Lang> {
    Lang::from_iso(language.language.as_str().as_bytes().try_into().ok()?)
}

fn chinese_word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut offset = 0usize;
    JIEBA
        .cut(text, true)
        .into_iter()
        .filter_map(|token| {
            let word = token.word;
            let start = offset;
            let end = start + word.len();
            offset = end;

            contains_chinese(word).then_some(start..end)
        })
        .collect()
}

fn contains_korean(text: &str) -> bool {
    let script_map = CodePointMapData::<IcuScript>::new();
    text.chars()
        .any(|ch| script_map.get(ch) == IcuScript::Hangul)
}

fn contains_chinese(text: &str) -> bool {
    let script_map = CodePointMapData::<IcuScript>::new();
    text.chars()
        .map(|ch| script_map.get(ch))
        .try_fold(false, |has_chinese, script| match script {
            IcuScript::Han | IcuScript::Bopomofo => Some(true),
            IcuScript::Hiragana | IcuScript::Katakana | IcuScript::Hangul => None,
            _ => Some(has_chinese),
        })
        .unwrap_or(false)
}

fn hyphenatable_word_bounds(text: &str) -> Option<(usize, usize)> {
    let start = text.find(|ch: char| ch.is_alphabetic())?;
    let end = text
        .char_indices()
        .rev()
        .find(|&(_, ch)| ch.is_alphabetic())
        .map(|(idx, ch)| idx + ch.len_utf8())?;
    if start >= end {
        return None;
    }

    let core = &text[start..end];
    core.chars()
        .all(|ch| ch.is_alphabetic())
        .then_some((start, end))
}

/// The text's segments, broken as `breaking` allows and measured from the
/// glyphs it shaped into.
pub(super) fn segments(shaped: &ShapedText, breaking: Breaking) -> Vec<Segment> {
    let styled = shaped.styled;
    let hyphenate = match breaking {
        Breaking::Words => None,
        Breaking::Hyphens | Breaking::Anywhere => styled.hyphenation,
    };
    let mut cuts = opportunities(&styled.text, hyphenate);
    if breaking == Breaking::Anywhere {
        cuts = anywhere(&styled.text, cuts);
    }
    let advances = advances(shaped);
    joined(cuts, &breakable(shaped))
        .into_iter()
        .map(|(range, after)| measure(shaped, &advances, range, after))
        .collect()
}

/// `cuts` with each word also cut between its graphemes, unless it ends at a
/// hyphenation point.
fn anywhere(text: &str, cuts: Vec<Cut>) -> Vec<Cut> {
    let mut out = Vec::with_capacity(cuts.len());
    for (range, next, after) in cuts {
        if after == Break::Hyphen {
            out.push((range, next, after));
            continue;
        }
        let word = &text[range.clone()];
        let visible = word.trim_end().len();
        let mut start = range.start;
        for at in GraphemeClusterSegmenter::new()
            .segment_str(word)
            .filter(|at| *at > 0 && *at < visible)
        {
            let end = range.start + at;
            out.push((start..end, end, Break::Anywhere));
            start = end;
        }
        out.push((start..range.end, next, after));
    }
    out
}

/// `cuts` where a break may fall. A cut inside a cluster, such as a
/// ligature, or inside an upright group joins the next piece.
fn joined(cuts: Vec<Cut>, breakable: &[bool]) -> Vec<(Range<usize>, Break)> {
    let mut out = Vec::with_capacity(cuts.len());
    let mut joined = None;
    for (range, next, after) in cuts {
        let start = joined.take().unwrap_or(range.start);
        if matches!(after, Break::Mandatory | Break::End) || breakable[next] {
            out.push((start..range.end, after));
        } else {
            joined = Some(start);
        }
    }
    out
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

/// Where the text may break: where a run starts or ends, or a cluster
/// starts, but not inside an upright group.
fn breakable(shaped: &ShapedText) -> Vec<bool> {
    let mut at = vec![false; shaped.styled.text.len() + 1];
    for run in &shaped.runs {
        at[run.range.start] = true;
        at[run.range.end] = true;
        if run.orientation != Orientation::TateChuYoko {
            for glyph in shaped.glyphs(run) {
                at[glyph.cluster] = true;
            }
        }
    }
    at
}

/// The advance of the text before each byte: each glyph's at its cluster,
/// and an upright group's cell at its start.
fn advances(shaped: &ShapedText) -> Vec<f64> {
    let styled = shaped.styled;
    let mut at = vec![0.0; styled.text.len() + 1];
    for run in &shaped.runs {
        if run.orientation == Orientation::TateChuYoko {
            at[run.range.start] += f64::from(styled.spans[run.span].size);
            continue;
        }
        for glyph in shaped.glyphs(run) {
            at[glyph.cluster] += f64::from(glyph.advance);
        }
    }
    let mut sum = 0.0;
    for value in &mut at {
        (*value, sum) = (sum, sum + *value);
    }
    at
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fonts::FontSystem,
        text::testing::{primary, styled},
    };

    /// The text of each piece.
    fn pieces(text: &str, hyphenate: Option<Lang>) -> Vec<&str> {
        opportunities(text, hyphenate)
            .into_iter()
            .map(|(range, ..)| &text[range])
            .collect()
    }

    #[test]
    fn break_on_whitespace() {
        let text = "The quick brown fox jumps over the lazy dog.";
        let expected = vec![
            "The ", "quick ", "brown ", "fox ", "jumps ", "over ", "the ", "lazy ", "dog.",
        ];
        assert_eq!(pieces(text, None), expected);
    }

    #[test]
    fn a_line_break_ends_its_piece_and_belongs_to_no_line() {
        let text = "Hello, \nWorld!";
        assert_eq!(
            opportunities(text, None),
            [(0..7, 8, Break::Mandatory), (8..14, 14, Break::End)]
        );
    }

    #[test]
    fn chinese_word_segmentation_keeps_jieba_words_together() {
        let text = "\u{5357}\u{4eac}\u{5e02}\u{957f}\u{6c5f}\u{5927}\u{6865}";
        assert_eq!(
            pieces(text, None),
            vec![
                "\u{5357}\u{4eac}\u{5e02}",
                "\u{957f}\u{6c5f}\u{5927}\u{6865}",
            ]
        );
    }

    #[test]
    fn chinese_word_segmentation_preserves_icu_punctuation_rules() {
        let text = "\u{5c0f}\u{8bf4}\u{ff0c}\u{4f60}\u{597d}";
        assert_eq!(
            pieces(text, None),
            vec!["\u{5c0f}\u{8bf4}\u{ff0c}", "\u{4f60}\u{597d}",]
        );
    }

    #[test]
    fn chinese_word_segmentation_does_not_resegment_kana_text() {
        let text = "\u{543e}\u{8f29}\u{306f}\u{732b}";
        assert_eq!(
            pieces(text, None),
            vec!["\u{543e}", "\u{8f29}", "\u{306f}", "\u{732b}",]
        );
    }

    #[test]
    fn korean_word_segmentation_uses_keep_all() {
        let text = "B6층 모험가들이 돌아갔으니까 청소 부탁해";
        assert_eq!(
            pieces(text, None),
            vec!["B6층 ", "모험가들이 ", "돌아갔으니까 ", "청소 ", "부탁해"]
        );
    }

    #[test]
    fn hyphenation_splits_a_long_word_at_hyphen_breaks() {
        let text = "antidisestablishmentarianism";
        let pieces = opportunities(text, Some(Lang::English));
        assert!(pieces.len() > 1, "expected hyphen breaks, got {pieces:?}");
        let (last, parts) = pieces.split_last().expect("a piece");
        assert!(parts.iter().all(|(_, _, after)| *after == Break::Hyphen));
        assert_eq!(last.2, Break::End);
        let rebuilt: String = pieces
            .iter()
            .map(|(range, ..)| &text[range.clone()])
            .collect();
        assert_eq!(rebuilt, text);
    }

    #[test]
    fn hyphenation_does_not_invent_breaks_missing_from_the_patterns() {
        assert_eq!(pieces("strength", Some(Lang::English)), ["strength"]);
    }

    #[test]
    fn hyphenation_keeps_two_letters_each_side_and_skips_short_words() {
        // English's own bounds keep three letters at a word's end, which
        // would leave "within" whole; "tower" has fewer than six letters.
        assert_eq!(
            pieces("tower within", Some(Lang::English)),
            ["tower ", "with", "in"]
        );
    }

    #[test]
    fn hyphenation_language_tags_cover_hypher_languages() {
        let cases = [
            ("af", Lang::Afrikaans),
            ("sq", Lang::Albanian),
            ("as", Lang::Assamese),
            ("be", Lang::Belarusian),
            ("bn", Lang::Bengali),
            ("bg", Lang::Bulgarian),
            ("ca", Lang::Catalan),
            ("hr", Lang::Croatian),
            ("cs", Lang::Czech),
            ("da", Lang::Danish),
            ("nl", Lang::Dutch),
            ("en-US", Lang::English),
            ("et", Lang::Estonian),
            ("fi", Lang::Finnish),
            ("fr-FR", Lang::French),
            ("gl", Lang::Galician),
            ("ka", Lang::Georgian),
            ("de-DE", Lang::German),
            ("el", Lang::Greek),
            ("gu", Lang::Gujarati),
            ("hi", Lang::Hindi),
            ("hu", Lang::Hungarian),
            ("is", Lang::Icelandic),
            ("it-IT", Lang::Italian),
            ("kn", Lang::Kannada),
            ("ku", Lang::Kurmanji),
            ("la", Lang::Latin),
            ("lt", Lang::Lithuanian),
            ("ml", Lang::Malayalam),
            ("mr", Lang::Marathi),
            ("mn", Lang::Mongolian),
            ("no", Lang::Norwegian),
            ("nb", Lang::Norwegian),
            ("nn", Lang::Norwegian),
            ("or", Lang::Oriya),
            ("pa", Lang::Panjabi),
            ("pl", Lang::Polish),
            ("pt-BR", Lang::Portuguese),
            ("ru", Lang::Russian),
            ("sa", Lang::Sanskrit),
            ("sr", Lang::Serbian),
            ("sk", Lang::Slovak),
            ("sl", Lang::Slovenian),
            ("es-ES", Lang::Spanish),
            ("sv", Lang::Swedish),
            ("ta", Lang::Tamil),
            ("te", Lang::Telugu),
            ("tr", Lang::Turkish),
            ("tk", Lang::Turkmen),
            ("uk", Lang::Ukrainian),
        ];

        let of = |tag: &str| hyphenation(&tag.parse().expect("a language tag"));
        for (tag, lang) in cases {
            assert_eq!(of(tag), Some(lang), "tag={tag}");
        }

        assert_eq!(of("ja-JP"), None);
    }

    #[test]
    fn hyphenation_supports_unicode_words() {
        let text = "электрификация";
        let pieces = pieces(text, Some(Lang::Russian));
        assert!(pieces.len() > 1, "expected hyphen breaks, got {pieces:?}");
        assert_eq!(pieces.concat(), text);
    }

    #[test]
    fn japanese_break_on_characters() {
        let text = "吾輩は猫である。名前はまだない。";
        let expected = vec![
            "吾", "輩", "は", "猫", "で", "あ", "る。", "名", "前", "は", "ま", "だ", "な", "い。",
        ];
        assert_eq!(pieces(text, None), expected);
    }

    #[test]
    fn mixed_language_breaks_01() {
        let text = "『シャイニング』（The Shining）は、スタンリー・キューブリックが製作・監督し、小説家のダイアン・ジョンソンと共同脚本を務めた、1980年公開のサイコロジカルホラー映画。";
        #[rustfmt::skip]
        let expected = vec![
            "『シャ", "イ", "ニ", "ン", "グ』", "（The ", "Shining）", "は、", "ス", "タ", "ン", "リー・", "キュー", "ブ", "リッ", "ク", "が", "製", "作・", "監", "督", "し、", "小", "説", "家", "の", "ダ", "イ", "ア", "ン・", "ジョ", "ン", "ソ", "ン", "と", "共", "同", "脚", "本", "を", "務", "め", "た、", "1980", "年", "公", "開", "の", "サ", "イ", "コ", "ロ", "ジ", "カ", "ル", "ホ", "ラー", "映", "画。"
        ];
        assert_eq!(pieces(text, None), expected);
    }

    #[test]
    fn mixed_chinese_language_breaks_use_jieba() {
        let text = "《我是猫》是日本作家夏目漱石创作的长篇小说，也是其代表作，它确立了夏目漱石在文学史上的地位。作品淋漓尽致地反映了二十世纪初，日本中小资产阶级的思想和生活，尖锐地揭露和批判了明治“文明开化”的资本主义社会。小说采用幽默、讽刺、滑稽的手法，借助一只猫的视觉、听觉、感觉，嘲笑了明治时代知识分子空虚的精神生活，小说构思奇巧，描写夸张，结构灵活，具有鲜明的艺术特色。";
        #[rustfmt::skip]
        let expected = vec![
            "《我", "是", "猫》", "是", "日本", "作家", "夏目漱石", "创作", "的", "长篇小说，", "也", "是", "其", "代表作，", "它", "确立", "了", "夏目漱石", "在", "文学史", "上", "的", "地位。", "作品", "淋漓尽致", "地", "反映", "了", "二十世纪", "初，", "日本", "中小", "资产阶级", "的", "思想", "和", "生活，", "尖锐", "地", "揭露", "和", "批判", "了", "明治“文明", "开化”的", "资本主义", "社会。", "小说", "采用", "幽默、", "讽刺、", "滑稽", "的", "手法，", "借助", "一只", "猫", "的", "视觉、", "听觉、", "感觉，", "嘲笑", "了", "明治", "时代", "知识分子", "空虚", "的", "精神", "生活，", "小说", "构思", "奇巧，", "描写", "夸张，", "结构", "灵活，", "具有", "鲜明", "的", "艺术", "特色。"
        ];
        assert_eq!(pieces(text, None), expected);
    }

    #[test]
    fn breaking_anywhere_measures_the_word_as_shaped_once() -> anyhow::Result<()> {
        // A kerning pair, A and V, crosses every break.
        let styled = styled("AVAVAV", vec![primary(&mut FontSystem::new())]);
        let shaped = ShapedText::new(&styled)?;
        let total = |segments: Vec<Segment>| segments.iter().map(|s| s.advance).sum::<f32>();
        let word = total(segments(&shaped, Breaking::Words));
        let anywhere = total(segments(&shaped, Breaking::Anywhere));
        assert!((word - anywhere).abs() < 1e-3, "{word} against {anywhere}");
        // Letter by letter, the pairs would lose their kerning.
        let mut buffer = harfrust::Buffer::new();
        let run = &shaped.runs[0];
        let mut alone = 0.0;
        for at in 0..6 {
            alone += shaped
                .reshape(&mut buffer, run, at..at + 1)?
                .iter()
                .map(|glyph| glyph.advance)
                .sum::<f32>();
        }
        assert!(alone > word + 1.0, "{alone} against {word}");
        Ok(())
    }

    #[test]
    fn an_empty_paragraph_keeps_its_span_s_size() -> anyhow::Result<()> {
        let styled = styled("a\n\nb", vec![primary(&mut FontSystem::new())]);
        let shaped = ShapedText::new(&styled)?;
        let sizes: Vec<f32> = segments(&shaped, Breaking::Words)
            .iter()
            .map(|segment| segment.size)
            .collect();
        assert_eq!(sizes, [16.0; 3]);
        Ok(())
    }
}
