mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::processor::Detection;

use self::{config::Config, model::Model, processor::Processor};

crate::model_repository!("mayocream/coo-comic-onomatopoeia-safetensors" @ "b5d31460573b6f61c1d4bdaea5fe4e18425e6a61" {
    WEIGHTS = "mtsv3/model.safetensors"
});

/// COO's reported-best MTSv3 comic onomatopoeia detector.
#[derive(Debug)]
pub struct ComicOnomatopoeiaDetector {
    device: Device,
    model: Model,
    processor: Processor,
}

impl ComicOnomatopoeiaDetector {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve COO MTSv3 weights")?;
        let config = Config::default();
        let processor = Processor::new(config);
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

    pub fn inference(&self, image: &DynamicImage) -> Result<Vec<Detection>> {
        koharu_torch::no_grad(|| {
            let (pixel_values, size) = self.processor.preprocess(image, self.device)?;
            let prediction = self.model.forward(&pixel_values);
            self.processor.postprocess(&prediction, size)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch CUDA runtime"]
    async fn checkpoint_inference_smoke_test() -> Result<()> {
        crate::init().await?;
        let model = ComicOnomatopoeiaDetector::load(crate::Device::cuda(0)).await?;
        let image = image::open(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("benches/fixtures/object_detection/1.jpg"),
        )?;
        assert!(!model.inference(&image)?.is_empty());
        Ok(())
    }
}
