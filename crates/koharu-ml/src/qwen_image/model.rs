//! Native Qwen-Image 2.1 Turbo model assembly.
//!
//! Component mapping follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/532d2c15/docs/models/qwen_image_2.1.md
//! which builds the Qwen3-VL 8B text encoder, with its vision projector for
//! condition images, Qwen-Image 2.1's own VAE, and the denoiser in:
//! https://github.com/leejet/stable-diffusion.cpp/blob/532d2c15/src/model/integrations/qwen_image.cpp

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow};
use image::{DynamicImage, RgbaImage};
use koharu_diffusion::{Cache, Canceller, Context, ContextParams, ImageParams};

#[derive(Debug)]
pub(super) struct ModelPaths {
    pub(super) transformer: PathBuf,
    pub(super) text_encoder: PathBuf,
    pub(super) vision: PathBuf,
    pub(super) vae: PathBuf,
}

#[derive(Debug)]
pub(super) struct Model {
    context: Mutex<Context>,
    /// Stops a generation under way.
    pub(super) canceller: Canceller,
}

impl Model {
    /// The model on `device`.
    pub(super) fn new(device: &crate::Device, paths: &ModelPaths) -> Result<Self> {
        let context = Context::new(&ContextParams {
            diffusion_model: Some(&paths.transformer),
            llm: Some(&paths.text_encoder),
            llm_vision: Some(&paths.vision),
            vae: Some(&paths.vae),
            device: Some(&device.name),
            mmap: true,
            ..ContextParams::default()
        })
        .context("failed to load Qwen-Image 2.1 Turbo components")?;
        Ok(Self {
            canceller: context.canceller(),
            context: Mutex::new(context),
        })
    }

    /// Qwen-Image 2.1 decodes four channels, so its images carry alpha.
    pub(super) fn forward(&self, params: &ImageParams) -> Result<Vec<RgbaImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("Qwen-Image 2.1 Turbo context lock was poisoned"))?;
        let images = context
            .generate_image(&ImageParams {
                // EasyCache skips 2 of an inpainting's 5 steps, 1.4 times as
                // fast, for fills within a level of the full run's.
                cache: Some(Cache::EasyCache),
                // 512-pixel tiles keep the VAE within 1.2 GB on a megapixel, which
                // an 8 GB GPU beside the other models can spare.
                vae_tile: Some(512),
                ..params.clone()
            })
            .context("Qwen-Image 2.1 Turbo inference failed")?;
        Ok(images.into_iter().map(DynamicImage::into_rgba8).collect())
    }
}
