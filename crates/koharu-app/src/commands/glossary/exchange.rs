use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, bail};
use koharu_scene::{
    Glossary, GlossaryEntry, GlossaryEntryId, GlossaryKind, GlossaryValueOrigin, LanguageTag,
    Revision, Snapshot, normalize_glossary_source,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{GlossaryView, glossary};

const GLOSSARY_FORMAT: &str = "koharu-glossary";
const GLOSSARY_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Type)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlossaryExchange {
    pub format: String,
    pub version: u32,
    pub source_language: Option<LanguageTag>,
    pub target_language: Option<LanguageTag>,
    pub entries: Vec<GlossaryExchangeEntry>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Type)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlossaryExchangeEntry {
    pub source: String,
    pub translation: Option<String>,
    pub kind: GlossaryKind,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GlossaryImportStrategy {
    KeepExisting,
    ReplaceExisting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LanguageField {
    Source,
    Target,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Type)]
pub(crate) struct LanguageMismatch {
    pub field: LanguageField,
    pub current: Option<LanguageTag>,
    pub imported: Option<LanguageTag>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Type)]
pub(crate) struct GlossaryImportPreview {
    pub added: u32,
    pub conflicting: u32,
    pub identical: u32,
    pub language_mismatches: Vec<LanguageMismatch>,
}

impl crate::commands::project::Project {
    pub(crate) fn export_glossary_json(&self) -> Result<String> {
        let glossary = glossary(&self.snapshot())?;
        let exchange = GlossaryExchange {
            format: GLOSSARY_FORMAT.to_owned(),
            version: GLOSSARY_VERSION,
            source_language: glossary.source_language,
            target_language: glossary.target_language,
            entries: glossary
                .entries
                .into_iter()
                .map(|entry| GlossaryExchangeEntry {
                    source: entry.source,
                    translation: entry.translation,
                    kind: entry.kind,
                    enabled: entry.enabled,
                })
                .collect(),
        };
        serde_json::to_string_pretty(&exchange).context("failed to serialize glossary export")
    }

    pub(crate) fn preview_glossary_import(&self, json: &str) -> Result<GlossaryImportPreview> {
        let imported = parse_exchange(json)?;
        let snapshot = self.snapshot();
        validate_exchange(&snapshot, &imported)?;
        Ok(preview(&glossary(&snapshot)?, &imported))
    }

    pub(crate) async fn apply_glossary_import(
        &mut self,
        expected_revision: Revision,
        json: &str,
        strategy: GlossaryImportStrategy,
        confirm_language_mismatch: bool,
    ) -> Result<GlossaryView> {
        if self.revision() != expected_revision {
            bail!(
                "glossary edit expected revision {expected_revision}, but the project is at {}",
                self.revision()
            );
        }
        let imported = parse_exchange(json)?;
        let snapshot = self.snapshot();
        validate_exchange(&snapshot, &imported)?;
        let current = glossary(&snapshot)?;
        let preview = preview(&current, &imported);
        if !preview.language_mismatches.is_empty() && !confirm_language_mismatch {
            bail!("glossary import language mismatch requires confirmation");
        }

        self.mutate_glossary(expected_revision, move |glossary| {
            if glossary.source_language.is_none() {
                glossary.source_language = imported.source_language;
            }
            if glossary.target_language.is_none() {
                glossary.target_language = imported.target_language;
            }

            let mut existing = glossary
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
            for imported in imported.entries {
                let key = (normalize_glossary_source(&imported.source), imported.kind);
                if let Some(&index) = existing.get(&key) {
                    if strategy == GlossaryImportStrategy::ReplaceExisting
                        && !semantic_equal(&glossary.entries[index], &imported)
                    {
                        let entry = &mut glossary.entries[index];
                        entry.source = imported.source;
                        entry.translation_origin = imported
                            .translation
                            .as_ref()
                            .map(|_| GlossaryValueOrigin::Imported);
                        entry.translation = imported.translation;
                        entry.kind = imported.kind;
                        entry.enabled = imported.enabled;
                        entry.source_origin = GlossaryValueOrigin::Imported;
                    }
                    continue;
                }

                let index = glossary.entries.len();
                existing.insert(key, index);
                glossary.entries.push(GlossaryEntry {
                    id: GlossaryEntryId::new(),
                    source: imported.source,
                    translation_origin: imported
                        .translation
                        .as_ref()
                        .map(|_| GlossaryValueOrigin::Imported),
                    translation: imported.translation,
                    kind: imported.kind,
                    enabled: imported.enabled,
                    confidence: None,
                    occurrence_count: 0,
                    examples: Vec::new(),
                    source_origin: GlossaryValueOrigin::Imported,
                    present_in_last_scan: false,
                });
            }
            Ok(())
        })
        .await
    }
}

fn parse_exchange(json: &str) -> Result<GlossaryExchange> {
    let exchange: GlossaryExchange =
        serde_json::from_str(json).context("invalid glossary exchange JSON")?;
    if exchange.format != GLOSSARY_FORMAT {
        bail!("unsupported glossary exchange format: {}", exchange.format);
    }
    if exchange.version != GLOSSARY_VERSION {
        bail!(
            "unsupported glossary exchange version: {}",
            exchange.version
        );
    }

    let mut keys = HashSet::with_capacity(exchange.entries.len());
    for entry in &exchange.entries {
        if !keys.insert((normalize_glossary_source(&entry.source), entry.kind)) {
            bail!("glossary import contains duplicate source and kind combinations");
        }
    }
    Ok(exchange)
}

fn preview(current: &Glossary, imported: &GlossaryExchange) -> GlossaryImportPreview {
    let existing = current
        .entries
        .iter()
        .map(|entry| {
            (
                (normalize_glossary_source(&entry.source), entry.kind),
                entry,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut result = GlossaryImportPreview {
        added: 0,
        conflicting: 0,
        identical: 0,
        language_mismatches: language_mismatches(current, imported),
    };
    for entry in &imported.entries {
        let key = (normalize_glossary_source(&entry.source), entry.kind);
        match existing.get(&key) {
            None => result.added += 1,
            Some(existing) if semantic_equal(existing, entry) => result.identical += 1,
            Some(_) => result.conflicting += 1,
        }
    }
    result
}

fn validate_exchange(snapshot: &Snapshot, imported: &GlossaryExchange) -> Result<()> {
    let validation = Glossary {
        enabled: true,
        source_language: imported.source_language.clone(),
        target_language: imported.target_language.clone(),
        source_fingerprint: None,
        entries: imported
            .entries
            .iter()
            .map(|entry| GlossaryEntry {
                id: GlossaryEntryId::new(),
                source: entry.source.clone(),
                translation: entry.translation.clone(),
                kind: entry.kind,
                enabled: entry.enabled,
                confidence: None,
                occurrence_count: 0,
                examples: Vec::new(),
                source_origin: GlossaryValueOrigin::Imported,
                translation_origin: entry
                    .translation
                    .as_ref()
                    .map(|_| GlossaryValueOrigin::Imported),
                present_in_last_scan: false,
            })
            .collect(),
    };
    snapshot.patch(|edit| edit.set_project(&validation))?;
    Ok(())
}

fn language_mismatches(current: &Glossary, imported: &GlossaryExchange) -> Vec<LanguageMismatch> {
    let mut mismatches = Vec::new();
    if let (Some(current), Some(imported)) = (&current.source_language, &imported.source_language)
        && current != imported
    {
        mismatches.push(LanguageMismatch {
            field: LanguageField::Source,
            current: Some(current.clone()),
            imported: Some(imported.clone()),
        });
    }
    if let (Some(current), Some(imported)) = (&current.target_language, &imported.target_language)
        && current != imported
    {
        mismatches.push(LanguageMismatch {
            field: LanguageField::Target,
            current: Some(current.clone()),
            imported: Some(imported.clone()),
        });
    }
    mismatches
}

fn semantic_equal(existing: &GlossaryEntry, imported: &GlossaryExchangeEntry) -> bool {
    existing.source == imported.source
        && existing.translation == imported.translation
        && existing.kind == imported.kind
        && existing.enabled == imported.enabled
}

#[cfg(test)]
mod tests {
    use koharu_scene::{
        Glossary, GlossaryEntry, GlossaryEntryId, GlossaryKind, GlossaryValueOrigin, LanguageTag,
        Revision, Session,
    };
    use serde_json::json;

    use crate::commands::{
        glossary::{GlossaryImportStrategy, LanguageField},
        project::Project,
    };

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
            confidence: Some(0.8),
            occurrence_count: 4,
            examples: vec!["scan example".to_owned()],
            source_origin: GlossaryValueOrigin::Detected,
            translation_origin: translation.map(|_| GlossaryValueOrigin::Automatic),
            present_in_last_scan: true,
        }
    }

    async fn project_with(glossary: Glossary) -> Project {
        let mut session = Session::memory().await.unwrap();
        let patch = session
            .snapshot()
            .patch(|edit| edit.set_project(&glossary))
            .unwrap();
        session.commit(patch).await.unwrap();
        Project::new(session, "test".to_owned())
    }

    #[tokio::test]
    async fn export_json_v1_contains_only_semantic_fields() {
        let alice = entry("アリス", Some("Alice"), GlossaryKind::Person, false);
        let project = project_with(Glossary {
            enabled: true,
            source_language: Some(LanguageTag::new("ja").unwrap()),
            target_language: Some(LanguageTag::new("en").unwrap()),
            source_fingerprint: Some("fingerprint".to_owned()),
            entries: vec![alice],
        })
        .await;

        let exported: serde_json::Value =
            serde_json::from_str(&project.export_glossary_json().unwrap()).unwrap();
        assert_eq!(
            exported,
            json!({
                "format": "koharu-glossary",
                "version": 1,
                "source_language": "ja",
                "target_language": "en",
                "entries": [{
                    "source": "アリス",
                    "translation": "Alice",
                    "kind": "person",
                    "enabled": false
                }]
            })
        );
    }

    #[tokio::test]
    async fn import_preview_and_strategies_require_language_confirmation() {
        let alice = entry("アリス", Some("Alice"), GlossaryKind::Person, true);
        let identical = entry("東京", Some("Tokyo"), GlossaryKind::Place, true);
        let mut project = project_with(Glossary {
            enabled: true,
            source_language: Some(LanguageTag::new("ja").unwrap()),
            target_language: Some(LanguageTag::new("en").unwrap()),
            source_fingerprint: Some("saved".to_owned()),
            entries: vec![alice.clone(), identical],
        })
        .await;
        let import = json!({
            "format": "koharu-glossary",
            "version": 1,
            "source_language": "zh",
            "target_language": "fr",
            "entries": [
                {
                    "source": "アリス",
                    "translation": "Alicia",
                    "kind": "person",
                    "enabled": false
                },
                {
                    "source": "東京",
                    "translation": "Tokyo",
                    "kind": "place",
                    "enabled": true
                },
                {
                    "source": "ボブ",
                    "translation": "Bob",
                    "kind": "person",
                    "enabled": true
                }
            ]
        })
        .to_string();

        let preview = project.preview_glossary_import(&import).unwrap();
        assert_eq!(preview.added, 1);
        assert_eq!(preview.conflicting, 1);
        assert_eq!(preview.identical, 1);
        assert_eq!(preview.language_mismatches.len(), 2);
        assert_eq!(preview.language_mismatches[0].field, LanguageField::Source);
        assert_eq!(preview.language_mismatches[1].field, LanguageField::Target);

        let expected = project.revision();
        let error = project
            .apply_glossary_import(
                expected,
                &import,
                GlossaryImportStrategy::KeepExisting,
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "glossary import language mismatch requires confirmation"
        );
        assert_eq!(project.revision(), expected);
        assert!(project.undo.is_empty());

        let kept = project
            .apply_glossary_import(
                expected,
                &import,
                GlossaryImportStrategy::KeepExisting,
                true,
            )
            .await
            .unwrap();
        assert_eq!(kept.revision, Revision::new(expected.get() + 1));
        assert_eq!(project.undo.len(), 1);
        assert_eq!(kept.source_language.as_ref().unwrap().as_str(), "ja");
        assert_eq!(kept.target_language.as_ref().unwrap().as_str(), "en");
        let kept_alice = kept
            .entries
            .iter()
            .find(|entry| entry.id == alice.id)
            .unwrap();
        assert_eq!(kept_alice.translation.as_deref(), Some("Alice"));
        assert!(kept_alice.enabled);
        let bob = kept
            .entries
            .iter()
            .find(|entry| entry.source == "ボブ")
            .unwrap();
        assert_eq!(bob.source_origin, GlossaryValueOrigin::Imported);
        assert_eq!(bob.translation_origin, Some(GlossaryValueOrigin::Imported));
        assert_eq!(bob.confidence, None);
        assert_eq!(bob.occurrence_count, 0);
        assert!(bob.examples.is_empty());
        assert!(!bob.present_in_last_scan);

        let replaced = project
            .apply_glossary_import(
                kept.revision,
                &import,
                GlossaryImportStrategy::ReplaceExisting,
                true,
            )
            .await
            .unwrap();
        assert_eq!(project.undo.len(), 2);
        let replaced_alice = replaced
            .entries
            .iter()
            .find(|entry| entry.id == alice.id)
            .unwrap();
        assert_eq!(replaced_alice.translation.as_deref(), Some("Alicia"));
        assert!(!replaced_alice.enabled);
        assert_eq!(replaced_alice.source_origin, GlossaryValueOrigin::Imported);
        assert_eq!(
            replaced_alice.translation_origin,
            Some(GlossaryValueOrigin::Imported)
        );
        assert_eq!(replaced_alice.confidence, alice.confidence);
        assert_eq!(replaced_alice.occurrence_count, alice.occurrence_count);
        assert_eq!(replaced_alice.examples, alice.examples);
    }
}
