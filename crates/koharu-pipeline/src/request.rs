use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{ProgressSink, Scope, Stage};
use koharu_scene::{EntityId, Glossary, GlossaryKind, normalize_glossary_source};
use koharu_translator::{TerminologyEntry, TerminologyKind};

#[derive(Clone, Debug)]
pub struct InpaintingMask {
    pub page: EntityId,
    pub png: Arc<[u8]>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, Type)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    #[default]
    Full,
    Through {
        stage: Stage,
    },
    Only {
        stage: Stage,
    },
    Stages {
        stages: Vec<Stage>,
    },
}

impl Operation {
    pub(crate) fn stages(&self) -> Result<Vec<Stage>> {
        let stages = match self {
            Self::Full => Stage::ALL.to_vec(),
            Self::Through {
                stage: Stage::Detection,
            } => vec![Stage::Detection],
            Self::Through { stage: Stage::Ocr } => vec![Stage::Detection, Stage::Ocr],
            Self::Through {
                stage: Stage::Translation,
            } => {
                vec![Stage::Detection, Stage::Ocr, Stage::Translation]
            }
            Self::Through {
                stage: Stage::Inpainting,
            } => vec![Stage::Detection, Stage::Inpainting],
            Self::Only { stage } => vec![*stage],
            Self::Stages { stages } => Stage::ALL
                .into_iter()
                .filter(|stage| stages.contains(stage))
                .collect(),
        };
        if stages.is_empty() {
            bail!("at least one pipeline stage must be selected");
        }
        Ok(stages)
    }
}

#[derive(Clone)]
pub struct Request {
    pub operation: Operation,
    pub scope: Scope,
    pub stop: StopToken,
    pub progress: Option<ProgressSink>,
    pub inpainting_mask: Option<InpaintingMask>,
    pub terminology: Arc<[TerminologyEntry]>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            operation: Operation::Full,
            scope: Scope::Project,
            stop: StopToken::default(),
            progress: None,
            inpainting_mask: None,
            terminology: Arc::from([]),
        }
    }
}

impl Request {
    #[must_use]
    pub fn with_terminology(mut self, terminology: impl Into<Arc<[TerminologyEntry]>>) -> Self {
        self.terminology = terminology.into();
        self
    }
}

#[must_use]
pub fn terminology_from_glossary(glossary: &Glossary) -> Arc<[TerminologyEntry]> {
    if !glossary.enabled {
        return Arc::from([]);
    }
    let mut entries = glossary
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .enabled
                .then_some(entry.translation.as_ref())
                .flatten()
                .map(|translation| {
                    let kind = terminology_kind(entry.kind);
                    (
                        (kind, normalize_glossary_source(&entry.source), entry.id),
                        TerminologyEntry {
                            source: entry.source.clone(),
                            translation: translation.clone(),
                            kind,
                        },
                    )
                })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    entries.into_iter().map(|(_, entry)| entry).collect()
}

fn terminology_kind(kind: GlossaryKind) -> TerminologyKind {
    match kind {
        GlossaryKind::Person => TerminologyKind::Person,
        GlossaryKind::Place => TerminologyKind::Place,
        GlossaryKind::Organization => TerminologyKind::Organization,
        GlossaryKind::Item => TerminologyKind::Item,
        GlossaryKind::Ability => TerminologyKind::Ability,
        GlossaryKind::Term => TerminologyKind::Term,
        GlossaryKind::Other => TerminologyKind::Other,
    }
}

#[derive(Clone, Default)]
pub struct StopToken(Arc<AtomicBool>);

impl StopToken {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{
        Glossary, GlossaryEntry, GlossaryEntryId, GlossaryKind, GlossaryValueOrigin,
    };
    use koharu_translator::{TerminologyEntry, TerminologyKind};

    use super::{Request, terminology_from_glossary};

    fn entry(
        source: &str,
        translation: Option<&str>,
        kind: GlossaryKind,
        enabled: bool,
    ) -> GlossaryEntry {
        GlossaryEntry {
            id: GlossaryEntryId::new(),
            source: source.to_owned(),
            translation: translation.map(str::to_owned),
            kind,
            enabled,
            confidence: None,
            occurrence_count: 0,
            examples: Vec::new(),
            source_origin: GlossaryValueOrigin::User,
            translation_origin: translation.map(|_| GlossaryValueOrigin::User),
            present_in_last_scan: true,
        }
    }

    fn glossary(entries: Vec<GlossaryEntry>) -> Glossary {
        Glossary {
            enabled: true,
            source_language: None,
            target_language: None,
            source_fingerprint: None,
            entries,
        }
    }

    fn entry_with_id(
        id: &str,
        source: &str,
        translation: &str,
        kind: GlossaryKind,
    ) -> GlossaryEntry {
        let mut entry = entry(source, Some(translation), kind, true);
        entry.id = id.parse().unwrap();
        entry
    }

    #[test]
    fn disabled_glossary_has_no_translation_terminology() {
        let glossary = Glossary {
            enabled: false,
            source_language: None,
            target_language: None,
            source_fingerprint: None,
            entries: vec![entry("アリス", Some("Alice"), GlossaryKind::Person, true)],
        };

        assert!(terminology_from_glossary(&glossary).is_empty());
    }

    #[test]
    fn translation_terminology_includes_every_kind_and_filters_ineligible_entries() {
        let glossary = glossary(vec![
            entry("person", Some("Person"), GlossaryKind::Person, true),
            entry("place", Some("Place"), GlossaryKind::Place, true),
            entry(
                "organization",
                Some("Organization"),
                GlossaryKind::Organization,
                true,
            ),
            entry("item", Some("Item"), GlossaryKind::Item, true),
            entry("ability", Some("Ability"), GlossaryKind::Ability, true),
            entry("term", Some("Term"), GlossaryKind::Term, true),
            entry("other", Some("Other"), GlossaryKind::Other, true),
            entry("disabled", Some("Disabled"), GlossaryKind::Term, false),
            entry("untranslated", None, GlossaryKind::Term, true),
        ]);

        assert_eq!(
            terminology_from_glossary(&glossary).as_ref(),
            [
                TerminologyEntry {
                    source: "person".to_owned(),
                    translation: "Person".to_owned(),
                    kind: TerminologyKind::Person,
                },
                TerminologyEntry {
                    source: "place".to_owned(),
                    translation: "Place".to_owned(),
                    kind: TerminologyKind::Place,
                },
                TerminologyEntry {
                    source: "organization".to_owned(),
                    translation: "Organization".to_owned(),
                    kind: TerminologyKind::Organization,
                },
                TerminologyEntry {
                    source: "item".to_owned(),
                    translation: "Item".to_owned(),
                    kind: TerminologyKind::Item,
                },
                TerminologyEntry {
                    source: "ability".to_owned(),
                    translation: "Ability".to_owned(),
                    kind: TerminologyKind::Ability,
                },
                TerminologyEntry {
                    source: "term".to_owned(),
                    translation: "Term".to_owned(),
                    kind: TerminologyKind::Term,
                },
                TerminologyEntry {
                    source: "other".to_owned(),
                    translation: "Other".to_owned(),
                    kind: TerminologyKind::Other,
                },
            ]
        );
    }

    #[test]
    fn translation_terminology_order_is_kind_normalized_source_then_id() {
        let glossary = glossary(vec![
            entry_with_id(
                "00000000-0000-0000-0000-000000000002",
                "Ａlice",
                "Alice Two",
                GlossaryKind::Person,
            ),
            entry_with_id(
                "00000000-0000-0000-0000-000000000004",
                "alpha",
                "Alpha",
                GlossaryKind::Term,
            ),
            entry_with_id(
                "00000000-0000-0000-0000-000000000003",
                "place",
                "Place",
                GlossaryKind::Place,
            ),
            entry_with_id(
                "00000000-0000-0000-0000-000000000001",
                "alice",
                "Alice One",
                GlossaryKind::Person,
            ),
        ]);

        let actual = terminology_from_glossary(&glossary);

        assert_eq!(
            actual
                .iter()
                .map(|entry| (
                    entry.kind,
                    entry.source.as_str(),
                    entry.translation.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                (TerminologyKind::Person, "alice", "Alice One"),
                (TerminologyKind::Person, "Ａlice", "Alice Two"),
                (TerminologyKind::Place, "place", "Place"),
                (TerminologyKind::Term, "alpha", "Alpha"),
            ]
        );
    }

    #[tokio::test]
    async fn request_translation_terminology_is_unchanged_after_scene_edit() {
        let original = glossary(vec![entry(
            "アリス",
            Some("Alice"),
            GlossaryKind::Person,
            true,
        )]);
        let mut session = koharu_scene::Session::memory().await.unwrap();
        let setup = session
            .snapshot()
            .patch(|edit| edit.set_project(&original))
            .unwrap();
        session.commit(setup).await.unwrap();
        let snapshot = session.snapshot();
        let glossary = snapshot.project_component::<Glossary>().unwrap().unwrap();
        let request = Request::default().with_terminology(terminology_from_glossary(&glossary));

        let mut changed = glossary;
        changed.entries[0].translation = Some("Alicia".to_owned());
        let update = session
            .snapshot()
            .patch(|edit| edit.set_project(&changed))
            .unwrap();
        session.commit(update).await.unwrap();

        assert_eq!(request.terminology[0].translation, "Alice");
        assert_eq!(
            session
                .snapshot()
                .project_component::<Glossary>()
                .unwrap()
                .unwrap()
                .entries[0]
                .translation
                .as_deref(),
            Some("Alicia")
        );
    }
}
