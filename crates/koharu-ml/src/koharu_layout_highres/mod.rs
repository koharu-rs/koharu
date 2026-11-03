//! High-resolution manga layout: RF-DETR Seg 2XL instance segmentation of a
//! page's text, onomatopoeia, speech bubbles, and panels.
//!
//! Checkpoint:
//! https://huggingface.co/mayocream/koharu-layout-highres/tree/53fef6f1301f4cbd8324ddc24f554b3767fc9e45
//! Upstream:
//! https://github.com/roboflow/rf-detr/tree/4ab7c18729de9d02ffd0495795d0831b5630f01b

mod config;
mod layout;
mod measure;
mod model;
mod order;
mod processor;
mod rooms;
mod suppress;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_runtime::Repository;
use koharu_torch::Device;

pub use self::{
    layout::{Bubble, Kind, Layout, Lettering, Panel},
    measure::Typography,
    processor::Mask,
    rooms::rooms,
};

use self::{
    config::{Config, Thresholds},
    model::Model,
    processor::{Detection, ImageProcessor},
};

const REPOSITORY: Repository = Repository::model(
    "mayocream/koharu-layout-highres",
    "53fef6f1301f4cbd8324ddc24f554b3767fc9e45",
);

/// The lowest confidence an onomatopoeia detection keeps by default. The
/// checkpoint recommends 0.3, but on 212 pages of Blue Archive's comics about
/// two in five onomatopoeia scoring under 0.4 were drawn marks, such as anger
/// veins, sweat drops, and motion lines, or lettering a text detection
/// already covers.
const ONOMATOPOEIA_THRESHOLD: f32 = 0.4;

/// The lowest confidence a bubble detection keeps by default. The checkpoint
/// recommends 0.5, but on the same pages most bubbles scoring between 0.4 and
/// 0.5 were plain speech bubbles, and inpainting fills only a detected
/// bubble's text with its paper.
const BUBBLE_THRESHOLD: f32 = 0.4;

/// Each class's lowest confidence a detection keeps, between 0 and 1, or the
/// default: the checkpoint's recommended one, except for onomatopoeia and
/// bubbles.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Options {
    pub text: Option<f32>,
    pub onomatopoeia: Option<f32>,
    pub bubble: Option<f32>,
    pub panel: Option<f32>,
}

#[derive(Debug)]
pub struct KoharuLayoutHighres {
    device: Device,
    model: Model,
    processor: ImageProcessor,
    thresholds: Thresholds,
}

impl KoharuLayoutHighres {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device);
        let [config, weights] = REPOSITORY
            .resolve(["config.json", "model.safetensors"])
            .await
            .context("failed to resolve Koharu Layout HighRes's files")?;
        let config: Config = serde_json::from_slice(&fs_err::read(&config)?)?;
        let processor = ImageProcessor::new(&config)?;

        let mut model = Model::new(device, kind)?;
        model.load(&weights)?;
        Ok(Self {
            device,
            model,
            processor,
            thresholds: Thresholds {
                onomatopoeia: ONOMATOPOEIA_THRESHOLD,
                bubble: BUBBLE_THRESHOLD,
                ..config.recommended_thresholds
            },
        })
    }

    /// The page's layout: its text and onomatopoeia in reading order, its
    /// bubbles with the room each of their texts has, and its panels, one
    /// detection to an object.
    pub fn inference(&self, image: &DynamicImage, options: &Options) -> Result<Layout> {
        let thresholds = Thresholds {
            text: options.text.unwrap_or(self.thresholds.text),
            onomatopoeia: options.onomatopoeia.unwrap_or(self.thresholds.onomatopoeia),
            bubble: options.bubble.unwrap_or(self.thresholds.bubble),
            panel: options.panel.unwrap_or(self.thresholds.panel),
        };
        let mut detections = koharu_torch::no_grad(|| {
            let pixel_values = self.processor.preprocess(image, self.device)?;
            let output = self.model.forward(&pixel_values)?;
            self.processor
                .postprocess(&output, image.width(), image.height(), thresholds)
        })?;
        suppress::suppress(&mut detections);
        // RF-DETR's lettering masks hug each stroke but miss counters and
        // parts of strokes.
        let page = image.to_rgb8();
        for detection in &mut detections {
            if matches!(detection.label.as_str(), "text" | "onomatopoeia") {
                detection.mask = measure::glyph_mask(&detection.mask, detection.bbox, &page);
            }
        }
        suppress::merge(&mut detections);
        order::sort(&mut detections);
        Ok(layout::build(detections, image.width(), image.height()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;

    use super::{KoharuLayoutHighres, Mask, Options};

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires CUDA"]
    async fn checkpoint_matches_rfdetr_upstream_structured_output() -> Result<()> {
        let device = koharu_runtime::device().await?;
        let image = image::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("benches/fixtures/object_detection/1.jpg"),
        )?;
        let model = KoharuLayoutHighres::load(device).await?;
        let result = model.inference(&image, &Options::default())?;

        // rfdetr 1.7.0 in F32 on CUDA, the page letterboxed as in training.
        // CUDA kernels vary across LibTorch releases, so compare structured
        // geometry and mask area in addition to bounded confidence differences.
        // The tolerances cover BF16 runs of both RF-DETR and this port; the
        // lower panel's right edge moves from 685 to 702 px between them.
        let area = |mask: &Mask| mask.pixels.iter().filter(|&&pixel| pixel != 0).count();
        let best = result
            .bubbles
            .iter()
            .max_by(|left, right| left.score.total_cmp(&right.score))
            .expect("a bubble");
        assert!((best.score - 0.977_458_1).abs() < 0.01);
        for (actual, expected) in best
            .bbox
            .into_iter()
            .zip([567.304_3, 547.317_1, 691.978, 720.860_9])
        {
            assert!((actual - expected).abs() < 6.0);
        }
        assert!(area(&best.mask).abs_diff(18_968) < 100);

        let lower_panel = result
            .panels
            .iter()
            .find(|panel| panel.bbox[1] > 700.0)
            .expect("lower page panel");
        let middle_panel = result
            .panels
            .iter()
            .find(|panel| panel.bbox[1] > 500.0 && panel.bbox[1] < 700.0)
            .expect("middle page panel");
        for (actual, expected) in
            lower_panel
                .bbox
                .into_iter()
                .zip([69.583_76, 798.296, 684.963_1, 1_009.382_4])
        {
            assert!((actual - expected).abs() < 20.0);
        }
        for (actual, expected) in middle_panel
            .bbox
            .into_iter()
            .zip([68.926, 552.693_5, 699.970_5, 788.836_5])
        {
            assert!((actual - expected).abs() < 5.0);
        }
        assert!(area(&lower_panel.mask).abs_diff(137_080) < 4_000);
        assert!(area(&middle_panel.mask).abs_diff(143_571) < 1_500);
        Ok(())
    }
}
