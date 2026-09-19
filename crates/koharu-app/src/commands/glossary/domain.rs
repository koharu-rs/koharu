use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use koharu_scene::{
    EntityId, GLOSSARY_MAX_EXAMPLES, Glossary, GlossaryEntry, GlossaryEntryId, GlossaryKind,
    GlossaryValueOrigin, LanguageTag, Revision, Snapshot, SourceText, normalize_glossary_source,
};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize, Type)]
pub(crate) struct OcrSourceRecord {
    #[specta(type = f64)]
    pub page_order: usize,
    pub content: EntityId,
    pub source_language: Option<String>,
    pub source_text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub(crate) struct GlossaryView {
    pub revision: Revision,
    pub enabled: bool,
    pub stale: bool,
    pub source_language: Option<LanguageTag>,
    pub target_language: Option<LanguageTag>,
    pub saved_source_fingerprint: Option<String>,
    pub current_source_fingerprint: String,
    pub entries: Vec<GlossaryEntryView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub(crate) struct GlossaryEntryView {
    pub revision: Revision,
    pub id: GlossaryEntryId,
    pub source: String,
    pub translation: Option<String>,
    pub kind: GlossaryKind,
    pub enabled: bool,
    pub confidence: Option<f32>,
    pub occurrence_count: u32,
    pub examples: Vec<String>,
    pub source_origin: GlossaryValueOrigin,
    pub translation_origin: Option<GlossaryValueOrigin>,
    pub present_in_last_scan: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub(crate) struct GlossaryEntryInput {
    pub source: String,
    pub translation: Option<String>,
    pub kind: GlossaryKind,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub(crate) struct ScanCandidate {
    pub source: String,
    pub kind: GlossaryKind,
    pub confidence: f32,
    pub example: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub(crate) struct TermTranslationResult {
    pub id: GlossaryEntryId,
    pub source: String,
    pub translation: String,
}

pub(crate) fn ocr_fingerprint(records: impl IntoIterator<Item = OcrSourceRecord>) -> String {
    let mut records = records.into_iter().collect::<Vec<_>>();
    records.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"koharu-ocr-source-fingerprint-v1\0");
    for record in records {
        hasher.update(&(record.page_order as u64).to_le_bytes());
        hasher.update(record.content.as_uuid().as_bytes());
        hash_optional_string(&mut hasher, record.source_language.as_deref());
        hash_string(&mut hasher, &record.source_text);
    }
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn ocr_source_fingerprint(snapshot: &Snapshot) -> Result<String> {
    let mut records = Vec::new();
    for (page_order, page) in snapshot.pages().enumerate() {
        for entity in snapshot.subtree(page.id())? {
            let Some(source) = snapshot.component::<SourceText>(entity.id())? else {
                continue;
            };
            records.push(OcrSourceRecord {
                page_order,
                content: entity.id(),
                source_language: source.language.map(|language| language.to_string()),
                source_text: source.text.value,
            });
        }
    }
    Ok(ocr_fingerprint(records))
}

pub(super) fn glossary(snapshot: &Snapshot) -> Result<Glossary> {
    Ok(snapshot
        .project_component::<Glossary>()?
        .unwrap_or(Glossary {
            enabled: false,
            source_language: None,
            target_language: None,
            source_fingerprint: None,
            entries: Vec::new(),
        }))
}

fn glossary_view(snapshot: &Snapshot) -> Result<GlossaryView> {
    let revision = snapshot.revision();
    let current_source_fingerprint = ocr_source_fingerprint(snapshot)?;
    let glossary = glossary(snapshot)?;
    let stale = glossary.source_fingerprint.as_deref() != Some(&current_source_fingerprint);
    Ok(GlossaryView {
        revision,
        enabled: glossary.enabled,
        stale,
        source_language: glossary.source_language,
        target_language: glossary.target_language,
        saved_source_fingerprint: glossary.source_fingerprint,
        current_source_fingerprint,
        entries: glossary
            .entries
            .into_iter()
            .map(|entry| GlossaryEntryView {
                revision,
                id: entry.id,
                source: entry.source,
                translation: entry.translation,
                kind: entry.kind,
                enabled: entry.enabled,
                confidence: entry.confidence,
                occurrence_count: entry.occurrence_count,
                examples: entry.examples,
                source_origin: entry.source_origin,
                translation_origin: entry.translation_origin,
                present_in_last_scan: entry.present_in_last_scan,
            })
            .collect(),
    })
}

impl crate::commands::project::Project {
    pub(crate) fn glossary_view(&self) -> Result<GlossaryView> {
        glossary_view(&self.snapshot())
    }

    pub(crate) async fn set_glossary_enabled(
        &mut self,
        expected_revision: Revision,
        enabled: bool,
    ) -> Result<GlossaryView> {
        self.mutate_glossary(expected_revision, |glossary| {
            glossary.enabled = enabled;
            Ok(())
        })
        .await
    }

    pub(crate) async fn add_glossary_entry(
        &mut self,
        expected_revision: Revision,
        input: GlossaryEntryInput,
    ) -> Result<GlossaryView> {
        self.mutate_glossary(expected_revision, |glossary| {
            glossary.entries.push(GlossaryEntry {
                id: GlossaryEntryId::new(),
                source: input.source,
                translation_origin: input
                    .translation
                    .as_ref()
                    .map(|_| GlossaryValueOrigin::User),
                translation: input.translation,
                kind: input.kind,
                enabled: input.enabled,
                confidence: None,
                occurrence_count: 0,
                examples: Vec::new(),
                source_origin: GlossaryValueOrigin::User,
                present_in_last_scan: false,
            });
            Ok(())
        })
        .await
    }

    pub(crate) async fn update_glossary_entry(
        &mut self,
        expected_revision: Revision,
        id: GlossaryEntryId,
        input: GlossaryEntryInput,
    ) -> Result<GlossaryView> {
        self.mutate_glossary(expected_revision, |glossary| {
            let entry = glossary
                .entries
                .iter_mut()
                .find(|entry| entry.id == id)
                .context("glossary entry was not found")?;
            entry.source = input.source;
            entry.translation_origin = input
                .translation
                .as_ref()
                .map(|_| GlossaryValueOrigin::User);
            entry.translation = input.translation;
            entry.kind = input.kind;
            entry.enabled = input.enabled;
            entry.source_origin = GlossaryValueOrigin::User;
            Ok(())
        })
        .await
    }

    pub(crate) async fn delete_glossary_entry(
        &mut self,
        expected_revision: Revision,
        id: GlossaryEntryId,
    ) -> Result<GlossaryView> {
        self.mutate_glossary(expected_revision, |glossary| {
            let before = glossary.entries.len();
            glossary.entries.retain(|entry| entry.id != id);
            if glossary.entries.len() == before {
                bail!("glossary entry was not found");
            }
            Ok(())
        })
        .await
    }

    pub(crate) async fn apply_glossary_scan(
        &mut self,
        expected_revision: Revision,
        source_language: Option<LanguageTag>,
        target_language: Option<LanguageTag>,
        candidates: Vec<ScanCandidate>,
    ) -> Result<GlossaryView> {
        #[derive(Debug)]
        struct Aggregate {
            source: String,
            kind: GlossaryKind,
            confidence: f32,
            occurrence_count: u32,
            examples: Vec<String>,
        }

        let source_fingerprint = ocr_source_fingerprint(&self.snapshot())?;
        let mut positions = HashMap::<(String, GlossaryKind), usize>::new();
        let mut aggregates = Vec::<Aggregate>::new();
        for candidate in candidates {
            let key = (normalize_glossary_source(&candidate.source), candidate.kind);
            if let Some(&index) = positions.get(&key) {
                let aggregate = &mut aggregates[index];
                if !candidate.confidence.is_finite()
                    || !(0.0..=1.0).contains(&candidate.confidence)
                    || candidate.confidence > aggregate.confidence
                {
                    aggregate.confidence = candidate.confidence;
                }
                aggregate.occurrence_count = aggregate.occurrence_count.saturating_add(1);
                if let Some(example) = candidate.example
                    && aggregate.examples.len() < GLOSSARY_MAX_EXAMPLES
                    && !aggregate.examples.contains(&example)
                {
                    aggregate.examples.push(example);
                }
            } else {
                let mut examples = Vec::new();
                if let Some(example) = candidate.example {
                    examples.push(example);
                }
                positions.insert(key, aggregates.len());
                aggregates.push(Aggregate {
                    source: candidate.source,
                    kind: candidate.kind,
                    confidence: candidate.confidence,
                    occurrence_count: 1,
                    examples,
                });
            }
        }

        self.mutate_glossary(expected_revision, |glossary| {
            glossary.source_language = source_language;
            glossary.target_language = target_language;
            glossary.source_fingerprint = Some(source_fingerprint);
            for entry in &mut glossary.entries {
                entry.present_in_last_scan = false;
            }

            let existing = glossary
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    (
                        (normalize_glossary_source(&entry.source), entry.kind),
                        index,
                    )
                })
                .collect::<HashMap<_, _>>();
            for aggregate in aggregates {
                let key = (normalize_glossary_source(&aggregate.source), aggregate.kind);
                if let Some(&index) = existing.get(&key) {
                    let entry = &mut glossary.entries[index];
                    if !matches!(
                        entry.source_origin,
                        GlossaryValueOrigin::User | GlossaryValueOrigin::Imported
                    ) {
                        entry.source = aggregate.source;
                    }
                    entry.confidence = Some(aggregate.confidence);
                    entry.occurrence_count = aggregate.occurrence_count;
                    entry.examples = aggregate.examples;
                    entry.present_in_last_scan = true;
                } else {
                    glossary.entries.push(GlossaryEntry {
                        id: GlossaryEntryId::new(),
                        source: aggregate.source,
                        translation: None,
                        kind: aggregate.kind,
                        enabled: true,
                        confidence: Some(aggregate.confidence),
                        occurrence_count: aggregate.occurrence_count,
                        examples: aggregate.examples,
                        source_origin: GlossaryValueOrigin::Detected,
                        translation_origin: None,
                        present_in_last_scan: true,
                    });
                }
            }
            Ok(())
        })
        .await
    }

    pub(crate) async fn apply_term_translations(
        &mut self,
        expected_revision: Revision,
        translations: Vec<TermTranslationResult>,
    ) -> Result<GlossaryView> {
        let translations = translations
            .into_iter()
            .map(|translation| (translation.id, translation))
            .collect::<HashMap<_, _>>();
        self.mutate_glossary(expected_revision, |glossary| {
            for entry in &mut glossary.entries {
                let Some(result) = translations.get(&entry.id) else {
                    continue;
                };
                if entry.source != result.source
                    || !matches!(
                        entry.translation_origin,
                        None | Some(GlossaryValueOrigin::Automatic)
                    )
                {
                    continue;
                }
                entry.translation = Some(result.translation.clone());
                entry.translation_origin = Some(GlossaryValueOrigin::Automatic);
            }
            Ok(())
        })
        .await
    }

    pub(super) async fn mutate_glossary(
        &mut self,
        expected_revision: Revision,
        mutate: impl FnOnce(&mut Glossary) -> Result<()>,
    ) -> Result<GlossaryView> {
        let snapshot = self.snapshot();
        if snapshot.revision() != expected_revision {
            bail!(
                "glossary edit expected revision {expected_revision}, but the project is at {}",
                snapshot.revision()
            );
        }
        let mut value = glossary(&snapshot)?;
        mutate(&mut value)?;
        let patch = snapshot.patch(|edit| edit.set_project(&value))?;
        let commit = self.session.commit(patch).await?;
        self.record_commit(&commit);
        glossary_view(&commit.snapshot)
    }
}

fn hash_optional_string(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hash_string(hasher, value);
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn hash_string(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use koharu_scene::{
        At, Authored, EntityId, LanguageTag, Origin, PageDraft, RemovePolicy, Session, SourceText,
        TextLayout, TextLayoutKind, Translation, Visibility,
    };

    use super::{OcrSourceRecord, ocr_fingerprint, ocr_source_fingerprint};
    use crate::commands::{
        glossary::{GlossaryEntryInput, GlossaryView, ScanCandidate, TermTranslationResult},
        project::Project,
    };
    use koharu_scene::{GlossaryKind, GlossaryValueOrigin, Revision};

    fn record(
        page_order: usize,
        content: EntityId,
        language: Option<&str>,
        text: &str,
    ) -> OcrSourceRecord {
        OcrSourceRecord {
            page_order,
            content,
            source_language: language.map(str::to_owned),
            source_text: text.to_owned(),
        }
    }

    #[test]
    fn fingerprint_is_deterministic_and_order_independent() {
        let first = EntityId::new();
        let second = EntityId::new();
        let records = vec![
            record(1, second, Some("ja"), "二"),
            record(0, first, Some("ja"), "一"),
        ];
        let reversed = records.iter().cloned().rev().collect::<Vec<_>>();

        let fingerprint = ocr_fingerprint(records);
        assert_eq!(fingerprint, ocr_fingerprint(reversed));
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(fingerprint, fingerprint.to_ascii_lowercase());
    }

    #[tokio::test]
    async fn scene_fingerprint_tracks_only_ordered_ocr_sources() {
        let mut session = Session::memory().await.unwrap();
        let empty = ocr_source_fingerprint(&session.snapshot()).unwrap();
        assert_eq!(empty, ocr_source_fingerprint(&session.snapshot()).unwrap());

        let mut ids = None;
        let setup = session
            .snapshot()
            .patch(|edit| {
                let first_page = edit.add_page(PageDraft::new("one", 100.0, 100.0), At::End)?;
                let first = edit.add_text_content(first_page, At::End)?;
                edit.set(
                    first,
                    &SourceText {
                        text: Authored::user("一".to_owned()),
                        language: Some(LanguageTag::new("ja")?),
                    },
                )?;
                let second_page = edit.add_page(PageDraft::new("two", 100.0, 100.0), At::End)?;
                let second = edit.add_text_content(second_page, At::End)?;
                edit.set(
                    second,
                    &SourceText {
                        text: Authored::user("二".to_owned()),
                        language: Some(LanguageTag::new("ja")?),
                    },
                )?;
                ids = Some((first_page, first, second_page, second));
                Ok(())
            })
            .unwrap();
        session.commit(setup).await.unwrap();
        let (first_page, first, second_page, _) = ids.unwrap();
        let baseline = ocr_source_fingerprint(&session.snapshot()).unwrap();
        assert_ne!(baseline, empty);

        let presentation_only = session
            .snapshot()
            .patch(|edit| {
                edit.set(
                    first,
                    &Translation {
                        text: Authored::user("one".to_owned()),
                        language: Some(LanguageTag::new("en")?),
                    },
                )?;
                let layer = edit.add_text_layer(
                    first_page,
                    At::End,
                    first,
                    &TextLayout {
                        origin: Origin::User,
                        kind: TextLayoutKind::Paragraph,
                        angle_degrees: None,
                    },
                )?;
                edit.set(
                    layer,
                    &Visibility {
                        origin: Origin::User,
                        visible: false,
                        opacity: 0.5,
                    },
                )
            })
            .unwrap();
        session.commit(presentation_only).await.unwrap();
        assert_eq!(
            ocr_source_fingerprint(&session.snapshot()).unwrap(),
            baseline
        );

        let change_text = session
            .snapshot()
            .patch(|edit| {
                edit.set(
                    first,
                    &SourceText {
                        text: Authored::user(" changed ".to_owned()),
                        language: Some(LanguageTag::new("ja")?),
                    },
                )
            })
            .unwrap();
        session.commit(change_text).await.unwrap();
        assert_ne!(
            ocr_source_fingerprint(&session.snapshot()).unwrap(),
            baseline
        );

        let change_language = session
            .snapshot()
            .patch(|edit| {
                edit.set(
                    first,
                    &SourceText {
                        text: Authored::user("一".to_owned()),
                        language: Some(LanguageTag::new("zh")?),
                    },
                )
            })
            .unwrap();
        session.commit(change_language).await.unwrap();
        assert_ne!(
            ocr_source_fingerprint(&session.snapshot()).unwrap(),
            baseline
        );

        let mut added = None;
        let add_source = session
            .snapshot()
            .patch(|edit| {
                edit.set(
                    first,
                    &SourceText {
                        text: Authored::user("一".to_owned()),
                        language: Some(LanguageTag::new("ja")?),
                    },
                )?;
                let content = edit.add_text_content(first_page, At::End)?;
                edit.set(
                    content,
                    &SourceText {
                        text: Authored::user("三".to_owned()),
                        language: Some(LanguageTag::new("ja")?),
                    },
                )?;
                added = Some(content);
                Ok(())
            })
            .unwrap();
        session.commit(add_source).await.unwrap();
        assert_ne!(
            ocr_source_fingerprint(&session.snapshot()).unwrap(),
            baseline
        );

        let remove_source = session
            .snapshot()
            .patch(|edit| edit.remove_entity(added.unwrap(), RemovePolicy::Cascade))
            .unwrap();
        session.commit(remove_source).await.unwrap();
        assert_eq!(
            ocr_source_fingerprint(&session.snapshot()).unwrap(),
            baseline
        );

        let reorder = session
            .snapshot()
            .patch(|edit| edit.move_entity(second_page, None, At::Start))
            .unwrap();
        session.commit(reorder).await.unwrap();
        assert_ne!(
            ocr_source_fingerprint(&session.snapshot()).unwrap(),
            baseline
        );
    }

    fn entry(source: &str, translation: Option<&str>) -> GlossaryEntryInput {
        GlossaryEntryInput {
            source: source.to_owned(),
            translation: translation.map(str::to_owned),
            kind: GlossaryKind::Person,
            enabled: true,
        }
    }

    fn assert_view_revision(view: &GlossaryView, revision: Revision) {
        assert_eq!(view.revision, revision);
        assert!(view.entries.iter().all(|entry| entry.revision == revision));
    }

    #[tokio::test]
    async fn project_glossary_mutations_are_revision_guarded_and_record_one_commit() {
        let mut project = Project::new(Session::memory().await.unwrap(), "test".to_owned());
        let initial = project.glossary_view().unwrap();
        assert_view_revision(&initial, Revision::ZERO);
        assert!(!initial.enabled);
        assert!(initial.stale);
        assert_eq!(initial.source_language, None);
        assert_eq!(initial.target_language, None);
        assert_eq!(initial.saved_source_fingerprint, None);
        assert_eq!(initial.entries, Vec::new());

        let added = project
            .add_glossary_entry(Revision::ZERO, entry("アリス", Some("Alice")))
            .await
            .unwrap();
        assert_view_revision(&added, Revision::new(1));
        assert_eq!(project.undo.len(), 1);
        assert_eq!(added.entries.len(), 1);
        assert_eq!(added.entries[0].source_origin, GlossaryValueOrigin::User);
        assert_eq!(
            added.entries[0].translation_origin,
            Some(GlossaryValueOrigin::User)
        );
        let id = added.entries[0].id;

        let error = project
            .set_glossary_enabled(Revision::ZERO, true)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "glossary edit expected revision 0, but the project is at 1"
        );
        assert_eq!(project.revision(), Revision::new(1));
        assert_eq!(project.undo.len(), 1);

        let enabled = project
            .set_glossary_enabled(Revision::new(1), true)
            .await
            .unwrap();
        assert_view_revision(&enabled, Revision::new(2));
        assert!(enabled.enabled);
        assert_eq!(project.undo.len(), 2);

        let updated = project
            .update_glossary_entry(
                Revision::new(2),
                id,
                GlossaryEntryInput {
                    source: "アリシア".to_owned(),
                    translation: None,
                    kind: GlossaryKind::Term,
                    enabled: false,
                },
            )
            .await
            .unwrap();
        assert_view_revision(&updated, Revision::new(3));
        assert_eq!(updated.entries[0].source, "アリシア");
        assert_eq!(updated.entries[0].translation, None);
        assert_eq!(updated.entries[0].kind, GlossaryKind::Term);
        assert!(!updated.entries[0].enabled);
        assert_eq!(project.undo.len(), 3);

        let invalid_revision = project.revision();
        assert!(
            project
                .add_glossary_entry(invalid_revision, entry(" \t", None))
                .await
                .is_err()
        );
        assert_eq!(project.revision(), invalid_revision);
        assert_eq!(project.undo.len(), 3);

        let deleted = project
            .delete_glossary_entry(Revision::new(3), id)
            .await
            .unwrap();
        assert_view_revision(&deleted, Revision::new(4));
        assert!(deleted.entries.is_empty());
        assert_eq!(project.undo.len(), 4);
    }

    fn stored_entry(
        source: &str,
        translation: Option<&str>,
        kind: GlossaryKind,
        enabled: bool,
        source_origin: GlossaryValueOrigin,
    ) -> koharu_scene::GlossaryEntry {
        koharu_scene::GlossaryEntry {
            id: koharu_scene::GlossaryEntryId::new(),
            source: source.to_owned(),
            translation: translation.map(str::to_owned),
            kind,
            enabled,
            confidence: Some(0.25),
            occurrence_count: 7,
            examples: vec!["old example".to_owned()],
            source_origin,
            translation_origin: translation.map(|_| source_origin),
            present_in_last_scan: true,
        }
    }

    #[tokio::test]
    async fn scan_merge_aggregates_and_preserves_existing_semantics() {
        let manual = stored_entry(
            " Ａlice ",
            Some("Alicia"),
            GlossaryKind::Person,
            false,
            GlossaryValueOrigin::User,
        );
        let imported = stored_entry(
            "Magic  Sword",
            Some("魔法剣"),
            GlossaryKind::Item,
            false,
            GlossaryValueOrigin::Imported,
        );
        let absent = stored_entry(
            "Old",
            None,
            GlossaryKind::Term,
            true,
            GlossaryValueOrigin::Detected,
        );
        let mut session = Session::memory().await.unwrap();
        let setup = session
            .snapshot()
            .patch(|edit| {
                edit.set_project(&koharu_scene::Glossary {
                    enabled: true,
                    source_language: None,
                    target_language: None,
                    source_fingerprint: Some("old".to_owned()),
                    entries: vec![manual.clone(), imported.clone(), absent.clone()],
                })
            })
            .unwrap();
        session.commit(setup).await.unwrap();
        let mut project = Project::new(session, "test".to_owned());
        let expected = project.revision();

        let view = project
            .apply_glossary_scan(
                expected,
                Some(LanguageTag::new("ja").unwrap()),
                Some(LanguageTag::new("en").unwrap()),
                vec![
                    ScanCandidate {
                        source: "alice".to_owned(),
                        kind: GlossaryKind::Person,
                        confidence: 0.6,
                        example: Some("first".to_owned()),
                    },
                    ScanCandidate {
                        source: "ＡLICE".to_owned(),
                        kind: GlossaryKind::Person,
                        confidence: 0.9,
                        example: Some("second".to_owned()),
                    },
                    ScanCandidate {
                        source: "alice".to_owned(),
                        kind: GlossaryKind::Person,
                        confidence: 0.7,
                        example: Some("first".to_owned()),
                    },
                    ScanCandidate {
                        source: "ＭＡＧＩＣ sword".to_owned(),
                        kind: GlossaryKind::Item,
                        confidence: 0.8,
                        example: None,
                    },
                    ScanCandidate {
                        source: "ボブ".to_owned(),
                        kind: GlossaryKind::Person,
                        confidence: 0.75,
                        example: Some("ボブさん".to_owned()),
                    },
                ],
            )
            .await
            .unwrap();

        assert_view_revision(&view, Revision::new(expected.get() + 1));
        assert!(view.enabled);
        assert!(!view.stale);
        assert_eq!(
            view.saved_source_fingerprint,
            Some(view.current_source_fingerprint.clone())
        );
        assert_eq!(view.source_language.unwrap().as_str(), "ja");
        assert_eq!(view.target_language.unwrap().as_str(), "en");
        assert_eq!(project.undo.len(), 1);
        assert_eq!(view.entries.len(), 4);

        let manual_view = view
            .entries
            .iter()
            .find(|entry| entry.id == manual.id)
            .unwrap();
        assert_eq!(manual_view.source, manual.source);
        assert_eq!(manual_view.kind, manual.kind);
        assert_eq!(manual_view.enabled, manual.enabled);
        assert_eq!(manual_view.translation, manual.translation);
        assert_eq!(manual_view.source_origin, GlossaryValueOrigin::User);
        assert_eq!(manual_view.confidence, Some(0.9));
        assert_eq!(manual_view.occurrence_count, 3);
        assert_eq!(manual_view.examples, ["first", "second"]);
        assert!(manual_view.present_in_last_scan);

        let imported_view = view
            .entries
            .iter()
            .find(|entry| entry.id == imported.id)
            .unwrap();
        assert_eq!(imported_view.source, imported.source);
        assert_eq!(imported_view.enabled, imported.enabled);
        assert_eq!(imported_view.translation, imported.translation);
        assert_eq!(imported_view.source_origin, GlossaryValueOrigin::Imported);
        assert_eq!(imported_view.occurrence_count, 1);
        assert!(imported_view.present_in_last_scan);

        let absent_view = view
            .entries
            .iter()
            .find(|entry| entry.id == absent.id)
            .unwrap();
        assert_eq!(absent_view.confidence, absent.confidence);
        assert_eq!(absent_view.occurrence_count, absent.occurrence_count);
        assert_eq!(absent_view.examples, absent.examples);
        assert!(!absent_view.present_in_last_scan);

        let added = view
            .entries
            .iter()
            .find(|entry| entry.source == "ボブ")
            .unwrap();
        assert!(added.enabled);
        assert_eq!(added.translation, None);
        assert_eq!(added.translation_origin, None);
        assert_eq!(added.source_origin, GlossaryValueOrigin::Detected);
        assert_eq!(added.confidence, Some(0.75));
        assert_eq!(added.occurrence_count, 1);
        assert_eq!(added.examples, ["ボブさん"]);
        assert!(added.present_in_last_scan);
    }

    #[tokio::test]
    async fn translation_results_preserve_concurrent_entry_edits() {
        let mut automatic = stored_entry(
            "アリス",
            Some("old automatic"),
            GlossaryKind::Person,
            true,
            GlossaryValueOrigin::Detected,
        );
        automatic.translation_origin = Some(GlossaryValueOrigin::Automatic);
        let mut user = stored_entry(
            "ボブ",
            Some("custom Bob"),
            GlossaryKind::Person,
            true,
            GlossaryValueOrigin::User,
        );
        user.translation_origin = Some(GlossaryValueOrigin::User);
        let changed = stored_entry(
            "変更後",
            None,
            GlossaryKind::Term,
            true,
            GlossaryValueOrigin::User,
        );
        let mut imported = stored_entry(
            "剣",
            Some("Blade"),
            GlossaryKind::Item,
            true,
            GlossaryValueOrigin::Imported,
        );
        imported.translation_origin = Some(GlossaryValueOrigin::Imported);

        let mut session = Session::memory().await.unwrap();
        let setup = session
            .snapshot()
            .patch(|edit| {
                edit.set_project(&koharu_scene::Glossary {
                    enabled: true,
                    source_language: Some(LanguageTag::new("ja")?),
                    target_language: Some(LanguageTag::new("en")?),
                    source_fingerprint: None,
                    entries: vec![
                        automatic.clone(),
                        user.clone(),
                        changed.clone(),
                        imported.clone(),
                    ],
                })
            })
            .unwrap();
        session.commit(setup).await.unwrap();
        let mut project = Project::new(session, "test".to_owned());
        let expected = project.revision();

        let view = project
            .apply_term_translations(
                expected,
                vec![
                    TermTranslationResult {
                        id: automatic.id,
                        source: automatic.source.clone(),
                        translation: "Alice".to_owned(),
                    },
                    TermTranslationResult {
                        id: user.id,
                        source: user.source.clone(),
                        translation: "Bob".to_owned(),
                    },
                    TermTranslationResult {
                        id: changed.id,
                        source: "変更前".to_owned(),
                        translation: "Changed".to_owned(),
                    },
                    TermTranslationResult {
                        id: imported.id,
                        source: imported.source.clone(),
                        translation: "Sword".to_owned(),
                    },
                    TermTranslationResult {
                        id: koharu_scene::GlossaryEntryId::new(),
                        source: "削除済み".to_owned(),
                        translation: "Deleted".to_owned(),
                    },
                ],
            )
            .await
            .unwrap();

        assert_eq!(view.revision, Revision::new(expected.get() + 1));
        assert_eq!(project.undo.len(), 1);
        let automatic_view = view
            .entries
            .iter()
            .find(|entry| entry.id == automatic.id)
            .unwrap();
        assert_eq!(automatic_view.translation.as_deref(), Some("Alice"));
        assert_eq!(
            automatic_view.translation_origin,
            Some(GlossaryValueOrigin::Automatic)
        );
        let user_view = view
            .entries
            .iter()
            .find(|entry| entry.id == user.id)
            .unwrap();
        assert_eq!(user_view.translation, user.translation);
        assert_eq!(user_view.translation_origin, user.translation_origin);
        let changed_view = view
            .entries
            .iter()
            .find(|entry| entry.id == changed.id)
            .unwrap();
        assert_eq!(changed_view.translation, None);
        let imported_view = view
            .entries
            .iter()
            .find(|entry| entry.id == imported.id)
            .unwrap();
        assert_eq!(imported_view.translation, imported.translation);
        assert_eq!(
            imported_view.translation_origin,
            imported.translation_origin
        );
    }
}
