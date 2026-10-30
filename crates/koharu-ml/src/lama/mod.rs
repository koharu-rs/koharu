//! LaMa inference with IOPaint-compatible orchestration.

mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::{DynamicImage, GrayImage, RgbImage};
use koharu_runtime::Repository;

pub use self::config::{HDStrategy, InpaintRequest};
use self::{config::FFCResNetGeneratorConfig, model::Model, processor::InpaintModel};

const REPOSITORY: Repository = Repository::model(
    "mayocream/lama-manga",
    "f91c85b26913b3e83f9877867b4c336da3675238",
);

#[derive(Debug)]
pub struct LaMa {
    model: Model,
    processor: InpaintModel,
}

impl LaMa {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device)?;
        let [weights_path] = REPOSITORY
            .resolve(["lama-manga.safetensors"])
            .await
            .context("failed to resolve LaMa weights")?;
        let mut model = Model::new(&FFCResNetGeneratorConfig::default(), device, kind)?;
        model
            .load(&weights_path)
            .context("failed to load LaMa safetensors")?;
        Ok(Self {
            model,
            processor: InpaintModel::new(device),
        })
    }

    pub fn inference(
        &self,
        image: &DynamicImage,
        mask: &GrayImage,
        config: &InpaintRequest,
    ) -> Result<RgbImage> {
        koharu_torch::no_grad(|| self.processor.call(&self.model, image, mask, config))
    }
}
