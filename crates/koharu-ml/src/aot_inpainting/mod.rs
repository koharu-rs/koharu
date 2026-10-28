//! BallonsTranslator-compatible AOT image inpainting.

mod model;
mod processor;

use anyhow::{Context, Result};
use image::{DynamicImage, GrayImage, RgbImage};

use self::{model::Model, processor::Processor};

crate::model_repository!("mayocream/aot-inpainting" @ "cffe2346ac2b5ebe1f2d61335d602d12cc144c6f" {
    WEIGHTS = "model.safetensors"
});

#[derive(Debug)]
pub struct AotInpainting {
    model: Model,
    processor: Processor,
}

impl AotInpainting {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device)?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve AOT inpainting weights")?;
        let mut model = Model::new(device, kind)?;
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self {
            model,
            processor: Processor::new(device),
        })
    }

    pub fn inference(&self, image: &DynamicImage, mask: &GrayImage) -> Result<RgbImage> {
        self.inference_with_max_side(image, mask, 2048)
    }

    pub fn inference_with_max_side(
        &self,
        image: &DynamicImage,
        mask: &GrayImage,
        max_side: u32,
    ) -> Result<RgbImage> {
        tch::no_grad(|| self.processor.call(&self.model, image, mask, max_side))
    }
}
