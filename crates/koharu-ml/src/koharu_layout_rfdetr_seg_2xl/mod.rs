//! High-resolution manga layout instance segmentation with RF-DETR Seg 2XL.
//!
//! Checkpoint and strict Python loader:
//! https://huggingface.co/mayocream/koharu-layout-rfdetr-seg-2xl-1152/tree/aed55fdb8ca953c6bec33cf6ed6dd52a9b72bfa2
//! RF-DETR upstream implementation:
//! https://github.com/roboflow/rf-detr/tree/4ab7c18729de9d02ffd0495795d0831b5630f01b

mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::{KoharuLayoutRFDetrSeg2XLConfig, KoharuLayoutThresholds},
    processor::{
        InputFit, KoharuLayoutDetection, KoharuLayoutDetections, KoharuLayoutMask,
        KoharuLayoutRFDetrImageProcessor,
    },
};

use self::model::Model;

crate::model_repository!("mayocream/koharu-layout-rfdetr-seg-2xl-1152" @ "aed55fdb8ca953c6bec33cf6ed6dd52a9b72bfa2" {
    CONFIG = "inference_config.json",
    WEIGHTS = "model.safetensors",
});

#[derive(Debug)]
pub struct KoharuLayoutRFDetrSeg2XL {
    device: Device,
    model: Model,
    processor: KoharuLayoutRFDetrImageProcessor,
}

impl KoharuLayoutRFDetrSeg2XL {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve KoharuLayout RF-DETR inference config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve KoharuLayout RF-DETR weights")?;
        let config = KoharuLayoutRFDetrSeg2XLConfig::from_file(&config_path)?;
        let processor = KoharuLayoutRFDetrImageProcessor::new(&config)?;
        let mut model = Model::new(device);
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self {
            device,
            model,
            processor,
        })
    }

    pub fn inference(
        &self,
        image: &DynamicImage,
        input_fit: InputFit,
    ) -> Result<KoharuLayoutDetections> {
        self.inference_with_thresholds(image, self.processor.recommended_thresholds(), input_fit)
    }

    pub fn inference_with_thresholds(
        &self,
        image: &DynamicImage,
        thresholds: KoharuLayoutThresholds,
        input_fit: InputFit,
    ) -> Result<KoharuLayoutDetections> {
        koharu_torch::no_grad(|| {
            let (pixel_values, transform) =
                self.processor.preprocess(image, self.device, input_fit)?;
            let output = self.model.forward(&pixel_values);
            self.processor.postprocess(&output, &transform, thresholds)
        })
    }

    pub fn recommended_thresholds(&self) -> KoharuLayoutThresholds {
        self.processor.recommended_thresholds()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;

    use super::{InputFit, KoharuLayoutDetections, KoharuLayoutRFDetrSeg2XL};

    /// Loads the RF-DETR fixture page, which is deliberately not square.
    async fn fixture() -> Result<image::DynamicImage> {
        Ok(image::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("benches/fixtures/object_detection/1.jpg"),
        )?)
    }

    /// Asserts the postprocessing contract that must hold for every fit
    /// strategy: a caller can treat each box and mask as page pixels without
    /// further clamping. A transform that mis-maps geometry, emits a negative
    /// origin or keeps a collapsed box fails here regardless of what the model
    /// happened to predict.
    fn assert_page_space(result: &KoharuLayoutDetections) {
        let width = result.image_width as f32;
        let height = result.image_height as f32;
        for detection in &result.detections {
            let [x1, y1, x2, y2] = detection.bbox;
            assert!(
                [x1, y1, x2, y2].iter().all(|value| value.is_finite()),
                "{} has a non-finite box {detection:?}",
                detection.label,
            );
            assert!(
                (0.0..=width).contains(&x1) && (0.0..=width).contains(&x2),
                "{} box {detection:?} leaves the page horizontally",
                detection.label,
            );
            assert!(
                (0.0..=height).contains(&y1) && (0.0..=height).contains(&y2),
                "{} box {detection:?} leaves the page vertically",
                detection.label,
            );
            assert!(
                x2 > x1 && y2 > y1,
                "{} kept a collapsed box {detection:?}",
                detection.label,
            );
            assert!(
                detection.mask.x + detection.mask.width <= result.image_width
                    && detection.mask.y + detection.mask.height <= result.image_height,
                "{} mask {:?} leaves the page",
                detection.label,
                (
                    detection.mask.x,
                    detection.mask.y,
                    detection.mask.width,
                    detection.mask.height
                ),
            );
        }
    }

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires CUDA"]
    async fn every_fit_strategy_yields_page_space_geometry() -> Result<()> {
        crate::init().await?;
        let image = fixture().await?;
        let model = KoharuLayoutRFDetrSeg2XL::load(crate::Device::cuda(0)).await?;

        // Deliberately no reference coordinates. The checkpoint was trained on
        // stretched input, so letterboxing changes what it predicts and any
        // absolute baseline recorded against the stretch says nothing about the
        // current code. What must hold for every strategy is that the fit
        // transform and the postprocessing agree on page space.
        for input_fit in [InputFit::LetterBox, InputFit::Stretch] {
            let result = model.inference(&image, input_fit)?;
            assert_eq!(
                (result.image_width, result.image_height),
                (image.width(), image.height()),
                "detections must be reported in page pixels",
            );
            assert_page_space(&result);
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires CUDA"]
    async fn both_fit_strategies_agree_on_the_dominant_region() -> Result<()> {
        crate::init().await?;
        let image = fixture().await?;
        let model = KoharuLayoutRFDetrSeg2XL::load(crate::Device::cuda(0)).await?;

        // A relative cross-check that survives retraining: the highest-scoring
        // region of a page is the same region under both mappings. A transposed
        // or mis-offset transform displaces the box far enough to change it,
        // while ordinary scoring jitter does not.
        let dominant = |result: &KoharuLayoutDetections| {
            result
                .detections
                .iter()
                .max_by(|left, right| left.score.total_cmp(&right.score))
                .map(|detection| detection.label.clone())
        };
        let letterboxed = model.inference(&image, InputFit::LetterBox)?;
        let stretched = model.inference(&image, InputFit::Stretch)?;
        let (Some(letterboxed), Some(stretched)) = (dominant(&letterboxed), dominant(&stretched))
        else {
            panic!("both fit strategies must detect the dominant region");
        };
        assert_eq!(
            letterboxed, stretched,
            "the dominant region must not depend on the fit strategy",
        );
        Ok(())
    }
}
