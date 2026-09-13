use anyhow::{Context as _, Result};
use koharu_scene::glossary_matches;

use crate::TranslationRequest;

enum Piece {
    Literal(String),
    Translated(usize),
}

// Promptless services cannot obey a system glossary; keep confirmed spans out of their input.
pub(crate) struct ProtectedSegments {
    segments: Vec<Vec<Piece>>,
}

impl ProtectedSegments {
    pub(crate) fn prepare(request: &mut TranslationRequest) -> Self {
        let mut fragments = Vec::new();
        let segments = std::mem::take(&mut request.segments)
            .into_iter()
            .map(|source| {
                let (normalized, matches) = glossary_matches(&source, &request.glossary);
                let mut pieces = Vec::new();
                if matches.is_empty() {
                    append(&mut pieces, &mut fragments, &source);
                    return pieces;
                }
                let mut offset = 0;
                for term in matches {
                    append(&mut pieces, &mut fragments, &normalized[offset..term.start]);
                    pieces.push(Piece::Literal(request.glossary[term.entry].target.clone()));
                    offset = term.end;
                }
                append(&mut pieces, &mut fragments, &normalized[offset..]);
                pieces
            })
            .collect();
        request.segments = fragments;
        Self { segments }
    }

    pub(crate) fn restore(self, translations: &[String]) -> Result<Vec<String>> {
        self.segments
            .into_iter()
            .map(|pieces| {
                let mut text = String::new();
                for piece in pieces {
                    match piece {
                        Piece::Literal(value) => text.push_str(&value),
                        Piece::Translated(index) => text.push_str(
                            translations
                                .get(index)
                                .context("provider omitted a glossary-adjacent segment")?,
                        ),
                    }
                }
                Ok(text)
            })
            .collect()
    }
}

fn append(pieces: &mut Vec<Piece>, fragments: &mut Vec<String>, text: &str) {
    if text.is_empty() {
        return;
    }
    if text.trim().is_empty() {
        pieces.push(Piece::Literal(text.to_owned()));
    } else {
        pieces.push(Piece::Translated(fragments.len()));
        fragments.push(text.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_scene::{GlossaryCategory, GlossaryEntry};
    fn entries() -> Vec<GlossaryEntry> {
        vec![
            GlossaryEntry {
                source: "高橋".into(),
                target: "高桥".into(),
                category: GlossaryCategory::Person,
                notes: "surname".into(),
                enabled: true,
            },
            GlossaryEntry {
                source: "無関係".into(),
                target: "unrelated".into(),
                category: GlossaryCategory::Other,
                notes: String::new(),
                enabled: true,
            },
        ]
    }

    #[test]
    fn shared_prompt_contains_only_relevant_confirmed_terms() {
        let request = TranslationRequest::new(["高橋さん"], crate::Language::English)
            .with_glossary(&entries());
        let (system, user) = crate::prompt::prompts(&request).unwrap();
        assert!(system.contains("use the specified target consistently"));
        assert!(system.contains("take precedence"));
        assert!(user.contains("高桥"));
        assert!(!user.contains("unrelated"));
    }

    #[test]
    fn promptless_services_preserve_glossary_and_segment_count() {
        let mut request =
            TranslationRequest::new(["高橋", "高橋さん", "other"], crate::Language::English)
                .with_glossary(&entries());
        let protected = ProtectedSegments::prepare(&mut request);
        assert_eq!(request.segments, ["さん", "other"]);
        assert_eq!(
            protected.restore(&["先生".into(), "其他".into()]).unwrap(),
            ["高桥", "高桥先生", "其他"]
        );
    }
}
