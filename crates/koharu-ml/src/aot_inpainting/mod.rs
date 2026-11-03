//! AOT-GAN inpainting as BallonsTranslator runs it.
//!
//! Upstream:
//! https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/inpaint/inpaint_default.py

mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GrayImage, RgbImage};
use koharu_runtime::Repository;
use koharu_torch::Device;

use self::model::Model;

const REPOSITORY: Repository = Repository::model(
    "mayocream/aot-inpainting",
    "cffe2346ac2b5ebe1f2d61335d602d12cc144c6f",
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// The longest side the page is shrunk to before it reaches the model,
    /// BallonsTranslator's `inpaint_size`.
    pub max_side: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { max_side: 2048 }
    }
}

#[derive(Debug)]
pub struct AotInpainting {
    device: Device,
    model: Model,
}

impl AotInpainting {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device);
        let [weights] = REPOSITORY
            .resolve(["model.safetensors"])
            .await
            .context("failed to resolve AOT inpainting's files")?;

        let mut model = Model::new(device, kind)?;
        model.load(&weights)?;
        Ok(Self { device, model })
    }

    /// `image` repainted where `mask` is white.
    pub fn inference(
        &self,
        image: &DynamicImage,
        mask: &GrayImage,
        options: &Options,
    ) -> Result<RgbImage> {
        ensure!(
            options.max_side > 0,
            "AOT inpainting max side must be positive"
        );
        let image = image.to_rgb8();
        ensure!(
            image.dimensions() == mask.dimensions(),
            "image and mask dimensions differ: image={:?}, mask={:?}",
            image.dimensions(),
            mask.dimensions()
        );
        ensure!(
            image.width() > 0 && image.height() > 0,
            "image dimensions must be non-zero"
        );
        koharu_torch::no_grad(|| {
            let input = processor::preprocess(&image, mask, options.max_side, self.device)?;
            let output = self.model.forward(&input.image, &input.mask)?;
            processor::postprocess(&output, &input, &image, mask)
        })
    }
}
