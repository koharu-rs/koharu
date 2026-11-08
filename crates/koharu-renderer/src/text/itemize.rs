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
use crate::script::harfrust_script;

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
/// back.
pub(super) fn itemize(styled: &Styled, levels: &[Level]) -> (Vec<Run>, Vec<char>) {
    let mut runs = Vec::new();
    let mut uncovered = Vec::new();
    for (piece, span) in pieces(styled, levels) {
        let parts = match styled.vertical {
            true => upright_groups(&styled.text, piece)
                .into_iter()
                .flat_map(|(part, group)| match group {
                    true => vec![(part, Orientation::TateChuYoko)],
                    false => orientations(&styled.text, part),
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
                let run = Run {
                    range,
                    span,
                    font: 0,
                    level,
                    script: harfrust_script(script),
                    orientation,
                    glyphs: 0..0,
                };
                fall_back(styled, run, &mut runs, &mut uncovered);
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

/// Pushes `run` in the first of its span's fonts when that covers it, else
/// cut where, grapheme by grapheme, the first font that covers it changes.
/// A grapheme no font covers takes the first, and its characters no font has
/// go to `uncovered`.
fn fall_back(styled: &Styled, run: Run, runs: &mut Vec<Run>, uncovered: &mut Vec<char>) {
    let fonts = &styled.spans[run.span].fonts;
    if fonts.is_empty() {
        return;
    }
    let text = &styled.text[run.range.clone()];
    if fonts[0].covers(text) {
        runs.push(run);
        return;
    }
    // Graphemes keep combining marks, variation selectors, and emoji ZWJ
    // sequences together instead of falling back per Unicode scalar.
    let boundaries: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(text).collect();
    let mut start = run.range.start;
    let mut current = None;
    for pair in boundaries.windows(2) {
        let grapheme = &text[pair[0]..pair[1]];
        let font = fonts
            .iter()
            .position(|font| font.covers(grapheme))
            .unwrap_or_else(|| {
                uncovered.extend(grapheme.chars().filter(|character| {
                    let mut utf8 = [0; 4];
                    let character = character.encode_utf8(&mut utf8);
                    !fonts.iter().any(|font| font.covers(character))
                }));
                0
            });
        if let Some(current) = current
            && current != font
        {
            let end = run.range.start + pair[0];
            runs.push(Run {
                range: start..end,
                font: current,
                ..run.clone()
            });
            start = end;
        }
        current = Some(font);
    }
    runs.push(Run {
        range: start..run.range.end,
        font: current.unwrap_or(0),
        ..run
    });
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

/// `range` of vertical text cut where it turns, grapheme by grapheme. What
/// Unicode's Vertical_Orientation (UAX #50) rotates, such as Latin letters
/// and digits, is set sideways, as Photoshop and CSS's `text-orientation:
/// mixed` set it; the rest stands upright, `vert` giving the characters it
/// transforms their vertical forms. `!` and `?` stand upright in full width,
/// as manga sets them, and so does an emoji: a grapheme with the emoji
/// variation selector or a keycap, which UAX #50 keeps upright.
fn orientations(text: &str, range: Range<usize>) -> Vec<(Range<usize>, Orientation)> {
    let orientation = CodePointMapData::<VerticalOrientation>::new();
    let part = &text[range.clone()];
    let boundaries: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(part).collect();
    let mut parts: Vec<(Range<usize>, Orientation)> = Vec::new();
    for pair in boundaries.windows(2) {
        let grapheme = &part[pair[0]..pair[1]];
        let first = grapheme.chars().next().expect("a grapheme has a character");
        let turned = match orientation.get(first) == VerticalOrientation::Rotated
            && !is_exclamation(first)
            && !grapheme.contains(['\u{fe0f}', '\u{20e3}'])
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

/// `range` cut around its runs of two or three `!` and `?`, which vertical
/// text sets upright side by side in one cell; each part says whether it is
/// such a group.
fn upright_groups(text: &str, range: Range<usize>) -> Vec<(Range<usize>, bool)> {
    let mut parts = Vec::new();
    let mut start = range.start;
    let mut chars = text[range.clone()].char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if !is_exclamation(ch) {
            continue;
        }
        let group_start = range.start + offset;
        let mut group_end = group_start + ch.len_utf8();
        let mut count = 1;
        while let Some(&(next, next_ch)) = chars.peek() {
            if !is_exclamation(next_ch) || count == 3 {
                break;
            }
            group_end = range.start + next + next_ch.len_utf8();
            count += 1;
            chars.next();
        }
        if count < 2 {
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
    use koharu_scene::Color;
    use unicode_bidi::BidiInfo;

    use super::*;
    use crate::{
        fonts::{Font, FontSystem},
        text::style::Span,
        text::testing::{primary, styled},
    };

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
            ..styled(text, Vec::new())
        };
        assert_eq!(
            cut(&styled),
            [("ab", 0), ("c", 0), ("d ", 1), ("\u{05e9}\u{05dc}", 1)]
        );
    }

    #[test]
    fn vertical_text_turns_latin_and_digits_sideways() {
        let text = "東京Tokyo Tower、高さ333!";
        let parts: Vec<(&str, Orientation)> = orientations(text, 0..text.len())
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
            orientations(keycap, 0..keycap.len()),
            [(0..keycap.len(), Orientation::Upright)]
        );
    }

    #[test]
    fn a_sideways_right_to_left_word_keeps_its_direction() {
        let styled = Styled {
            vertical: true,
            ..styled(
                "東\u{05e9}\u{05dc}\u{05d5}\u{05dd}",
                vec![primary(&mut FontSystem::new())],
            )
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
        let mut fonts = FontSystem::new();
        let primary = primary(&mut fonts);
        let (fallback, symbol): (Font, char) = SYMBOLS
            .iter()
            .filter(|symbol| !primary.has_glyph(**symbol))
            .find_map(|symbol| {
                FALLBACK_FAMILIES.iter().find_map(|name| {
                    let font = fonts.query_family(name).ok()?;
                    (font.post_script_name() != primary.post_script_name()
                        && font.has_glyph(*symbol))
                    .then_some((font, *symbol))
                })
            })
            .expect("no fallback font with missing primary glyph found");

        let styled = styled(&format!("A{symbol}!"), vec![primary, fallback]);
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

    #[test]
    fn characters_no_font_has_are_reported_once() {
        let styled = styled("a\u{378}b\u{378}", vec![primary(&mut FontSystem::new())]);
        let levels = BidiInfo::new(&styled.text, None).levels;
        // U+0378 is unassigned, so no font has it.
        assert_eq!(itemize(&styled, &levels).1, ['\u{378}']);
    }
}
