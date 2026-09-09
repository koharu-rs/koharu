mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::PPOCRV6MediumDetConfig,
    processor::{PPOCRV6MediumDetImageProcessor, TextDetection, TextDetections},
};

use self::model::Model;

crate::model_repository!("PaddlePaddle/PP-OCRv6_medium_det_safetensors" @ "4236c2b61741a259c091fd879dcc4edc339e916c" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
    PROCESSOR = "preprocessor_config.json",
});

#[derive(Debug)]
pub struct PPOCRV6MediumDet {
    device: Device,
    model: Model,
    processor: PPOCRV6MediumDetImageProcessor,
}

impl PPOCRV6MediumDet {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve PP-OCRv6 medium detection config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve PP-OCRv6 medium detection weights")?;
        let processor_path = PROCESSOR
            .resolve()
            .await
            .context("failed to resolve PP-OCRv6 medium detection image processor")?;

        let config = PPOCRV6MediumDetConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = PPOCRV6MediumDetImageProcessor::from_file(&processor_path)
            .with_context(|| format!("failed to read {}", processor_path.display()))?;
        let mut model = Model::new(&config, device);
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;

        Ok(Self {
            device,
            model,
            processor,
        })
    }

    pub fn inference(&self, image: &DynamicImage) -> Result<TextDetections> {
        koharu_torch::no_grad(|| {
            let pixel_values = self.processor.preprocess(image, self.device)?;
            let output = self.model.forward(&pixel_values);
            self.processor.postprocess(&output, image)
        })
    }
}

#[cfg(test)]
mod tests {
    use koharu_runtime::{Feature, Runtime};

    use super::*;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and LibTorch runtime"]
    async fn loads_medium_detector_checkpoint() {
        Runtime::discover([Feature::Torch])
            .unwrap()
            .initialize()
            .await
            .unwrap();
        let model = PPOCRV6MediumDet::load(crate::Device::cpu()).await.unwrap();
        model.inference(&DynamicImage::new_rgb8(64, 64)).unwrap();
    }
}
