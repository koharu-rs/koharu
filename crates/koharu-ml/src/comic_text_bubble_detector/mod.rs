//! Comic Text & Bubble Detector RT-DETR-v2 implementation.
//!
//! Original implementation:
//! https://github.com/ogkalu2/comic-translate/blob/ca3261fd1a8d4805f6b9cc0669847d463ccb8a41/modules/detection/rtdetr_v2.py

mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::{RTDetrResNetConfig, RTDetrV2Config},
    processor::{RTDetrImageProcessor, SizeDict, TextBlock},
};

use self::{
    model::Model,
    processor::{ImageSlicer, create_text_blocks},
};

crate::model_repository!("ogkalu/comic-text-and-bubble-detector" @ "16e8a622f91fabc6b5b65c96d32d1183f8843546" {
    CONFIG = "config.json",
    PREPROCESSOR_CONFIG = "preprocessor_config.json",
    WEIGHTS = "model.safetensors",
});

#[derive(Debug)]
pub struct RTDetrV2Detection {
    device: Device,
    config: RTDetrV2Config,
    processor: RTDetrImageProcessor,
    image_slicer: ImageSlicer,
    model: Model,
}

impl RTDetrV2Detection {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve comic text/bubble detector config")?;
        let processor_path = PREPROCESSOR_CONFIG
            .resolve()
            .await
            .context("failed to resolve comic text/bubble detector preprocessor config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve comic text/bubble detector weights")?;

        let mut config: RTDetrV2Config = serde_json::from_str(
            &std::fs::read_to_string(&config_path)
                .with_context(|| format!("failed to read {}", config_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
        let processor: RTDetrImageProcessor = serde_json::from_str(
            &std::fs::read_to_string(&processor_path)
                .with_context(|| format!("failed to read {}", processor_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", processor_path.display()))?;

        // The processor always produces this resolution, so RT-DETR can reuse
        // its fixed anchors rather than rebuilding and uploading them per slice.
        config.anchor_image_size = Some(vec![processor.size.height, processor.size.width]);

        let mut model = Model::new(config.clone(), device);
        model
            .load(&weights_path)
            .context("failed to load comic text/bubble detector safetensors")?;

        Ok(Self {
            device,
            config,
            processor,
            image_slicer: ImageSlicer::default(),
            model,
        })
    }

    pub fn inference(
        &self,
        image: &DynamicImage,
        confidence_threshold: f32,
    ) -> Result<Vec<TextBlock>> {
        koharu_torch::no_grad(|| {
            let (bubble_boxes, text_boxes) = self
                .image_slicer
                .process_slices_for_detection(image, |slice| {
                    self.detect_single_image(slice, confidence_threshold)
                })?;
            Ok(create_text_blocks(image, text_boxes, bubble_boxes))
        })
    }

    #[allow(clippy::type_complexity)]
    fn detect_single_image(
        &self,
        image: &DynamicImage,
        confidence_threshold: f32,
    ) -> Result<(Vec<[f32; 4]>, Vec<[f32; 4]>)> {
        let pixel_values = self.processor.preprocess(image, self.device)?;
        let outputs = self.model.forward(&pixel_values);
        let results = self.processor.post_process_object_detection(
            &outputs,
            confidence_threshold,
            &[(image.height(), image.width())],
            self.config.use_focal_loss,
        )?;
        let result = results
            .into_iter()
            .next()
            .context("missing comic text/bubble detector result")?;

        let mut bubble_boxes = Vec::new();
        let mut text_boxes = Vec::new();
        for ((bbox, _score), label) in result
            .boxes
            .into_iter()
            .zip(result.scores)
            .zip(result.labels)
        {
            let bbox = bbox.map(|value| value as i32 as f32);
            match label {
                0 => bubble_boxes.push(bbox),
                1 | 2 => text_boxes.push(bbox),
                _ => {}
            }
        }
        Ok((bubble_boxes, text_boxes))
    }
}
