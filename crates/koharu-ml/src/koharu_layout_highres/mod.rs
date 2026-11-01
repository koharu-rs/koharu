//! High-resolution manga layout: RF-DETR Seg 2XL instance segmentation of a
//! page's text, onomatopoeia, speech bubbles, and panels.
//!
//! Checkpoint:
//! https://huggingface.co/mayocream/koharu-layout-highres/tree/53fef6f1301f4cbd8324ddc24f554b3767fc9e45
//! Upstream:
//! https://github.com/roboflow/rf-detr/tree/4ab7c18729de9d02ffd0495795d0831b5630f01b

mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_runtime::Repository;
use koharu_torch::Device;

pub use self::processor::{Detection, Detections, Mask};

use self::{
    config::{Config, Thresholds},
    model::Model,
    processor::ImageProcessor,
};

const REPOSITORY: Repository = Repository::model(
    "mayocream/koharu-layout-highres",
    "53fef6f1301f4cbd8324ddc24f554b3767fc9e45",
);

/// Each class's lowest confidence a detection keeps, between 0 and 1, or the
/// checkpoint's recommended one.
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
        let (device, kind) = crate::backend::torch(device)?;
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
            thresholds: config.recommended_thresholds,
        })
    }

    pub fn inference(&self, image: &DynamicImage, options: &Options) -> Result<Detections> {
        let thresholds = Thresholds {
            text: options.text.unwrap_or(self.thresholds.text),
            onomatopoeia: options.onomatopoeia.unwrap_or(self.thresholds.onomatopoeia),
            bubble: options.bubble.unwrap_or(self.thresholds.bubble),
            panel: options.panel.unwrap_or(self.thresholds.panel),
        };
        koharu_torch::no_grad(|| {
            let pixel_values = self.processor.preprocess(image, self.device)?;
            let output = self.model.forward(&pixel_values)?;
            self.processor
                .postprocess(&output, image.width(), image.height(), thresholds)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;

    use super::{KoharuLayoutHighres, Options};

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires CUDA"]
    async fn checkpoint_matches_rfdetr_upstream_structured_output() -> Result<()> {
        let device = &koharu_runtime::devices().await?.torch;
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
        let best = &result.detections[0];
        assert_eq!(best.label, "bubble");
        assert!((best.score - 0.977_458_1).abs() < 0.01);
        for (actual, expected) in best
            .bbox
            .into_iter()
            .zip([567.304_3, 547.317_1, 691.978, 720.860_9])
        {
            assert!((actual - expected).abs() < 6.0);
        }
        assert!(best.area.abs_diff(18_968) < 100);

        let lower_panel = result
            .detections
            .iter()
            .find(|detection| detection.label == "panel" && detection.bbox[1] > 700.0)
            .expect("lower page panel");
        let middle_panel = result
            .detections
            .iter()
            .find(|detection| {
                detection.label == "panel" && detection.bbox[1] > 500.0 && detection.bbox[1] < 700.0
            })
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
        assert!(lower_panel.area.abs_diff(137_080) < 4_000);
        assert!(middle_panel.area.abs_diff(143_571) < 1_500);
        Ok(())
    }
}
