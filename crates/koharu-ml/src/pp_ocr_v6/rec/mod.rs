mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{
    config::PPOCRV6MediumRecConfig,
    processor::{PPOCRV6MediumRecImageProcessor, SizeDict, TextRecognition},
};

use self::model::Model;

crate::model_repository!("PaddlePaddle/PP-OCRv6_medium_rec_safetensors" @ "024cad6a831de75c2c3c26e711ba8c4a82ccd24b" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
    PROCESSOR = "preprocessor_config.json",
});

#[derive(Debug)]
pub struct PPOCRV6MediumRec {
    device: Device,
    model: Model,
    processor: PPOCRV6MediumRecImageProcessor,
}

impl PPOCRV6MediumRec {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve PP-OCRv6 medium recognition config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve PP-OCRv6 medium recognition weights")?;
        let processor_path = PROCESSOR
            .resolve()
            .await
            .context("failed to resolve PP-OCRv6 medium recognition image processor")?;

        let config = PPOCRV6MediumRecConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = PPOCRV6MediumRecImageProcessor::from_file(&processor_path)
            .with_context(|| format!("failed to read {}", processor_path.display()))?;
        if processor.character_list.len() != config.head_out_channels as usize {
            anyhow::bail!(
                "PP-OCRv6 recognition vocabulary has {} entries but the head has {} outputs",
                processor.character_list.len(),
                config.head_out_channels
            );
        }
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

    pub fn inference(&self, image: &DynamicImage) -> Result<TextRecognition> {
        koharu_torch::no_grad(|| {
            let pixel_values = self.processor.preprocess(image, self.device)?;
            let probabilities = self.model.forward(&pixel_values);
            self.processor.postprocess(&probabilities)
        })
    }
}

#[cfg(test)]
mod tests {
    use koharu_runtime::{Feature, Runtime};

    use super::*;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and LibTorch runtime"]
    async fn loads_medium_recognizer_checkpoint() {
        Runtime::discover([Feature::Torch])
            .unwrap()
            .initialize()
            .await
            .unwrap();
        let model = PPOCRV6MediumRec::load(crate::Device::cpu()).await.unwrap();
        model.inference(&DynamicImage::new_rgb8(128, 32)).unwrap();
    }
}
