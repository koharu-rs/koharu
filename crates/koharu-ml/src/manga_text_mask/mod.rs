mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::processor::{MangaTextMask, MangaTextMaskCleaningOptions};

use self::{model::Model, processor::Processor};

crate::model_repository!("mayocream/manga-text-segmentation-2025" @ "efd866e3ac6595ea20722f35ae343c403056ba76" {
    WEIGHTS = "model.safetensors"
});

#[derive(Debug)]
pub struct MangaTextMaskGenerator {
    model: Model,
    processor: Processor,
}

impl MangaTextMaskGenerator {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve manga text segmentation weights")?;
        let mut model = Model::new(device);
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self {
            model,
            processor: Processor::new(device),
        })
    }

    /// Runs the original full-resolution inference path without test-time augmentation.
    pub fn inference(&self, image: &DynamicImage) -> Result<MangaTextMask> {
        self.call(image, false, false, None)
    }

    /// Runs the original independent horizontal and vertical flip augmentations.
    pub fn inference_with_tta(
        &self,
        image: &DynamicImage,
        horizontal_flip: bool,
        vertical_flip: bool,
    ) -> Result<MangaTextMask> {
        self.call(image, horizontal_flip, vertical_flip, None)
    }

    /// Downscales oversized input with `fast_image_resize` before inference.
    ///
    /// This is an explicit performance extension. `inference` remains pixel-for-pixel
    /// aligned with the upstream preprocessing and never resizes its input.
    pub fn inference_with_max_side(
        &self,
        image: &DynamicImage,
        max_side: u32,
    ) -> Result<MangaTextMask> {
        self.call(image, false, false, Some(max_side))
    }

    fn call(
        &self,
        image: &DynamicImage,
        horizontal_flip: bool,
        vertical_flip: bool,
        max_side: Option<u32>,
    ) -> Result<MangaTextMask> {
        koharu_torch::no_grad(|| {
            self.processor
                .call(&self.model, image, horizontal_flip, vertical_flip, max_side)
        })
    }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgb, RgbImage};

    use super::MangaTextMaskGenerator;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_loads_and_runs() -> anyhow::Result<()> {
        crate::init().await?;
        let model = MangaTextMaskGenerator::load(crate::Device::cpu()).await?;
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(96, 64, |x, y| {
            Rgb([
                ((x * 7 + y * 3) % 256) as u8,
                ((x * 5 + y * 11) % 256) as u8,
                ((x * 13 + y * 2) % 256) as u8,
            ])
        }));
        let output = model.inference(&image)?;
        assert_eq!((output.width(), output.height()), (96, 64));
        assert_eq!(output.probabilities().len(), 96 * 64);
        assert!(
            output
                .probabilities()
                .iter()
                .all(|probability| (0.0..=1.0).contains(probability))
        );

        // Reference values from the commit-pinned Python implementation on CPU.
        let expected = [
            0.007_002_114_3,
            0.020_354_025,
            0.010_177_039,
            0.000_451_958_16,
            0.000_130_330_62,
            0.000_302_413_57,
            0.000_051_601_994,
            0.006_196_571_5,
        ];
        for (&index, expected) in [0, 1, 95, 96, 777, 2048, 4095, 6143].iter().zip(expected) {
            let actual = output.probabilities()[index];
            assert!(
                (actual - expected).abs() < 1e-6,
                "probability {index}: actual={actual}, expected={expected}"
            );
        }
        let sum = output.probabilities().iter().sum::<f32>();
        assert!((sum - 1.352_233).abs() < 1e-4, "sum={sum}");

        let resized = model.inference_with_max_side(&image, 32)?;
        assert_eq!((resized.width(), resized.height()), (96, 64));
        assert_eq!(resized.probabilities().len(), 96 * 64);
        Ok(())
    }
}
