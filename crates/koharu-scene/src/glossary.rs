use std::collections::BTreeSet;

use icu_normalizer::ComposingNormalizerBorrowed;
use revision::revisioned;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{Component, Result, ValidationContext};

#[revisioned(revision = 1)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GlossaryCategory {
    Person,
    Place,
    Organization,
    Title,
    Skill,
    Item,
    Terminology,
    #[default]
    Other,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Type)]
pub struct GlossaryEntry {
    pub source: String,
    pub target: String,
    pub category: GlossaryCategory,
    pub notes: String,
    pub enabled: bool,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, Type)]
pub struct GlossaryCandidate {
    pub source: String,
    pub suggested_target: String,
    pub category: GlossaryCategory,
    pub occurrences: u32,
    pub page_count: u32,
}

#[revisioned(revision = 1)]
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize, Type)]
#[serde(default)]
pub struct ProjectGlossary {
    pub entries: Vec<GlossaryEntry>,
    pub candidates: Vec<GlossaryCandidate>,
    pub ignored: Vec<String>,
}

impl Component for ProjectGlossary {
    const KIND: &'static str = "dev.koharu.project.glossary";

    fn validate(&self, _context: &ValidationContext<'_>) -> Result<()> {
        self.validate_entries()
    }
}

impl ProjectGlossary {
    pub fn validate_entries(&self) -> Result<()> {
        if self.entries.len() > 10_000
            || self.candidates.len() > 10_000
            || self.ignored.len() > 10_000
        {
            return Err(crate::Error::invalid("glossary exceeds 10000 entries"));
        }
        let mut sources = BTreeSet::new();
        for entry in &self.entries {
            let key = normalize_term(&entry.source);
            if key.is_empty()
                || entry.source.len() > 1024
                || entry.target.len() > 4096
                || entry.notes.len() > 4096
                || (entry.enabled && entry.target.trim().is_empty())
            {
                return Err(crate::Error::invalid(
                    "glossary entry has an empty or oversized field",
                ));
            }
            if !sources.insert(key) {
                return Err(crate::Error::invalid(
                    "glossary contains duplicate source terms",
                ));
            }
        }
        let mut candidates = BTreeSet::new();
        for candidate in &self.candidates {
            let key = normalize_term(&candidate.source);
            if key.is_empty()
                || candidate.source.len() > 1024
                || candidate.suggested_target.len() > 4096
                || !candidates.insert(key)
            {
                return Err(crate::Error::invalid(
                    "invalid or duplicate glossary candidate",
                ));
            }
        }
        if self.ignored.iter().any(|source| source.len() > 1024) {
            return Err(crate::Error::invalid("ignored glossary term is too long"));
        }
        Ok(())
    }

    pub fn replace_candidates(&mut self, candidates: Vec<GlossaryCandidate>) {
        let excluded = self
            .entries
            .iter()
            .map(|entry| normalize_term(&entry.source))
            .chain(self.ignored.iter().map(|source| normalize_term(source)))
            .collect::<BTreeSet<_>>();
        let previous = std::mem::take(&mut self.candidates)
            .into_iter()
            .map(|candidate| (normalize_term(&candidate.source), candidate))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        self.candidates = candidates
            .into_iter()
            .filter(|candidate| {
                let key = normalize_term(&candidate.source);
                !excluded.contains(&key) && seen.insert(key)
            })
            .map(|mut candidate| {
                if let Some(existing) = previous.get(&normalize_term(&candidate.source)) {
                    candidate
                        .suggested_target
                        .clone_from(&existing.suggested_target);
                    candidate.category = existing.category;
                }
                candidate
            })
            .collect();
    }
}

pub fn normalize_text(text: &str) -> String {
    ComposingNormalizerBorrowed::new_nfkc()
        .normalize(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn normalize_term(text: &str) -> String {
    normalize_text(text)
        .trim_matches(|character: char| {
            character.is_whitespace() || "「」『』\"'“”‘’。、,.!?！?:;：；".contains(character)
        })
        .to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlossaryMatch {
    pub start: usize,
    pub end: usize,
    pub entry: usize,
}

// Offsets refer to the normalized text returned alongside the matches.
pub fn glossary_matches(source: &str, entries: &[GlossaryEntry]) -> (String, Vec<GlossaryMatch>) {
    let text = normalize_text(source);
    let mut terms = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.enabled && !entry.target.trim().is_empty())
        .map(|(index, entry)| (index, normalize_term(&entry.source)))
        .filter(|(_, term)| !term.is_empty())
        .collect::<Vec<_>>();
    terms.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.1.cmp(&b.1)));
    let mut matches = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        if let Some((index, term)) = terms.iter().find(|(_, term)| {
            text[offset..].starts_with(term.as_str()) && word_boundary(&text, offset, term)
        }) {
            let end = offset + term.len();
            matches.push(GlossaryMatch {
                start: offset,
                end,
                entry: *index,
            });
            offset = end;
        } else {
            offset += text[offset..]
                .chars()
                .next()
                .expect("offset is inside text")
                .len_utf8();
        }
    }
    (text, matches)
}

fn word_boundary(text: &str, offset: usize, term: &str) -> bool {
    let latin = |character: char| character.is_ascii_alphanumeric() || character == '_';
    !(term.chars().next().is_some_and(latin)
        && text[..offset].chars().next_back().is_some_and(latin)
        || term.chars().next_back().is_some_and(latin)
            && text[offset + term.len()..]
                .chars()
                .next()
                .is_some_and(latin))
}

pub fn relevant_glossary(segments: &[String], entries: &[GlossaryEntry]) -> Vec<GlossaryEntry> {
    let selected = segments
        .iter()
        .flat_map(|segment| {
            glossary_matches(segment, entries)
                .1
                .into_iter()
                .map(|value| value.entry)
        })
        .collect::<BTreeSet<_>>();
    let mut entries = selected
        .into_iter()
        .map(|index| entries[index].clone())
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        normalize_term(&b.source)
            .len()
            .cmp(&normalize_term(&a.source).len())
            .then_with(|| a.source.cmp(&b.source))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(source: &str, target: &str) -> GlossaryEntry {
        GlossaryEntry {
            source: source.into(),
            target: target.into(),
            category: GlossaryCategory::Person,
            notes: String::new(),
            enabled: true,
        }
    }

    #[test]
    fn normalized_longest_matches_exclude_disabled_and_unrelated_terms() {
        let mut disabled = entry("無効", "disabled");
        disabled.enabled = false;
        let entries = vec![
            entry("魔導", "magic"),
            entry("魔導院", "Academy"),
            entry("エーテル", "Ether"),
            entry("Ann", "安"),
            disabled,
        ];
        let selected = relevant_glossary(&["魔導院でｴｰﾃﾙ。Anna 無効".into()], &entries);
        assert_eq!(
            selected
                .iter()
                .map(|entry| entry.target.as_str())
                .collect::<Vec<_>>(),
            ["Ether", "Academy"]
        );
        let (_, matches) = glossary_matches("魔導院と魔導", &entries);
        assert_eq!(
            matches.iter().map(|value| value.entry).collect::<Vec<_>>(),
            [1, 0]
        );
        assert_eq!(normalize_term(" 「ｴｰﾃﾙ」 "), "エーテル");
    }

    #[tokio::test]
    async fn glossary_roundtrips_and_old_projects_need_no_migration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project");
        let mut session = crate::Session::create(&path).await.unwrap();
        assert!(
            session
                .snapshot()
                .project_component::<ProjectGlossary>()
                .unwrap()
                .is_none()
        );
        let mut glossary = ProjectGlossary {
            entries: vec![entry("高橋", "高桥")],
            ..Default::default()
        };
        let patch = session
            .snapshot()
            .patch(|edit| edit.set_project(&glossary))
            .unwrap();
        session.commit(patch).await.unwrap();
        drop(session);
        let mut session = crate::Session::open(&path).await.unwrap();
        assert_eq!(
            session
                .snapshot()
                .project_component::<ProjectGlossary>()
                .unwrap(),
            Some(glossary.clone())
        );
        glossary.entries[0].target = "Takahashi".into();
        glossary.entries[0].enabled = false;
        let commit = session
            .commit(
                session
                    .snapshot()
                    .patch(|edit| edit.set_project(&glossary))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            session
                .snapshot()
                .project_component::<ProjectGlossary>()
                .unwrap(),
            Some(glossary.clone())
        );
        session.undo(commit.revision).await.unwrap();
        assert!(
            session
                .snapshot()
                .project_component::<ProjectGlossary>()
                .unwrap()
                .unwrap()
                .entries[0]
                .enabled
        );
        drop(commit);
        glossary.entries.clear();
        session
            .commit(
                session
                    .snapshot()
                    .patch(|edit| edit.set_project(&glossary))
                    .unwrap(),
            )
            .await
            .unwrap();
        drop(session);
        assert!(
            crate::Session::open(&path)
                .await
                .unwrap()
                .snapshot()
                .project_component::<ProjectGlossary>()
                .unwrap()
                .unwrap()
                .entries
                .is_empty()
        );
    }

    #[test]
    fn candidates_are_separate_and_confirmed_terms_are_not_recreated() {
        let mut glossary = ProjectGlossary {
            entries: vec![entry("高橋", "高桥")],
            ignored: vec!["無視".into()],
            ..Default::default()
        };
        glossary.replace_candidates(
            ["高橋", "無視", "魔導院"]
                .map(|source| GlossaryCandidate {
                    source: source.into(),
                    suggested_target: String::new(),
                    category: GlossaryCategory::Other,
                    occurrences: 2,
                    page_count: 2,
                })
                .to_vec(),
        );
        assert_eq!(glossary.candidates.len(), 1);
        glossary.candidates[0].suggested_target = "Academy".into();
        glossary.candidates[0].category = GlossaryCategory::Organization;
        let mut refreshed = glossary.candidates[0].clone();
        refreshed.source = "「魔導院」".into();
        refreshed.suggested_target.clear();
        refreshed.category = GlossaryCategory::Other;
        refreshed.occurrences = 8;
        glossary.replace_candidates(vec![refreshed.clone(), refreshed]);
        assert_eq!(glossary.candidates.len(), 1);
        assert_eq!(glossary.candidates[0].suggested_target, "Academy");
        assert_eq!(
            glossary.candidates[0].category,
            GlossaryCategory::Organization
        );
        assert_eq!(glossary.candidates[0].occurrences, 8);
        assert!(relevant_glossary(&["魔導院".into()], &glossary.entries).is_empty());
        glossary.entries.push(entry("「高橋」", "duplicate"));
        assert!(glossary.validate_entries().is_err());
    }
}
