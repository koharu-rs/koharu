//! Unicode line breaking with language-aware segmentation and optional hyphenation.

use std::{ops::Range, sync::LazyLock};

use hypher::{Lang, hyphenate_bounded};
use icu_properties::{
    CodePointMapData,
    props::{LineBreak, Script as IcuScript},
};
use icu_segmenter::{
    LineSegmenter, LineSegmenterBorrowed,
    options::{LineBreakOptions, LineBreakWordOption},
};
use jieba_rs::Jieba;

static JIEBA: LazyLock<Jieba> = LazyLock::new(Jieba::new);

/// A line break candidate with its byte offset and whether it is mandatory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineBreakOpportunity {
    pub offset: usize,
    pub is_mandatory: bool,
}

/// Synthetic suffix to render only when a line actually breaks here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineBreakSuffix {
    Hyphen,
}

/// A trimmed line segment ready for shaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSegment {
    /// Range of visible text for this segment, excluding trailing mandatory break chars.
    pub range: Range<usize>,
    /// Byte offset where the next segment begins in the original string.
    pub next_offset: usize,
    /// Whether this segment ends with a mandatory break in the original text.
    pub is_mandatory: bool,
    /// Suffix to draw if this segment is the final segment on a wrapped line.
    pub break_suffix: Option<LineBreakSuffix>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HyphenationOptions {
    /// Pattern dictionary used to find valid break points.
    pub language: Lang,
    /// Words shorter than this remain intact.
    pub minimum_word_length: usize,
    /// Minimum characters retained before a discretionary break.
    pub minimum_prefix_length: usize,
    /// Minimum characters retained after a discretionary break.
    pub minimum_suffix_length: usize,
}

impl HyphenationOptions {
    #[must_use]
    pub fn new(language: Lang, minimum_word_length: usize) -> Self {
        let (minimum_prefix_length, minimum_suffix_length) = language.bounds();
        Self {
            language,
            minimum_word_length,
            minimum_prefix_length,
            minimum_suffix_length,
        }
    }

    #[must_use]
    pub fn with_fragment_bounds(
        mut self,
        minimum_prefix_length: usize,
        minimum_suffix_length: usize,
    ) -> Self {
        self.minimum_prefix_length = minimum_prefix_length;
        self.minimum_suffix_length = minimum_suffix_length;
        self
    }
}

/// Line breaker using ICU4X.
pub struct LineBreaker {
    segmenter: LineSegmenterBorrowed<'static>,
    korean_segmenter: LineSegmenterBorrowed<'static>,
    hyphenation: Option<HyphenationOptions>,
}

fn trim_mandatory_break_suffix(text: &str, start: usize, end: usize) -> usize {
    let mut trimmed_end = end;
    while trimmed_end > start {
        let Some(ch) = text[..trimmed_end].chars().next_back() else {
            break;
        };
        if !matches!(ch, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}') {
            break;
        }
        trimmed_end -= ch.len_utf8();
    }
    trimmed_end
}

impl LineBreaker {
    /// Creates a language-aware line breaker.
    #[must_use]
    pub fn new() -> Self {
        let korean_options = &mut LineBreakOptions::default();
        korean_options.word_option = Some(LineBreakWordOption::KeepAll);
        Self {
            segmenter: LineSegmenter::new_auto(LineBreakOptions::default()),
            korean_segmenter: LineSegmenter::new_auto(*korean_options),
            hyphenation: None,
        }
    }

    /// Enable pattern-based discretionary word hyphenation.
    #[must_use]
    pub fn with_hyphenation(mut self, options: HyphenationOptions) -> Self {
        self.hyphenation = Some(options);
        self
    }

    /// Returns a vector of line break opportunities in the given text.
    pub fn line_break_opportunities(&self, text: &str) -> Vec<LineBreakOpportunity> {
        let segmenter = if contains_korean(text) {
            &self.korean_segmenter
        } else {
            &self.segmenter
        };
        let opportunities = segmenter
            .segment_str(text)
            .map(|break_pos| LineBreakOpportunity {
                offset: break_pos,
                is_mandatory: text[..break_pos].chars().next_back().is_some_and(|c| {
                    matches!(
                        CodePointMapData::<LineBreak>::new().get(c),
                        LineBreak::MandatoryBreak
                            | LineBreak::CarriageReturn
                            | LineBreak::LineFeed
                            | LineBreak::NextLine
                    )
                }),
            })
            .collect();

        if contains_chinese(text) {
            apply_protected_ranges(opportunities, &chinese_word_ranges(text))
        } else {
            opportunities
        }
    }

    /// Returns shaped-text segments where mandatory break characters are excluded
    /// from the segment range but preserved in `next_offset`.
    pub fn line_segments(&self, text: &str) -> Vec<LineSegment> {
        self.line_break_opportunities(text)
            .windows(2)
            .flat_map(|window| {
                let start = window[0].offset;
                let end = window[1].offset;
                let is_mandatory = window[1].is_mandatory;
                let segment_end = if is_mandatory {
                    trim_mandatory_break_suffix(text, start, end)
                } else {
                    end
                };
                let segment = LineSegment {
                    range: start..segment_end,
                    next_offset: end,
                    is_mandatory,
                    break_suffix: None,
                };
                self.hyphenated_segments(text, segment)
            })
            .collect()
    }

    fn hyphenated_segments(&self, text: &str, segment: LineSegment) -> Vec<LineSegment> {
        let Some(config) = self.hyphenation else {
            return vec![segment];
        };
        if segment.is_mandatory || segment.range.is_empty() {
            return vec![segment];
        }

        let segment_text = &text[segment.range.clone()];
        let Some((core_start, core_end)) = hyphenatable_word_bounds(segment_text) else {
            return vec![segment];
        };

        let core = &segment_text[core_start..core_end];
        if core.chars().count() < config.minimum_word_length {
            return vec![segment];
        }

        let syllables: Vec<&str> = hyphenate_bounded(
            core,
            config.language,
            config.minimum_prefix_length,
            config.minimum_suffix_length,
        )
        .collect();
        if syllables.len() <= 1 {
            return vec![segment];
        }

        let mut result = Vec::with_capacity(syllables.len());
        let mut word_offset = 0usize;
        for (idx, syllable) in syllables.iter().enumerate() {
            let is_last = idx + 1 == syllables.len();
            let start = if idx == 0 {
                segment.range.start
            } else {
                segment.range.start + core_start + word_offset
            };
            word_offset += syllable.len();
            let end = if is_last {
                segment.range.end
            } else {
                segment.range.start + core_start + word_offset
            };
            result.push(LineSegment {
                range: start..end,
                next_offset: if is_last { segment.next_offset } else { end },
                is_mandatory: false,
                break_suffix: (!is_last).then_some(LineBreakSuffix::Hyphen),
            });
        }

        result
    }
}

impl Default for LineBreaker {
    fn default() -> Self {
        Self::new()
    }
}

pub fn hyphenation_lang_from_tag(value: &str) -> Option<Lang> {
    let lower = value.trim().to_ascii_lowercase();
    let primary = lower
        .split(['-', '_'])
        .next()
        .filter(|part| part.len() == 2)?;
    Lang::from_iso(primary.as_bytes().try_into().ok()?)
}

fn apply_protected_ranges(
    opportunities: Vec<LineBreakOpportunity>,
    protected_ranges: &[Range<usize>],
) -> Vec<LineBreakOpportunity> {
    if protected_ranges.is_empty() {
        return opportunities;
    }

    opportunities
        .into_iter()
        .filter(|opportunity| {
            opportunity.is_mandatory
                || !protected_ranges
                    .iter()
                    .any(|range| opportunity.offset > range.start && opportunity.offset < range.end)
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn break_on_whitespace() {
        let text = "The quick brown fox jumps over the lazy dog.";
        let linebreaker = LineBreaker::new();
        let breaks = linebreaker.line_break_opportunities(text);
        let segments: Vec<&str> = breaks
            .windows(2)
            .map(|w| &text[w[0].offset..w[1].offset])
            .collect();
        let expected = vec![
            "The ", "quick ", "brown ", "fox ", "jumps ", "over ", "the ", "lazy ", "dog.",
        ];
        assert_eq!(segments, expected);
    }

    #[test]
    fn break_on_newline() {
        let text = "Hello, \nWorld!";
        let linebreaker = LineBreaker::new();
        let breaks = linebreaker.line_break_opportunities(text);
        let expected = vec![
            LineBreakOpportunity {
                offset: 0,
                is_mandatory: false,
            },
            LineBreakOpportunity {
                offset: 8,
                is_mandatory: true,
            },
            LineBreakOpportunity {
                offset: 14,
                is_mandatory: false,
            },
        ];
        assert_eq!(breaks, expected);
    }

    #[test]
    fn line_segments_trim_newline_suffixes() {
        let text = "Hello, \nWorld!";
        let linebreaker = LineBreaker::new();
        let segments = linebreaker.line_segments(text);

        assert_eq!(segments.len(), 2);
        assert_eq!(&text[segments[0].range.clone()], "Hello, ");
        assert_eq!(segments[0].next_offset, 8);
        assert!(segments[0].is_mandatory);
        assert_eq!(segments[0].break_suffix, None);
        assert_eq!(&text[segments[1].range.clone()], "World!");
        assert_eq!(segments[1].next_offset, text.len());
        assert!(!segments[1].is_mandatory);
        assert_eq!(segments[1].break_suffix, None);
    }

    #[test]
    fn chinese_word_segmentation_keeps_jieba_words_together() {
        let text = "\u{5357}\u{4eac}\u{5e02}\u{957f}\u{6c5f}\u{5927}\u{6865}";
        let linebreaker = LineBreaker::new();
        let segments: Vec<&str> = linebreaker
            .line_segments(text)
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect();

        assert_eq!(
            segments,
            vec![
                "\u{5357}\u{4eac}\u{5e02}",
                "\u{957f}\u{6c5f}\u{5927}\u{6865}",
            ]
        );
    }

    #[test]
    fn chinese_word_segmentation_preserves_icu_punctuation_rules() {
        let text = "\u{5c0f}\u{8bf4}\u{ff0c}\u{4f60}\u{597d}";
        let linebreaker = LineBreaker::new();
        let segments: Vec<&str> = linebreaker
            .line_segments(text)
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect();

        assert_eq!(
            segments,
            vec!["\u{5c0f}\u{8bf4}\u{ff0c}", "\u{4f60}\u{597d}",]
        );
    }

    #[test]
    fn chinese_word_segmentation_does_not_resegment_kana_text() {
        let text = "\u{543e}\u{8f29}\u{306f}\u{732b}";
        let linebreaker = LineBreaker::new();
        let segments: Vec<&str> = linebreaker
            .line_segments(text)
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect();

        assert_eq!(
            segments,
            vec!["\u{543e}", "\u{8f29}", "\u{306f}", "\u{732b}",]
        );
    }

    #[test]
    fn korean_word_segmentation_uses_keep_all() {
        let text = "B6층 모험가들이 돌아갔으니까 청소 부탁해";
        let segments = LineBreaker::new()
            .line_segments(text)
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect::<Vec<_>>();

        assert_eq!(
            segments,
            vec!["B6층 ", "모험가들이 ", "돌아갔으니까 ", "청소 ", "부탁해"]
        );
    }

    #[test]
    fn hyphenation_adds_discretionary_segments_to_long_latin_words() {
        let text = "antidisestablishmentarianism";
        let linebreaker =
            LineBreaker::new().with_hyphenation(HyphenationOptions::new(Lang::English, 8));
        let segments = linebreaker.line_segments(text);

        assert!(
            segments.len() > 1,
            "expected long word to be split into hyphenation segments, got {segments:?}"
        );
        for segment in segments.iter().take(segments.len() - 1) {
            assert_eq!(segment.break_suffix, Some(LineBreakSuffix::Hyphen));
            assert!(!segment.is_mandatory);
        }
        assert_eq!(segments.last().unwrap().break_suffix, None);

        let rebuilt = segments
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect::<String>();
        assert_eq!(rebuilt, text);
    }

    #[test]
    fn hyphenation_does_not_invent_breaks_missing_from_the_patterns() {
        let text = "idols";
        let segments = LineBreaker::new()
            .with_hyphenation(HyphenationOptions::new(Lang::English, 5))
            .line_segments(text);

        assert_eq!(segments.len(), 1);
        assert_eq!(&text[segments[0].range.clone()], text);
        assert_eq!(segments[0].break_suffix, None);
    }

    #[test]
    fn configured_fragment_bounds_relax_language_defaults() {
        let text = "tower within";
        let segments = LineBreaker::new()
            .with_hyphenation(HyphenationOptions::new(Lang::English, 5).with_fragment_bounds(2, 2))
            .line_segments(text);
        let pieces = segments
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect::<Vec<_>>();

        assert_eq!(pieces, ["tow", "er ", "with", "in"]);
        assert_eq!(segments[0].break_suffix, Some(LineBreakSuffix::Hyphen));
        assert_eq!(segments[2].break_suffix, Some(LineBreakSuffix::Hyphen));
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

        for (tag, lang) in cases {
            assert_eq!(hyphenation_lang_from_tag(tag), Some(lang), "tag={tag}");
        }

        assert_eq!(hyphenation_lang_from_tag("German"), None);
        assert_eq!(hyphenation_lang_from_tag("ja-JP"), None);
    }

    #[test]
    fn hyphenation_supports_unicode_words() {
        let text = "электрификация";
        let linebreaker =
            LineBreaker::new().with_hyphenation(HyphenationOptions::new(Lang::Russian, 8));
        let segments = linebreaker.line_segments(text);

        assert!(
            segments.len() > 1,
            "expected unicode word to be split into hyphenation segments, got {segments:?}"
        );
        let rebuilt = segments
            .iter()
            .map(|segment| &text[segment.range.clone()])
            .collect::<String>();
        assert_eq!(rebuilt, text);
    }

    #[test]
    fn japanese_break_on_characters() {
        let text = "吾輩は猫である。名前はまだない。";
        let linebreaker = LineBreaker::new();
        let breaks = linebreaker.line_break_opportunities(text);
        let segments: Vec<&str> = breaks
            .windows(2)
            .map(|w| &text[w[0].offset..w[1].offset])
            .collect();
        let expected = vec![
            "吾", "輩", "は", "猫", "で", "あ", "る。", "名", "前", "は", "ま", "だ", "な", "い。",
        ];
        assert_eq!(segments, expected);
    }

    #[test]
    fn mixed_language_breaks_01() {
        let text = "『シャイニング』（The Shining）は、スタンリー・キューブリックが製作・監督し、小説家のダイアン・ジョンソンと共同脚本を務めた、1980年公開のサイコロジカルホラー映画。";
        let linebreaker = LineBreaker::new();
        let breaks = linebreaker.line_break_opportunities(text);
        let segments: Vec<&str> = breaks
            .windows(2)
            .map(|w| &text[w[0].offset..w[1].offset])
            .collect();
        #[rustfmt::skip]
        let expected = vec![
            "『シャ", "イ", "ニ", "ン", "グ』", "（The ", "Shining）", "は、", "ス", "タ", "ン", "リー・", "キュー", "ブ", "リッ", "ク", "が", "製", "作・", "監", "督", "し、", "小", "説", "家", "の", "ダ", "イ", "ア", "ン・", "ジョ", "ン", "ソ", "ン", "と", "共", "同", "脚", "本", "を", "務", "め", "た、", "1980", "年", "公", "開", "の", "サ", "イ", "コ", "ロ", "ジ", "カ", "ル", "ホ", "ラー", "映", "画。"
        ];
        assert_eq!(segments, expected);
    }

    #[test]
    fn mixed_chinese_language_breaks_use_jieba() {
        let text = "《我是猫》是日本作家夏目漱石创作的长篇小说，也是其代表作，它确立了夏目漱石在文学史上的地位。作品淋漓尽致地反映了二十世纪初，日本中小资产阶级的思想和生活，尖锐地揭露和批判了明治“文明开化”的资本主义社会。小说采用幽默、讽刺、滑稽的手法，借助一只猫的视觉、听觉、感觉，嘲笑了明治时代知识分子空虚的精神生活，小说构思奇巧，描写夸张，结构灵活，具有鲜明的艺术特色。";
        let linebreaker = LineBreaker::new();
        let breaks = linebreaker.line_break_opportunities(text);
        let segments: Vec<&str> = breaks
            .windows(2)
            .map(|w| &text[w[0].offset..w[1].offset])
            .collect();
        #[rustfmt::skip]
        let expected = vec![
            "《我", "是", "猫》", "是", "日本", "作家", "夏目漱石", "创作", "的", "长篇小说，", "也", "是", "其", "代表作，", "它", "确立", "了", "夏目漱石", "在", "文学史", "上", "的", "地位。", "作品", "淋漓尽致", "地", "反映", "了", "二十世纪", "初，", "日本", "中小", "资产阶级", "的", "思想", "和", "生活，", "尖锐", "地", "揭露", "和", "批判", "了", "明治“文明", "开化”的", "资本主义", "社会。", "小说", "采用", "幽默、", "讽刺、", "滑稽", "的", "手法，", "借助", "一只", "猫", "的", "视觉、", "听觉、", "感觉，", "嘲笑", "了", "明治", "时代", "知识分子", "空虚", "的", "精神", "生活，", "小说", "构思", "奇巧，", "描写", "夸张，", "结构", "灵活，", "具有", "鲜明", "的", "艺术", "特色。"
        ];
        assert_eq!(segments, expected);
    }
}
