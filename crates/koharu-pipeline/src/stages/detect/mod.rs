//! Detection: the page's panels, balloons, and blocks in reading order, in
//! place of the ones there, whoever made them. A block gets its source's
//! frame, tilt, and direction, and its mask.

mod layers;
mod model;
mod tree;

use koharu_ml::koharu_layout_highres::{Options, Typography};
use koharu_scene::{Content, Layer, Op, locked_inside};
use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub use self::model::Detector;
use super::{Task, key};
use crate::{PipelineConfig, Stage};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct DetectorConfig {
    pub model: Detector,
    /// Each class's lowest confidence a detection keeps, from 0 to 1, or the
    /// model's own.
    #[serde(deserialize_with = "threshold")]
    #[specta(type = Option<f32>)]
    pub text: Option<f32>,
    #[serde(deserialize_with = "threshold")]
    #[specta(type = Option<f32>)]
    pub onomatopoeia: Option<f32>,
    #[serde(deserialize_with = "threshold")]
    #[specta(type = Option<f32>)]
    pub bubble: Option<f32>,
    #[serde(deserialize_with = "threshold")]
    #[specta(type = Option<f32>)]
    pub panel: Option<f32>,
}

fn threshold<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f32>, D::Error> {
    let threshold = Option::<f32>::deserialize(deserializer)?;
    match threshold {
        Some(value) if !(0.0..=1.0).contains(&value) => Err(D::Error::custom(format!(
            "a detection threshold of {value} isn't between 0 and 1"
        ))),
        _ => Ok(threshold),
    }
}

pub(crate) async fn detect(task: &Task, config: &PipelineConfig) -> anyhow::Result<()> {
    let member = &task.members[0];
    let read = member.clone();
    let scan = tokio::task::spawn_blocking(move || {
        let _span =
            tracing::info_span!("read", stage = %Stage::Detect, page = read.index + 1).entered();
        read.scan()
    })
    .await??;
    let Some((_, scan)) = scan else {
        return Ok(());
    };
    let key = key(Stage::Detect, config).expect("a detector is Koharu's");
    let config = config.detector;
    let options = Options {
        text: config.text,
        onomatopoeia: config.onomatopoeia,
        bubble: config.bubble,
        panel: config.panel,
    };
    let (scan, detected) = task
        .inference(&[member], move |models| {
            let detector = models.get(&key, |device| model::load(config.model, device))?;
            let detected = detector.inference(&scan, &options)?;
            Ok((scan, detected))
        })
        .await?;
    let page = member.page;
    let name = config.model.to_string();
    let additions = tokio::task::spawn_blocking(move || {
        let pixels = scan.to_rgb8();
        let layout = {
            let _span = tracing::info_span!("layout").entered();
            detected.layout(&pixels)
        };
        let _span = tracing::info_span!("typography").entered();
        let typography: Vec<Option<Typography>> = layout
            .lettering
            .par_iter()
            .map(|lettering| lettering.typography(&pixels))
            .collect();
        layers::additions(&layout, &typography, page, &name)
    })
    .await??;
    member
        .apply(Stage::Detect, move |project| {
            let Ok(page) = project.page(page) else {
                return Ok(Vec::new());
            };
            let mut ops = Vec::new();
            removals(&page.layers, &mut ops);
            ops.extend(additions);
            Ok(ops)
        })
        .await
}

/// Removes the page's panels, balloons, and blocks, each with what's inside
/// it, but a locked one, which the user keeps. One holding a locked layer
/// stays, as the scene removes no layer with a lock inside, and the panels,
/// balloons, and blocks in it go.
fn removals(layers: &[Layer], ops: &mut Vec<Op>) {
    for layer in layers.iter().filter(|layer| !layer.locked) {
        match layer.content {
            Content::Panel(_) | Content::Balloon(_) | Content::Block(_)
                if locked_inside(layer).is_none() =>
            {
                ops.push(Op::RemoveLayer {
                    layer: layer.id.into(),
                })
            }
            _ => removals(&layer.children, ops),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context as _, Result};
    use koharu_scene::Edit;
    use serde_json::json;

    use super::*;
    use crate::stages::tests::{block, holder, lock, page};

    #[test]
    fn detecting_again_keeps_a_locked_layer_and_what_holds_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("panel", None, holder("panel")),
                ("balloon", Some("panel"), holder("balloon")),
                ("kept", Some("balloon"), block(20.0)),
                ("beside", Some("balloon"), block(80.0)),
                ("loose", Some("panel"), block(140.0)),
                ("second", None, holder("panel")),
                ("shut", Some("second"), holder("balloon")),
                ("held", Some("shut"), block(200.0)),
                ("third", None, holder("panel")),
                ("plain", Some("third"), block(260.0)),
            ],
        )?;
        let &[
            panel,
            balloon,
            kept,
            beside,
            loose,
            second,
            shut,
            held,
            third,
            plain,
        ] = &ids[..]
        else {
            anyhow::bail!("ten layers");
        };
        lock(&mut session, kept)?;
        lock(&mut session, shut)?;
        let page = session.project().pages().next().context("a page")?.id;
        let mut ops = Vec::new();
        removals(&session.project().page(page)?.layers, &mut ops);
        ops.push(serde_json::from_value(
            json!({"op": "add_layer", "page": page, "index": 0, "layer": block(320.0)}),
        )?);
        session.apply(Edit {
            label: "detect".to_owned(),
            ops,
        })?;
        let project = session.project();
        for stays in [panel, balloon, kept, second, shut, held] {
            assert!(project.layer(stays).is_ok());
        }
        for goes in [beside, loose, third, plain] {
            assert!(project.layer(goes).is_err());
        }
        assert_eq!(project.blocks(page)?.len(), 3);
        Ok(())
    }
}
