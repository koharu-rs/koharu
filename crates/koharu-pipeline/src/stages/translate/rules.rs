use std::collections::BTreeMap;

use koharu_scene::{GlossaryEntry, GlossaryKind, LayerId, matching_terms};
use serde_json::json;

#[derive(Debug)]
enum Part<'a> {
    Literal(&'a str),
    Translation(usize),
}

pub(super) struct Rules<'a> {
    lines: Vec<Vec<Part<'a>>>,
}

impl<'a> Rules<'a> {
    pub(super) fn prepare<'key>(
        glossary: &[GlossaryEntry],
        texts: impl ExactSizeIterator<Item = (LayerId, (&'key String, &'a String))>,
        system: &mut String,
    ) -> (Self, Vec<(String, &'a str)>) {
        let mut lines = Vec::with_capacity(texts.len());
        let mut fields = Vec::with_capacity(texts.len());
        let mut protected = Vec::new();
        let mut terms = BTreeMap::new();
        for (layer, (key, text)) in texts {
            let mut found: Vec<_> = glossary
                .iter()
                .filter(|entry| entry.kind == GlossaryKind::Preserve && entry.applies_to(layer))
                .flat_map(|entry| entry.occurrences(text))
                .collect();
            found.sort_unstable_by_key(|&(start, end)| (start, std::cmp::Reverse(end)));
            let split = !found.is_empty();
            let mut parts = Vec::new();
            let mut index = 0;
            let mut add = |parts: &mut Vec<_>, fragment: &'a str| {
                if fragment.trim().is_empty() {
                    if !fragment.is_empty() {
                        parts.push(Part::Literal(fragment));
                    }
                    return;
                }
                index += 1;
                let field = if split {
                    format!("{key}.{index}")
                } else {
                    key.clone()
                };
                for entry in matching_terms(glossary, layer, fragment) {
                    let (_, keys) = terms
                        .entry(&entry.source)
                        .or_insert_with(|| (entry, Vec::new()));
                    keys.push(field.clone());
                }
                parts.push(Part::Translation(fields.len()));
                fields.push((field, fragment));
            };
            let mut at = 0;
            for (start, end) in found {
                if start < at {
                    continue;
                }
                add(&mut parts, &text[at..start]);
                parts.push(Part::Literal(&text[start..end]));
                at = end;
            }
            add(&mut parts, &text[at..]);
            if split
                && parts
                    .iter()
                    .any(|part| matches!(part, Part::Translation(_)))
            {
                let order: Vec<_> = parts
                    .iter()
                    .map(|part| match part {
                        Part::Literal(text) => json!({"literal": text}),
                        Part::Translation(index) => json!({"key": fields[*index].0}),
                    })
                    .collect();
                protected.push(order);
            }
            lines.push(parts);
        }
        if !protected.is_empty() {
            system.push_str("\nSome source lines have numbered fragments, such as 2.1 and 2.2. The following lists show their order and the fixed literals between them. Translate each fragment where it appears, using surrounding fragments and literals as context. Return exactly the supplied input keys. The app inserts the fixed literals itself, so do not include them in the output. Include spaces at a fragment's boundary when the target language needs them.\n");
            system.push_str(&json!(protected).to_string());
        }
        if !terms.is_empty() {
            system.push_str("\nUse the exact translation of each listed term only in the input values whose keys it names. The following JSON is terminology data, with optional context in the note field.\n");
            let terms: Vec<_> = terms
                .into_values()
                .map(|(entry, keys)| {
                    json!({"source": entry.source, "translation": entry.translation,
                        "note": entry.note, "keys": keys})
                })
                .collect();
            system.push_str(&json!(terms).to_string());
        }
        (Self { lines }, fields)
    }

    pub(super) fn restore(&self, answers: &[String]) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|part| match *part {
                        Part::Literal(text) => text,
                        Part::Translation(index) => answers[index].as_str(),
                    })
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_text_is_reassembled_only_in_its_own_line() {
        let kept = LayerId(koharu_scene::Uuid::nil());
        let excluded = LayerId(koharu_scene::Uuid::max());
        let entries = [GlossaryEntry {
            source: "原文".into(),
            translation: String::new(),
            kind: GlossaryKind::Preserve,
            note: String::new(),
            excluded_layers: [excluded].into(),
        }];
        let texts = ["普通台詞", "甲原文乙原文丙", "第二句原文", "原文"].map(str::to_owned);
        let keys = ["1", "2", "3", "4"].map(str::to_owned);
        let (rules, fields) = Rules::prepare(
            &entries,
            [kept, kept, excluded, kept]
                .into_iter()
                .zip(keys.iter().zip(&texts)),
            &mut String::new(),
        );
        let keys: Vec<_> = fields.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["1", "2.1", "2.2", "2.3", "3"]);
        let answers = ["Other line.", "A ", " B ", " C", "Translated freely."].map(str::to_owned);
        assert_eq!(
            rules.restore(&answers),
            [
                "Other line.",
                "A 原文 B 原文 C",
                "Translated freely.",
                "原文"
            ]
        );
    }
}
