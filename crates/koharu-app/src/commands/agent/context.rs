//! What the agent is told about the project on every turn.
//!
//! This is deliberately *not* the UI's view models. The interface needs font
//! families, per-page layer counts, provider configs and typesetting defaults;
//! the agent would pay for all of it in every request of every turn, and none of
//! it is something it can act on. Anything the agent can only learn by asking is
//! behind a tool instead (`list_fonts`, `inspect_pages`).

use anyhow::{Context as _, Result};
use koharu_scene::Snapshot;
use serde_json::{Value, json};

use crate::commands::project::ProjectInfo;

/// Project identity, page index and pipeline selection, in that order: the parts
/// that rarely change come first so the prompt prefix stays identical while a
/// conversation continues, and the revision/active page that move constantly come
/// last.
pub(super) fn build(info: &ProjectInfo, snapshot: &Snapshot, pipeline: &Value) -> Result<Value> {
    Ok(json!({
        "pages": page_index(snapshot)?,
        "pipeline": {
            // Model selection only. `processor` holds a profile per model and is
            // the bulk of the serialized config without informing any decision
            // the agent can make.
            "detection": pipeline.get("detection"),
            "ocr": pipeline.get("ocr"),
            "inpainting": pipeline.get("inpainting"),
            "translation": {
                "target_language": pipeline
                    .get("translation")
                    .and_then(|translation| translation.get("target_language")),
                "instructions": pipeline
                    .get("translation")
                    .and_then(|translation| translation.get("instructions")),
            },
        },
        "project": {
            "name": info.name,
            "revision": info.revision,
            "active_page": info.active_page,
            "can_undo": info.can_undo,
            "can_redo": info.can_redo,
        },
    }))
}

/// The page index the agent selects from: identity and size only. Counting
/// layers means walking every element of every page, which is per-turn cost for
/// a number the agent can get from `inspect_pages` when it needs it.
fn page_index(snapshot: &Snapshot) -> Result<Vec<Value>> {
    snapshot
        .pages()
        .map(|page| {
            let value = page.page()?;
            Ok(json!({
                "id": page.id().to_string(),
                "label": value.label,
                "width": value.width,
                "height": value.height,
            }))
        })
        .collect()
}

/// The configured pipeline, as a JSON value to select fields from.
pub(super) fn pipeline_config() -> Result<Value> {
    let config = koharu_pipeline::PipelineConfig::load()?;
    let config = config.read()?;
    serde_json::to_value(&*config).context("pipeline configuration must serialize")
}

/// The language the configured pipeline translates into: the bulk reviewer's
/// target. Read from the same source the pipeline itself uses, and without the
/// provider credentials the full `Preferences` load would reach into the keyring
/// for.
pub(super) fn target_language() -> Result<String> {
    let config = koharu_pipeline::PipelineConfig::load()?;
    Ok(config.read()?.translation.target_language.to_string())
}

#[cfg(test)]
mod tests {
    use koharu_scene::{At, PageDraft, Revision, Session};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn context_carries_no_ui_only_payload() {
        let mut session = Session::memory().await.unwrap();
        let mut edit = session.snapshot().edit();
        edit.add_page(PageDraft::new("01", 800.0, 1200.0), At::End)
            .unwrap();
        session.commit(edit.finish().unwrap()).await.unwrap();
        let snapshot = session.snapshot();
        let info = ProjectInfo {
            name: "chapter".to_owned(),
            revision: Revision::default(),
            active_page: None,
            can_undo: false,
            can_redo: false,
        };
        let pipeline = json!({
            "detection": "koharu-layout-rfdetr-seg-2xl",
            "ocr": "paddleocr-vl-1.6",
            "inpainting": "lama",
            "translation": { "target_language": "portuguese", "instructions": null },
            "processor": { "per_model_profiles": "bulky and unactionable" },
        });

        let value = build(&info, &snapshot, &pipeline).unwrap();
        let body = value.to_string();
        for absent in [
            "processor",
            "per_model_profiles",
            "available_fonts",
            "layer_count",
            "typesetting",
            "providers",
        ] {
            assert!(!body.contains(absent), "context must not carry {absent}");
        }
        assert_eq!(value["pages"][0]["label"], "01");
        assert_eq!(value["pages"][0]["width"], 800.0);
        assert_eq!(
            value["pipeline"]["translation"]["target_language"],
            "portuguese"
        );
        assert_eq!(value["project"]["name"], "chapter");
        // Stable parts first, the constantly-changing project last, so the
        // prefix before it stays cacheable across turns.
        assert_eq!(
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["pages", "pipeline", "project"]
        );
    }
}
