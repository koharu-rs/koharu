//! Detection: the page's panels, balloons, and blocks in reading order, in
//! place of the ones there, whoever made them. A block gets its source's
//! frame, tilt, and direction, and its mask.

mod layers;
mod model;
mod tree;

use std::collections::BTreeMap;

use koharu_ml::koharu_layout_highres::{Layout, Options, Typography};
use koharu_ml::yuzumarker_font_detection::Kernel;
use koharu_scene::{Content, Layer, Op, PageId, Project, locked_inside};
use kurbo::Rect;
use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub use self::model::Detector;
use super::{Task, key};
use crate::{PipelineConfig, Stage, gpu::LoadOutcome};

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

/// What detection records of a layer in its `koharu` namespace: its score,
/// and how likely each kernel is to be its face and its glyph stroke over
/// its size, which placement picks its style by. A block found before
/// kernels were read has neither.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Detection {
    pub(crate) score: f32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) kernels: BTreeMap<Kernel, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stroke: Option<f64>,
}

pub(crate) async fn detect(task: &Task, config: &PipelineConfig) -> anyhow::Result<()> {
    let member = &task.members[0];
    if member.cancel.is_cancelled() {
        return Err(super::Stopped.into());
    }
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
    let run = member.run;
    let loading = key.clone();
    let (scan, detected) = task
        .inference(&[member], move |models| {
            let detected = models.run(
                &loading,
                &[run],
                None,
                |device| async move {
                    Ok(LoadOutcome {
                        model: model::load(config.model, device).await?,
                        teardown: None,
                        full_gpu: true,
                    })
                },
                |(detector, _)| detector.inference(&scan, &options),
            )?;
            Ok((scan, detected))
        })
        .await?;
    let page = member.page_id();
    let name = config.model.to_string();
    let (layout, typography, masks, crops) = tokio::task::spawn_blocking(move || {
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
        let masks = layers::masks(&layout, &typography);
        let crops = layers::crops(&layout, &pixels);
        (layout, typography, masks, crops)
    })
    .await?;
    let letterings = layout.lettering.len();
    let kernels = task
        .inference(&[member], move |models| {
            let (at, crops): (Vec<usize>, Vec<_>) = crops.into_iter().unzip();
            models.run(
                &key,
                &[run],
                None,
                |device| async move {
                    Ok(LoadOutcome {
                        model: model::load(config.model, device).await?,
                        teardown: None,
                        full_gpu: true,
                    })
                },
                |(_, fonts)| {
                    let _span = tracing::info_span!("fonts", crops = crops.len()).entered();
                    // Each lettering's readings, one a crop, averaged.
                    let mut sums: Vec<Option<(layers::Kernels, f32)>> = vec![None; letterings];
                    for (at, read) in at.iter().copied().zip(fonts.inference(&crops)?) {
                        let (sum, count) = sums[at].get_or_insert_with(Default::default);
                        for (kernel, probability) in read {
                            *sum.entry(kernel).or_default() += probability;
                        }
                        *count += 1.0;
                    }
                    Ok(sums
                        .into_iter()
                        .map(|sum| {
                            sum.map(|(sum, count)| {
                                sum.into_iter()
                                    .map(|(kernel, total)| (kernel, total / count))
                                    .collect()
                            })
                        })
                        .collect::<Vec<_>>())
                },
            )
        })
        .await?;
    member
        .apply(Stage::Detect, move |session| {
            let project = session.project();
            ops(&project, page, &layout, &typography, &kernels, masks, &name)
        })
        .await
}

/// The ops putting the layout's parts in place of the page's, but the blocks
/// kept, whose lettering takes no second one.
fn ops(
    project: &Project,
    page: PageId,
    layout: &Layout,
    typography: &[Option<Typography>],
    kernels: &[Option<layers::Kernels>],
    masks: Vec<Option<layers::Grown>>,
    model: &str,
) -> anyhow::Result<Vec<Op>> {
    let Ok(found) = project.page(page) else {
        return Ok(Vec::new());
    };
    let mut ops = Vec::new();
    removals(&found.layers, &mut ops);
    let kept = kept(project, page);
    ops.extend(layers::additions(
        layout, typography, kernels, masks, &kept, page, model,
    )?);
    Ok(ops)
}

/// The boxes of the blocks `removals` leaves on the page: the locked ones and
/// those in a locked layer.
fn kept(project: &Project, page: PageId) -> Vec<Rect> {
    project
        .blocks(page)
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, _)| project.locked_at(*id).is_some())
        .filter_map(|(_, block)| block.source.bounds())
        .collect()
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
    use anyhow::{Context as _, Result, bail};
    use koharu_ml::koharu_layout_highres::{Kind, Lettering, Mask};
    use koharu_scene::{Edit, LayerId, Session};
    use serde_json::json;

    use super::*;

    #[test]
    fn detecting_again_keeps_a_locked_block_and_adds_no_second() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let outline = "M0 0 L200 0 L200 160 L0 160 Z";
        let block = |x: f64, locked: bool| {
            json!({"kind": "block", "locked": locked, "source": {"frame": {"box": {
                "bounds": {"x": x, "y": 20.0, "width": 40.0, "height": 120.0},
            }}}})
        };
        session.apply(Edit {
            label: "test".to_owned(),
            ops: serde_json::from_value(json!([
                {"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 400}},
                {"op": "add_layer", "id": "$panel", "page": "$p", "index": 0,
                    "layer": {"kind": "panel", "outline": outline}},
                {"op": "add_layer", "id": "$balloon", "page": "$p", "parent": "$panel", "index": 0,
                    "layer": {"kind": "balloon", "outline": outline}},
                {"op": "add_layer", "page": "$p", "parent": "$balloon", "index": 0,
                    "layer": block(20.0, true)},
                {"op": "add_layer", "page": "$p", "parent": "$balloon", "index": 1,
                    "layer": block(80.0, false)},
            ]))?,
        })?;
        let blocks = |project: &Project, page| -> Result<Vec<LayerId>> {
            let blocks = project.blocks(page)?;
            Ok(blocks.into_iter().map(|(id, _)| id).collect())
        };
        let project = session.project();
        let page = project.pages().next().context("a page")?.id;
        let &[kept, beside] = &blocks(&project, page)?[..] else {
            bail!("two blocks");
        };
        // Both letterings, found again where they were.
        let lettering = |x: f32| Lettering {
            kind: Kind::Text,
            score: 0.9,
            bbox: [x, 20.0, x + 40.0, 140.0],
            mask: Mask {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                pixels: Vec::new(),
            },
            bubble: None,
            panel: None,
            room: None,
        };
        let layout = Layout {
            width: 400,
            height: 400,
            lettering: vec![lettering(20.0), lettering(80.0)],
            bubbles: Vec::new(),
            panels: Vec::new(),
        };
        let masks = layers::masks(&layout, &[None, None]);
        let ops = ops(
            &project,
            page,
            &layout,
            &[None, None],
            &[None, None],
            masks,
            "test",
        )?;
        session.apply(Edit {
            label: "detect".to_owned(),
            ops,
        })?;
        let found = blocks(&session.project(), page)?;
        assert_eq!(found.len(), 2);
        assert!(found.contains(&kept) && !found.contains(&beside));
        Ok(())
    }
}
