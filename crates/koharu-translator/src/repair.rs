//! Finding translations a language model misaligned, so they can be asked for
//! again.
//!
//! Replies are matched to balloons by the id the model returns. A model that
//! splits one long sentence across two ids leaves the first one cut off
//! mid-sentence, and the next id then carries the rest of it; one that skips
//! an id leaves that balloon untranslated. Each segment is still well-formed
//! JSON, so the parser cannot tell. These checks look at the text instead.

/// Punctuation that ends a sentence, in the scripts manga comes in.
const SENTENCE_END: &[char] = &[
    '.', '?', '!', '…', '~', '〜', '～', '。', '？', '！', '」', '』', '）', ')', '♡', '♥', '"',
    '”', '’', '*',
];

/// Punctuation a finished line does not end on.
const CLAUSE_BREAK: &[char] = &[',', '、', ':', ';', '，', '：', '；'];

/// Spanish words a sentence cannot end on: articles, prepositions,
/// conjunctions and possessives that always need something after them.
const DANGLING_WORDS: &[&str] = &[
    "un", "una", "el", "la", "los", "las", "de", "del", "que", "y", "o", "a", "en", "con", "por",
    "para", "como", "mi", "tu", "su",
];

/// Lines this short are a reply that dropped its punctuation ("Sí", "Vale"),
/// not a cut.
const SHORT_WORDS: usize = 4;
const SHORT_CHARS: usize = 20;

/// A translation cut off mid-sentence: its source ends a sentence and it
/// stops on a clause break, or on a word that needs another after it.
/// Dropping the final punctuation alone ("Yes." → "Sí") is not a cut.
pub(crate) fn is_cut(source: &str, translation: &str) -> bool {
    let Some(source_end) = source.trim_end().chars().last() else {
        return false;
    };
    let translation = translation.trim_end();
    let Some(translation_end) = translation.chars().last() else {
        return false;
    };
    if !SENTENCE_END.contains(&source_end) {
        return false;
    }
    if CLAUSE_BREAK.contains(&translation_end) {
        return true;
    }
    let short = translation.split_whitespace().count() < SHORT_WORDS
        || translation.chars().count() < SHORT_CHARS;
    !short
        && translation
            .split_whitespace()
            .last()
            .map(|word| word.trim_matches(|character: char| !character.is_alphanumeric()))
            .is_some_and(|word| DANGLING_WORDS.contains(&word.to_lowercase().as_str()))
}

/// A segment the model never returned: the parser leaves the source in its
/// place. Sources without letters ("!?", "…") read the same in any language,
/// and so do short Latin-script ones: interjections and names ("OK!",
/// "Hmm...", "Mama?").
pub(crate) fn is_missing(source: &str, translation: &str) -> bool {
    let source = source.trim();
    if source != translation.trim() || !source.chars().any(char::is_alphabetic) {
        return false;
    }
    let latin = source
        .chars()
        .filter(|character| character.is_alphabetic())
        .all(is_latin);
    !(latin && source.split_whitespace().count() <= 2)
}

fn is_latin(character: char) -> bool {
    matches!(character, 'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

/// Whether a translation reads as the rest of the sentence before it: it
/// starts in lower case, or its source opens a question or exclamation that
/// the translation does not open with '¿' or '¡'.
fn continues_previous(source: &str, translation: &str) -> bool {
    let translation = translation.trim();
    let opened = translation.starts_with(['¿', '¡']);
    let Some(first) = translation
        .chars()
        .find(|character| character.is_alphabetic())
    else {
        return false;
    };
    if first.is_lowercase() && !opened {
        return true;
    }
    let source = source.trim();
    let opens_sentence = source
        .chars()
        .find(|character| character.is_alphabetic())
        .is_some_and(|character| !character.is_lowercase());
    let asks_or_exclaims = |text: &str| text.ends_with(['?', '!', '？', '！']);
    opens_sentence && asks_or_exclaims(source) && asks_or_exclaims(translation) && !opened
}

/// Segments worth asking for again: each cut segment with the segments after
/// it that read as the rest of its sentence, and each untranslated segment.
/// The run after a cut stops at the first segment that stands on its own, so
/// one cut does not resend the rest of the page.
pub(crate) fn suspects(sources: &[String], translations: &[String]) -> Vec<usize> {
    let mut flagged = vec![false; sources.len()];
    let mut index = 0;
    while index < sources.len() {
        if is_cut(&sources[index], &translations[index]) {
            flagged[index] = true;
            index += 1;
            while index < sources.len() && continues_previous(&sources[index], &translations[index])
            {
                flagged[index] = true;
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    flagged
        .into_iter()
        .enumerate()
        .filter(|&(index, flag)| flag || is_missing(&sources[index], &translations[index]))
        .map(|(index, _)| index)
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
        assert!(is_cut(
            "Didn't we plan to use today as a side dish?",
            "¿No habíamos planeado usar hoy como un",
        ));
    }

    #[test]
    fn complete_or_unpunctuated_lines_are_not_cut() {
        assert!(!is_cut("Honey... Do you know?", "Cariño... ¿Sabes?"));
        assert!(!is_cut("Hahaha", "Jajaja"));
        assert!(!is_cut("Yeah...", "Sí..."));
        assert!(!is_cut("", "algo"));
    }

    #[test]
    fn dropped_final_punctuation_is_not_cut() {
        assert!(!is_cut("Yes.", "Sí"));
        assert!(!is_cut("Okay.", "Vale"));
        assert!(!is_cut("はい。", "Sí"));
        assert!(!is_cut("Tanaka-kun!", "Tanaka-kun"));
        assert!(!is_cut(
            "I'm going to the store with my sister.",
            "Voy a la tienda con mi hermana"
        ));
    }

    #[test]
    fn only_lettered_sources_can_be_missing() {
        assert!(is_missing("Are you okay?", "Are you okay?"));
        assert!(!is_missing("!?", "!?"));
        assert!(!is_missing("…", "…"));
        assert!(!is_missing("Are you okay?", "¿Estás bien?"));
    }

    #[test]
    fn short_latin_lines_may_stay_the_same() {
        assert!(!is_missing("OK!", "OK!"));
        assert!(!is_missing("Hmm...", "Hmm..."));
        assert!(!is_missing("Mama?", "Mama?"));
        assert!(is_missing("大丈夫？", "大丈夫？"));
        assert!(is_missing("Where are you going?", "Where are you going?"));
    }

    #[test]
    fn a_cut_and_its_continuation_are_suspect() {
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

        // The run stops at "¡Hoy es…", which stands on its own, so the shifted
        // segments after it are left as they are rather than resending the
        // rest of the page.
        assert_eq!(suspects(&sources, &translations), vec![1, 2]);
    }

    #[test]
    fn a_continuation_without_an_opening_mark_is_suspect() {
        let sources = strings(&[
            "Didn't we plan to use today as a side dish?",
            "Today is a dangerous day?",
            "Yeah...",
        ]);
        let translations = strings(&[
            "¿No habíamos planeado usar hoy como un",
            "Acompañamiento para el primer hijo?",
            "Sí...",
        ]);

        assert_eq!(suspects(&sources, &translations), vec![0, 1]);
    }

    #[test]
    fn a_clean_page_has_no_suspects() {
        let sources = strings(&["No way?", "Yeah...", "!?"]);
        let translations = strings(&["¿Cómo que no?", "Sí...", "!?"]);

        assert!(suspects(&sources, &translations).is_empty());
    }

    #[test]
    fn dropped_punctuation_and_unchanged_names_do_not_flag_the_page() {
        let sources = strings(&[
            "Okay.",
            "Where are you going?",
            "To the store.",
            "Wait for me!",
        ]);
        let translations = strings(&["Vale", "¿A dónde vas?", "A la tienda.", "¡Espérame!"]);
        assert!(suspects(&sources, &translations).is_empty());

        let sources = strings(&["Yes.", "はい。", "Tanaka-kun!", "OK!", "Hmm...", "Mama?"]);
        let translations = strings(&["Sí", "Sí", "Tanaka-kun", "OK!", "Hmm...", "Mama?"]);
        assert!(suspects(&sources, &translations).is_empty());
    }

    #[test]
    fn a_skipped_segment_alone_is_suspect() {
        let sources = strings(&["No way?", "Where are you going?", "Honey?"]);
        let translations = strings(&["¿Cómo que no?", "Where are you going?", "¿Cariño?"]);

        assert_eq!(suspects(&sources, &translations), vec![1]);
    }
}
