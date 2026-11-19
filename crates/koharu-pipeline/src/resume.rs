//! Completion comes from persisted results, including files made before resume existed.

use std::collections::BTreeSet;

use koharu_scene::{Hash, LayerId, Page, PageId, Project, SourceLine};
use serde_json::{Value, json};

use crate::{PipelineConfig, Stage};

pub(crate) fn detection_done(page: &Page) -> bool {
    fn detected(layers: &[koharu_scene::Layer]) -> bool {
        layers.iter().any(|layer| {
            layer
                .attributes
                .get("c2pa")
                .is_some_and(|a| a.contains_key("detection"))
                || detected(&layer.children)
        })
    }
    page.background.is_none() || page.detected.unwrap_or_else(|| detected(&page.layers))
}

pub(crate) fn pending_blocks(
    project: &Project,
    page: PageId,
    stage: Stage,
    config: &PipelineConfig,
    selected: Option<&[LayerId]>,
) -> Vec<LayerId> {
    let Ok(page) = project.page(page) else {
        return Vec::new();
    };
    if page.excluded {
        return Vec::new();
    }
    project
        .blocks(page.id)
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, _)| selected.is_none_or(|selected| selected.contains(id)))
        .filter_map(|(id, block)| {
            let source = block.source.content.text();
            let layer = project.layer(id).ok()?;
            let preserve = || project.keeps_original(&config.glossary, id, &source);
            let pending = match stage {
                Stage::Recognize => {
                    page.background.is_some()
                        && source.trim().is_empty()
                        && !layer
                            .attributes
                            .get("c2pa")
                            .is_some_and(|a| a.contains_key("ocr"))
                }
                Stage::Inpaint if preserve() => {
                    block.translation.is_some() || block.cleanup.visible
                }
                Stage::Inpaint => {
                    page.background.is_some()
                        && project.has_pixels(block.mask.id)
                        && !project.has_pixels(block.cleanup.id)
                }
                Stage::Translate if source.trim().is_empty() => false,
                Stage::Translate if preserve() => {
                    block.translation.is_some() || block.cleanup.visible
                }
                Stage::Translate => {
                    let saved = block.translation.as_ref().is_some_and(|translation| {
                        !translation.content.text().trim().is_empty()
                            && (translation.language == config.translator.target
                                || !layer
                                    .attributes
                                    .get("c2pa")
                                    .is_some_and(|a| a.contains_key("translation")))
                    });
                    let unchanged = layer
                        .attributes
                        .get("koharu")
                        .and_then(|a| a.get("translation_unchanged"))
                        .is_some_and(|mark| {
                            *mark == unchanged_translation(&source, &config.translator.target)
                        });
                    !saved && !unchanged
                }
                _ => false,
            };
            pending.then_some(id)
        })
        .collect()
}

pub(crate) fn unchanged_translation(
    source: &str,
    target: &koharu_scene::LanguageIdentifier,
) -> Value {
    json!({"source": source, "target": target})
}

pub(crate) fn glossary_lines(
    project: &Project,
    pages: &BTreeSet<PageId>,
) -> koharu_scene::Result<Vec<SourceLine>> {
    let mut lines = project.transcript()?;
    lines.retain(|line| {
        pages.contains(&line.page) && project.page(line.page).is_ok_and(|page| !page.excluded)
    });
    Ok(lines)
}

pub(crate) fn glossary_input(
    lines: &[SourceLine],
    target: &koharu_scene::LanguageIdentifier,
) -> Hash {
    let sources: Vec<_> = lines
        .iter()
        .map(|line| (line.page, line.layer, &line.text))
        .collect();
    // Older analyses stopped after extraction and never reviewed kana against the kanji catalog.
    Hash::of(
        &serde_json::to_vec(&("person-readings", target, sources)).expect("OCR input serializes"),
    )
}
