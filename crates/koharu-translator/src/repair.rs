//! Finding translations a language model misaligned, so they can be asked for
//! again.
//!
//! Replies are matched to balloons by the id the model returns. A model that
//! splits one long sentence across two ids leaves the first one cut off
//! mid-sentence, and every later id then carries its neighbour's text; one
//! that skips an id leaves that balloon untranslated. Each segment is still
//! well-formed JSON, so the parser cannot tell. These checks look at the text
//! instead.

/// Punctuation that ends a sentence, in the scripts manga comes in.
const SENTENCE_END: &[char] = &[
    '.', '?', '!', '…', '~', '〜', '～', '。', '？', '！', '」', '』', '）', ')', '♡', '♥', '"',
    '”', '’', '*',
];

/// A translation cut off mid-sentence: its source ends a sentence and it ends
/// on a word or a comma instead.
pub(crate) fn is_cut(source: &str, translation: &str) -> bool {
    let Some(source_end) = source.trim_end().chars().last() else {
        return false;
    };
    let Some(translation_end) = translation.trim_end().chars().last() else {
        return false;
    };
    SENTENCE_END.contains(&source_end)
        && (translation_end.is_alphanumeric() || matches!(translation_end, ',' | '、' | ';' | ':'))
}

/// A segment the model never returned: the parser leaves the source in its
/// place. Sources without letters ("!?", "…") read the same in any language.
pub(crate) fn is_missing(source: &str, translation: &str) -> bool {
    source.trim() == translation.trim() && source.chars().any(char::is_alphabetic)
}

/// Segments worth asking for again. A cut segment usually means the model
/// split it and pushed everything after it one id along, so every segment from
/// the first cut on is suspect.
pub(crate) fn suspects(sources: &[String], translations: &[String]) -> Vec<usize> {
    let first_cut = sources
        .iter()
        .zip(translations)
        .position(|(source, translation)| is_cut(source, translation));
    (0..sources.len())
        .filter(|&index| {
            first_cut.is_some_and(|cut| index >= cut)
                || is_missing(&sources[index], &translations[index])
        })
        .collect()
}

/// Whether a retried translation may replace the first one.
pub(crate) fn acceptable(source: &str, translation: &str) -> bool {
    !translation.trim().is_empty()
        && !is_cut(source, translation)
        && !is_missing(source, translation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_sentence_cut_mid_way_is_cut() {
        assert!(is_cut(
            "Didn't we plan to use today as a side dish to conceive the first child?",
            "¿No habíamos planeado usar la experiencia de hoy como un ",
        ));
        assert!(is_cut("待って。", "Espera,"));
    }

    #[test]
    fn complete_or_unpunctuated_lines_are_not_cut() {
        assert!(!is_cut("Honey... Do you know?", "Cariño... ¿Sabes?"));
        assert!(!is_cut("Hahaha", "Jajaja"));
        assert!(!is_cut("Yeah...", "Sí..."));
        assert!(!is_cut("", "algo"));
    }

    #[test]
    fn only_lettered_sources_can_be_missing() {
        assert!(is_missing("Are you okay?", "Are you okay?"));
        assert!(!is_missing("!?", "!?"));
        assert!(!is_missing("…", "…"));
        assert!(!is_missing("Are you okay?", "¿Estás bien?"));
    }

    #[test]
    fn everything_after_a_cut_is_suspect() {
        let sources = strings(&[
            "No way?",
            "Didn't we plan to use today as a side dish?",
            "Today is a dangerous day?",
            "Honey... Do you know what this means?",
            "Yeah...",
        ]);
        let translations = strings(&[
            "¿Cómo que no?",
            "¿No habíamos planeado usar hoy como un",
            "acompañamiento?",
            "¡Hoy es un día peligroso!",
            "Honey... Do you know what this means?",
        ]);

        assert_eq!(suspects(&sources, &translations), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_clean_page_has_no_suspects() {
        let sources = strings(&["No way?", "Yeah...", "!?"]);
        let translations = strings(&["¿Cómo que no?", "Sí...", "!?"]);

        assert!(suspects(&sources, &translations).is_empty());
    }

    #[test]
    fn a_skipped_segment_alone_is_suspect() {
        let sources = strings(&["No way?", "Yeah...", "Honey?"]);
        let translations = strings(&["¿Cómo que no?", "Yeah...", "¿Cariño?"]);

        assert_eq!(suspects(&sources, &translations), vec![1]);
    }
}
