//! The text cut into runs, each in one span, font, bidi level and script:
//! what HarfRust shapes at once.

use std::ops::Range;

use icu_properties::{
    CodePointMapData,
    props::{Script as IcuScript, VerticalOrientation},
};
use icu_segmenter::GraphemeClusterSegmenter;
use unicode_bidi::Level;

use super::{breaks::LINE_ENDS, style::Styled};
use crate::{fonts::Font, script::harfrust_script};

/// Text HarfRust shapes at once: one span, font, bidi level and script.
#[derive(Clone, Debug)]
pub(super) struct Run {
    pub range: Range<usize>,
    pub span: usize,
    /// The font's place in its span's chain.
    pub font: usize,
    pub level: Level,
    pub script: Option<harfrust::Script>,
    pub orientation: Orientation,
    /// Its glyphs in `ShapedText::glyphs`, in visual order.
    pub glyphs: Range<usize>,
}

/// How a run stands in a vertical column. Horizontal text's runs are all
/// `Upright`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Orientation {
    /// One character below another, in their vertical forms.
    Upright,
    /// Turned a quarter clockwise, set as a horizontal line down the column.
    Sideways,
    /// Two or three `!` and `?` side by side in one upright cell, as
    /// Photoshop's tate-chu-yoko sets them.
    TateChuYoko,
}

/// The runs of `styled`, whose bytes have bidi `levels`, in logical order
/// and leaving out the characters that break lines; and, in code point
/// order, the characters no font of their span has. Each run is cut where a
/// line breaks; where the span or the bidi level changes; where vertical
/// text turns; where the script changes; and where the span's fonts fall
/// back. Fonts are chosen over a whole piece before it's cut, so a mark a
/// turn cuts from its words still takes their font.
pub(super) fn itemize(styled: &Styled, levels: &[Level]) -> (Vec<Run>, Vec<char>) {
    let mut runs = Vec::new();
    let mut uncovered = Vec::new();
    for (piece, span) in pieces(styled, levels) {
        let fonts = fall_back(styled, piece.clone(), span, &mut uncovered);
        let font_at = |at: usize| {
            let (_, font) = fonts.iter().find(|(cut, _)| cut.contains(&at))?;
            styled.spans[span].fonts.get(*font)
        };
        let parts = match styled.vertical {
            true => upright_groups(&styled.text, piece)
                .into_iter()
                .flat_map(|(part, group)| match group {
                    true => vec![(part, Orientation::TateChuYoko)],
                    false => orientations(&styled.text, part, font_at),
                })
                .collect(),
            false => vec![(piece, Orientation::Upright)],
        };
        for (part, orientation) in parts {
            // Vertical text stands upright top to bottom whatever its
            // direction; what it turns sideways keeps its own, as a line.
            let level = match (styled.vertical, orientation) {
                (false, _) | (true, Orientation::Sideways) => levels[part.start],
                (true, Orientation::Upright | Orientation::TateChuYoko) => Level::ltr(),
            };
            for (range, script) in script_runs(&styled.text, part) {
                runs.extend(fonts.iter().filter_map(|(cut, font)| {
                    let (start, end) = (cut.start.max(range.start), cut.end.min(range.end));
                    (start < end).then(|| Run {
                        range: start..end,
                        span,
                        font: *font,
                        level,
                        script: harfrust_script(script),
                        orientation,
                        glyphs: 0..0,
                    })
                }));
            }
        }
    }
    uncovered.sort_unstable();
    uncovered.dedup();
    (runs, uncovered)
}

/// Each span cut where a line breaks or the bidi level changes, leaving out
/// the line break characters; with the span's index.
fn pieces(styled: &Styled, levels: &[Level]) -> Vec<(Range<usize>, usize)> {
    let mut pieces = Vec::new();
    for (index, span) in styled.spans.iter().enumerate() {
        let mut start: Option<usize> = None;
        for (at, ch) in styled.text[span.range.clone()].char_indices() {
            let at = span.range.start + at;
            if LINE_ENDS.contains(&ch) {
                pieces.extend(start.take().map(|start| (start..at, index)));
                continue;
            }
            match start {
                Some(from) if levels[from] == levels[at] => {}
                Some(from) => {
                    pieces.push((from..at, index));
                    start = Some(at);
                }
                None => start = Some(at),
            }
        }
        pieces.extend(start.map(|start| (start..span.range.end, index)));
    }
    pieces
}

/// `piece` of span `span` cut where its font changes, with each part's font:
/// grapheme by grapheme, the first of the span's fonts that covers it. A
/// grapheme of no script of its own, such as a mark, a digit, or a space,
/// takes the font of the text beside it when that covers it: the one before
/// it, or at the piece's start the first after it that a font covers. So a
/// sentence's punctuation and numbers come from the font of its words: a CJK
/// sentence's from its CJK font, with its vertical forms, even where the
/// first font, a Latin one, has them too. A grapheme no font covers takes
/// the first, and its characters no font has go to `uncovered`.
fn fall_back(
    styled: &Styled,
    piece: Range<usize>,
    span: usize,
    uncovered: &mut Vec<char>,
) -> Vec<(Range<usize>, usize)> {
    let fonts = &styled.spans[span].fonts;
    if fonts.is_empty() {
        return Vec::new();
    }
    let text = &styled.text[piece.clone()];
    if fonts[0].covers(text) {
        return vec![(piece, 0)];
    }
    // Graphemes keep combining marks, variation selectors, and emoji ZWJ
    // sequences together instead of falling back per Unicode scalar.
    let boundaries: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(text).collect();
    let graphemes: Vec<&str> = boundaries
        .windows(2)
        .map(|pair| &text[pair[0]..pair[1]])
        .collect();
    let own: Vec<Option<usize>> = graphemes
        .iter()
        .map(|grapheme| fonts.iter().position(|font| font.covers(grapheme)))
        .collect();
    let scripts = CodePointMapData::<IcuScript>::new();
    let weak = |grapheme: &str| {
        grapheme
            .chars()
            .all(|ch| matches!(scripts.get(ch), IcuScript::Common | IcuScript::Inherited))
    };
    let mut parts: Vec<(Range<usize>, usize)> = Vec::new();
    let mut before: Option<usize> = None;
    for (index, (grapheme, pair)) in graphemes.iter().zip(boundaries.windows(2)).enumerate() {
        let beside = before.or_else(|| own[index + 1..].iter().flatten().next().copied());
        let font = match beside {
            Some(beside) if weak(grapheme) && fonts[beside].covers(grapheme) => beside,
            _ => own[index].unwrap_or_else(|| {
                uncovered.extend(grapheme.chars().filter(|character| {
                    let mut utf8 = [0; 4];
                    let character = character.encode_utf8(&mut utf8);
                    !fonts.iter().any(|font| font.covers(character))
                }));
                0
            }),
        };
        before = Some(font);
        let range = piece.start + pair[0]..piece.start + pair[1];
        match parts.last_mut() {
            Some((last, at)) if *at == font => last.end = range.end,
            _ => parts.push((range, font)),
        }
    }
    parts
}

/// `range` in runs of one script. Common and inherited characters, such as
/// spaces and marks, join the script around them.
fn script_runs(text: &str, range: Range<usize>) -> Vec<(Range<usize>, IcuScript)> {
    let scripts = CodePointMapData::<IcuScript>::new();
    let weak = |script: IcuScript| matches!(script, IcuScript::Common | IcuScript::Inherited);
    let mut runs = Vec::new();
    let mut chars = text[range.clone()].char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let mut script = scripts.get(ch);
        let mut end = start + ch.len_utf8();
        while let Some(&(next, next_ch)) = chars.peek() {
            let next_script = scripts.get(next_ch);
            if next_script == script || weak(next_script) {
                end = next + next_ch.len_utf8();
            } else if weak(script) {
                script = next_script;
                end = next + next_ch.len_utf8();
            } else {
                break;
            }
            chars.next();
        }
        runs.push((range.start + start..range.start + end, script));
    }
    runs
}

/// `range` of vertical text cut where it turns, grapheme by grapheme, each
/// set in the font `font_at` its offset. What Unicode's Vertical_Orientation
/// (UAX #50) rotates, such as Latin letters and digits, is set sideways, as
/// Photoshop and CSS's `text-orientation: mixed` set it; the rest stands
/// upright, `vert` giving the characters it transforms their vertical forms.
/// What it transforms or else rotates (`Tr`), such as `〜` or a bracket,
/// turns sideways in a font without a vertical form for it. `!` and `?`
/// stand upright in full width, as manga sets them, and so does an emoji: a
/// grapheme with the emoji variation selector or a keycap, which UAX #50
/// keeps upright.
fn orientations<'a>(
    text: &str,
    range: Range<usize>,
    font_at: impl Fn(usize) -> Option<&'a Font>,
) -> Vec<(Range<usize>, Orientation)> {
    let orientation = CodePointMapData::<VerticalOrientation>::new();
    let part = &text[range.clone()];
    let boundaries: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(part).collect();
    let mut parts: Vec<(Range<usize>, Orientation)> = Vec::new();
    for pair in boundaries.windows(2) {
        let grapheme = &part[pair[0]..pair[1]];
        let first = grapheme.chars().next().expect("a grapheme has a character");
        let rotated = match orientation.get(first) {
            VerticalOrientation::Rotated => true,
            VerticalOrientation::TransformedRotated => {
                font_at(range.start + pair[0]).is_some_and(|font| !font.has_vertical_form(first))
            }
            _ => false,
        };
        let turned =
            match rotated && !is_exclamation(first) && !grapheme.contains(['\u{fe0f}', '\u{20e3}'])
            {
                true => Orientation::Sideways,
                false => Orientation::Upright,
            };
        let end = range.start + pair[1];
        match parts.last_mut() {
            Some((last, at)) if *at == turned => last.end = end,
            _ => parts.push((range.start + pair[0]..end, turned)),
        }
    }
    parts
}

fn is_exclamation(ch: char) -> bool {
    matches!(ch, '!' | '?' | '！' | '？')
}

/// `range` cut around what vertical text sets upright side by side in one
/// cell: two or three `!` and `?`, and a number of one or two digits, as
/// CSS's `text-combine-upright: digits 2` sets it, unless it's part of a
/// Latin word; each part says whether it is such a group. A longer number
/// turns sideways as a word does.
fn upright_groups(text: &str, range: Range<usize>) -> Vec<(Range<usize>, bool)> {
    let mut parts = Vec::new();
    let mut start = range.start;
    let mut chars = text[range.clone()].char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        let digits = ch.is_ascii_digit();
        if !digits && !is_exclamation(ch) {
            continue;
        }
        let group_start = range.start + offset;
        let mut group_end = group_start + ch.len_utf8();
        let mut count = 1;
        while let Some(&(next, next_ch)) = chars.peek() {
            let joins = match digits {
                true => next_ch.is_ascii_digit(),
                false => is_exclamation(next_ch) && count < 3,
            };
            if !joins {
                break;
            }
            group_end = range.start + next + next_ch.len_utf8();
            count += 1;
            chars.next();
        }
        let in_word = |ch: Option<char>| ch.is_some_and(|ch| ch.is_ascii_alphabetic());
        let grouped = match digits {
            true => {
                count <= 2
                    && !in_word(text[..group_start].chars().next_back())
                    && !in_word(text[group_end..].chars().next())
            }
            false => count >= 2,
        };
        if !grouped {
            continue;
        }
        if start < group_start {
            parts.push((start..group_start, false));
        }
        parts.push((group_start..group_end, true));
        start = group_end;
    }
    if start < range.end {
        parts.push((start..range.end, false));
    }
    parts
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Align, Color, Frame};
    use unicode_bidi::BidiInfo;

    use super::*;
    use crate::{
        fonts::{Font, Library},
        text::style::Span,
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

    /// The text of each piece, with its span.
    fn cut(styled: &Styled) -> Vec<(&str, usize)> {
        let levels = BidiInfo::new(&styled.text, None).levels;
        pieces(styled, &levels)
            .into_iter()
            .map(|(range, span)| (&styled.text[range], span))
            .collect()
    }

    #[test]
    fn pieces_end_at_line_breaks_spans_and_direction_changes() {
        // Two spans, "ab\nc" and "d " then Hebrew.
        let text = "ab\ncd \u{05e9}\u{05dc}";
        let span = |range| Span {
            range,
            fonts: Vec::new(),
            size: 16.0,
            fill: Color::BLACK,
            stroke: None,
        };
        let styled = Styled {
            spans: vec![span(0..4), span(4..text.len())],
            ..styled(text)
        };
        assert_eq!(
            cut(&styled),
            [("ab", 0), ("c", 0), ("d ", 1), ("\u{05e9}\u{05dc}", 1)]
        );
    }

    #[test]
    fn vertical_text_turns_latin_and_digits_sideways() {
        let text = "東京Tokyo Tower、高さ333!";
        let parts: Vec<(&str, Orientation)> = orientations(text, 0..text.len(), |_| None)
            .into_iter()
            .map(|(range, orientation)| (&text[range], orientation))
            .collect();
        assert_eq!(
            parts,
            [
                ("東京", Orientation::Upright),
                ("Tokyo Tower", Orientation::Sideways),
                ("、高さ", Orientation::Upright),
                ("333", Orientation::Sideways),
                ("!", Orientation::Upright),
            ]
        );
        // An emoji keycap starts with a digit but shows as an emoji.
        let keycap = "1\u{fe0f}\u{20e3}";
        assert_eq!(
            orientations(keycap, 0..keycap.len(), |_| None),
            [(0..keycap.len(), Orientation::Upright)]
        );
    }

    #[test]
    fn a_sideways_right_to_left_word_keeps_its_direction() {
        let styled = Styled {
            vertical: true,
            ..styled("東\u{05e9}\u{05dc}\u{05d5}\u{05dd}")
        };
        let levels = BidiInfo::new(&styled.text, None).levels;
        let (runs, _) = itemize(&styled, &levels);
        let hebrew = runs.last().expect("a run");
        assert_eq!(hebrew.orientation, Orientation::Sideways);
        assert!(hebrew.level.is_rtl());
        assert!(!runs[0].level.is_rtl());
    }

    #[test]
    fn vertical_text_sets_two_or_three_exclamations_upright() {
        let text = "あ!?い!!!!う!";
        let parts: Vec<(&str, bool)> = upright_groups(text, 0..text.len())
            .into_iter()
            .map(|(range, upright)| (&text[range], upright))
            .collect();
        assert_eq!(
            parts,
            [
                ("あ", false),
                ("!?", true),
                ("い", false),
                ("!!!", true),
                ("!う!", false)
            ]
        );
    }

    #[test]
    fn a_mark_takes_the_font_of_the_text_beside_it() {
        let latin = styled("").spans[0].fonts[0].clone();
        let cjk = [
            "Microsoft YaHei",
            "Yu Gothic",
            "MS Gothic",
            "Noto Sans CJK SC",
            "PingFang SC",
        ]
        .into_iter()
        .find_map(|family| Library::new().family(family).ok())
        .expect("an installed CJK font");
        // The Latin font has every mark and digit here, and no Han.
        let text = "咕呜…以“人”为15年 Hi, OK…";
        let mut styled = Styled {
            vertical: true,
            ..styled(text)
        };
        styled.spans[0].fonts = vec![latin, cjk];
        let levels = BidiInfo::new(&styled.text, None).levels;
        let (runs, _) = itemize(&styled, &levels);
        let font = |part: &str| {
            let at = text.find(part).expect("in the text");
            runs.iter()
                .find(|run| run.range.contains(&at))
                .expect("a run")
                .font
        };
        // A CJK sentence's marks and number come from its CJK font, even
        // where a turn cuts them from it; a Latin word's from its own.
        for part in ["…以", "“", "”", "15"] {
            assert_eq!(font(part), 1, "{part}");
        }
        for part in ["Hi", ",", "OK…"] {
            assert_eq!(font(part), 0, "{part}");
        }
        assert_eq!(font("…"), 1);
    }

    #[test]
    fn vertical_text_sets_a_number_of_one_or_two_digits_upright() {
        let text = "第1话已有15年333次A12";
        let parts: Vec<(&str, bool)> = upright_groups(text, 0..text.len())
            .into_iter()
            .map(|(range, upright)| (&text[range], upright))
            .collect();
        assert_eq!(
            parts,
            [
                ("第", false),
                ("1", true),
                ("话已有", false),
                ("15", true),
                ("年333次A12", false),
            ]
        );
    }

    const FALLBACK_FAMILIES: &[&str] = &[
        "Segoe UI Symbol",
        "Segoe UI Emoji",
        "Noto Sans Symbols",
        "Noto Sans Symbols2",
        "Noto Color Emoji",
        "Apple Color Emoji",
        "Apple Symbols",
        "Symbola",
        "Arial Unicode MS",
    ];
    const SYMBOLS: &[char] = &[
        '\u{1F4A9}',
        '\u{1F642}',
        '\u{2665}',
        '\u{2605}',
        '\u{2713}',
        '\u{2603}',
        '\u{262F}',
        '\u{2691}',
        '\u{26A0}',
    ];

    #[test]
    fn a_grapheme_the_first_font_lacks_falls_back() {
        let mut library = Library::new();
        let primary = styled("").spans[0].fonts[0].clone();
        let has = |font: &Font, symbol: char| font.covers(symbol.encode_utf8(&mut [0; 4]));
        let (fallback, symbol): (Font, char) = SYMBOLS
            .iter()
            .filter(|symbol| !has(&primary, **symbol))
            .find_map(|symbol| {
                FALLBACK_FAMILIES.iter().find_map(|name| {
                    let font = library.family(name).ok()?;
                    (font.post_script_name != primary.post_script_name && has(&font, *symbol))
                        .then_some((font, *symbol))
                })
            })
            .expect("no fallback font with missing primary glyph found");

        let mut styled = styled(&format!("A{symbol}!"));
        styled.spans[0].fonts = vec![primary, fallback];
        let levels = BidiInfo::new(&styled.text, None).levels;
        let (runs, uncovered) = itemize(&styled, &levels);
        let font = |at: usize| {
            runs.iter()
                .find(|run| run.range.contains(&at))
                .expect("a run")
                .font
        };
        assert_eq!((font(0), font(1)), (0, 1));
        assert!(uncovered.is_empty());
    }
}
