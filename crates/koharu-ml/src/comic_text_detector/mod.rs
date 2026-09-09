mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::processor::{Quad, TextBlock};

use self::{
    model::Model,
    processor::{postprocess, preprocess, rearranged_inference},
};

crate::model_repository!("mayocream/comic-text-detector" @ "15ade029f4dabd502bc97af6051c8b9f2bec24d5" {
    YOLO_WEIGHTS = "yolo-v5.safetensors",
    UNET_WEIGHTS = "unet.safetensors",
    DBNET_WEIGHTS = "dbnet.safetensors",
});

#[derive(Debug)]
pub struct ComicTextDetector {
    device: Device,
    model: Model,
}

impl ComicTextDetector {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let yolo_path = YOLO_WEIGHTS
            .resolve()
            .await
            .context("failed to resolve comic-text-detector YOLO weights")?;
        let unet_path = UNET_WEIGHTS
            .resolve()
            .await
            .context("failed to resolve comic-text-detector U-Net weights")?;
        let dbnet_path = DBNET_WEIGHTS
            .resolve()
            .await
            .context("failed to resolve comic-text-detector DBNet weights")?;

        let mut model = Model::new(device);
        model
            .load(&yolo_path, &unet_path, &dbnet_path)
            .context("failed to load comic-text-detector safetensors")?;

        Ok(Self { device, model })
    }

    pub fn inference(&self, image: &DynamicImage) -> Result<(image::GrayImage, Vec<TextBlock>)> {
        koharu_torch::no_grad(|| {
            if let Some(detection) =
                rearranged_inference(image, self.device, |input| self.model.forward(input))?
            {
                return Ok(detection);
            }
            let (pixel_values, dimensions) = preprocess(image, self.device)?;
            let outputs = self.model.forward(&pixel_values);
            postprocess(outputs, dimensions, image)
        })
    }
}
