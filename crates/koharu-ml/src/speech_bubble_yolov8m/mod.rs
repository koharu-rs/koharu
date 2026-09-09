mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::YoloV8mSpeechBubbleConfig,
    processor::{
        YoloV8mSegImageProcessor, YoloV8mSpeechBubbleInstance, YoloV8mSpeechBubbleInstances,
        YoloV8mSpeechBubbleMask,
    },
};

use self::model::Model;

crate::model_repository!("mayocream/speech-bubble-segmentation" @ "387bc1e93f3d24702bc8609798b6a13b37420edc" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
});

#[derive(Debug)]
pub struct YoloV8mSpeechBubbleSegmenter {
    device: Device,
    config: YoloV8mSpeechBubbleConfig,
    model: Model,
    processor: YoloV8mSegImageProcessor,
}

impl YoloV8mSpeechBubbleSegmenter {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve speech bubble segmentation config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve speech bubble segmentation weights")?;
        let config = YoloV8mSpeechBubbleConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = YoloV8mSegImageProcessor::new(&config)?;
        let mut model = Model::new(&config, device)?;
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self {
            device,
            config,
            model,
            processor,
        })
    }

    pub fn inference(&self, image: &DynamicImage) -> Result<YoloV8mSpeechBubbleInstances> {
        self.inference_with_thresholds(
            image,
            self.config.default_confidence_threshold,
            self.config.default_nms_threshold,
        )
    }

    pub fn inference_with_thresholds(
        &self,
        image: &DynamicImage,
        confidence_threshold: f32,
        nms_threshold: f32,
    ) -> Result<YoloV8mSpeechBubbleInstances> {
        koharu_torch::no_grad(|| {
            let (pixel_values, letterbox) = self.processor.preprocess(image, self.device)?;
            let outputs = self.model.forward(&pixel_values);
            self.processor
                .postprocess(&outputs, &letterbox, confidence_threshold, nms_threshold)
        })
    }
}
