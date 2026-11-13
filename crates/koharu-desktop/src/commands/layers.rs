//! The Layers panel's and the Inspector's edits. Each is one undo step,
//! labelled by a key the interface translates.

use std::sync::Arc;

use anyhow::bail;
use koharu_scene::{Content, LayerId, Op, Project};
use serde_json::{Map, Value, json};
use tauri::State;

use super::{Result, edit};
use crate::app::App;

/// Shows, hides, or fades layers.
#[tauri::command]
#[specta::specta]
pub async fn set_visibility(
    app: State<'_, Arc<App>>,
    layers: Vec<LayerId>,
    visible: Option<bool>,
    opacity: Option<f32>,
) -> Result<()> {
    let mut patch = Map::new();
    if let Some(visible) = visible {
        patch.insert("visible".to_owned(), json!(visible));
    }
    if let Some(opacity) = opacity {
        patch.insert("opacity".to_owned(), json!(opacity));
    }
    let patch = Value::Object(patch);
    let ops = layers
        .into_iter()
        .map(|layer| Op::UpdateLayer {
            layer: layer.into(),
            patch: patch.clone(),
        })
        .collect();
    edit(&app, "set_visibility", ops).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_layers(app: State<'_, Arc<App>>, layers: Vec<LayerId>) -> Result<()> {
    let ops = layers
        .into_iter()
        .map(|layer| Op::RemoveLayer {
            layer: layer.into(),
        })
        .collect();
    edit(&app, "delete_layers", ops).await
}

/// Moves a layer under `parent`, or to the page's top level, at `index`
/// counted after it's taken out.
#[tauri::command]
#[specta::specta]
pub async fn move_layer(
    app: State<'_, Arc<App>>,
    layer: LayerId,
    parent: Option<LayerId>,
    index: usize,
) -> Result<()> {
    let op = Op::MoveLayer {
        layer: layer.into(),
        parent: parent.map(Into::into),
        index,
    };
    edit(&app, "move_layer", vec![op]).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_locked(
    app: State<'_, Arc<App>>,
    layers: Vec<LayerId>,
    locked: bool,
) -> Result<()> {
    let ops = layers
        .into_iter()
        .map(|layer| Op::UpdateLayer {
            layer: layer.into(),
            patch: json!({ "locked": locked }),
        })
        .collect();
    edit(&app, "set_locked", ops).await
}

/// A text layer's text, or a block's translation.
#[tauri::command]
#[specta::specta]
pub async fn set_text(app: State<'_, Arc<App>>, layer: LayerId, text: String) -> Result<()> {
    let op = Op::SetText {
        layer: layer.into(),
        text,
    };
    edit(&app, "set_text", vec![op]).await
}

/// A block's source text, as OCR would have read it.
#[tauri::command]
#[specta::specta]
pub async fn set_source_text(app: State<'_, Arc<App>>, layer: LayerId, text: String) -> Result<()> {
    let op = Op::UpdateLayer {
        layer: layer.into(),
        patch: json!({ "source": { "content": text } }),
    };
    edit(&app, "set_source_text", vec![op]).await
}

/// Merges `style`, a `TextStyle` patch, into each layer's lettering.
#[tauri::command]
#[specta::specta]
pub async fn set_style(app: State<'_, Arc<App>>, layers: Vec<LayerId>, style: Value) -> Result<()> {
    let ops = style_ops(&app.session()?.lock().project(), &layers, &style)?;
    edit(&app, "set_style", ops).await
}

/// A JSON Merge Patch of a layer's values, for the Inspector's frame, fit,
/// anchor, direction, and language.
#[tauri::command]
#[specta::specta]
pub async fn update_layer(app: State<'_, Arc<App>>, layer: LayerId, patch: Value) -> Result<()> {
    let op = Op::UpdateLayer {
        layer: layer.into(),
        patch,
    };
    edit(&app, "update_layer", vec![op]).await
}

/// The ops merging `style` into each layer's lettering: a text's own style,
/// and a block's translation's, which is what it draws.
pub(crate) fn style_ops(
    project: &Project,
    layers: &[LayerId],
    style: &Value,
) -> anyhow::Result<Vec<Op>> {
    layers
        .iter()
        .map(|&layer| {
            let patch = match &project.layer(layer)?.content {
                Content::Text(_) => json!({ "style": style }),
                Content::Block(_) => json!({ "translation": { "style": style } }),
                _ => bail!("layer {layer} has no text to style"),
            };
            Ok(Op::UpdateLayer {
                layer: layer.into(),
                patch,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Content, Created, Edit, Session};
    use serde_json::json;

    use super::*;

    #[test]
    fn a_style_change_on_a_block_patches_its_translation() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session = Session::create(dir.path().join("style.khr"))?;
        let ops = serde_json::from_value(json!([
            {"op": "add_page", "id": "$page", "index": 0, "size": {"width": 100, "height": 100}},
            {"op": "add_layer", "id": "$block", "page": "$page", "index": 0, "layer":
                {"kind": "block", "source": {"content": "やあ"}, "translation": {"content": "Hi"}}},
            {"op": "add_layer", "id": "$text", "page": "$page", "index": 1, "layer":
                {"kind": "text", "content": "Yo"}},
        ]))?;
        let created = session
            .apply(Edit {
                label: "test".to_owned(),
                ops,
            })?
            .created;
        let layer = |label: &str| {
            created.iter().find_map(|created| match created {
                Created::Layer {
                    id,
                    label: Some(name),
                } if name == label => Some(*id),
                _ => None,
            })
        };
        let (Some(block), Some(text)) = (layer("block"), layer("text")) else {
            anyhow::bail!("the layers weren't made");
        };

        let ops = style_ops(
            &session.project(),
            &[block, text],
            &json!({"chars": {"size": 30.0}}),
        )?;
        session.apply(Edit {
            label: "set_style".to_owned(),
            ops,
        })?;

        let project = session.project();
        let Content::Block(block) = &project.layer(block)?.content else {
            anyhow::bail!("not a block");
        };
        let translation = block.translation.as_ref().map(|text| text.style.chars.size);
        assert_eq!(translation, Some(Some(30.0)));
        assert_eq!(block.source.style.chars.size, None);
        let Content::Text(text) = &project.layer(text)?.content else {
            anyhow::bail!("not a text");
        };
        assert_eq!(text.style.chars.size, Some(30.0));
        Ok(())
    }
}
