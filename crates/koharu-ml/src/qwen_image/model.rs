//! Native Qwen-Image 2.1 model assembly.
//!
//! Component mapping follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/58bb3c1b1cd4749e94e39f537e9adfce295dcaee/docs/qwen_image_2.1.md
//! which builds the Qwen3-VL 8B text encoder, with its vision projector for
//! condition images, Qwen-Image 2.1's own VAE, and the denoiser in:
//! https://github.com/leejet/stable-diffusion.cpp/blob/58bb3c1b1cd4749e94e39f537e9adfce295dcaee/src/model/integrations/qwen_image.cpp

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow};
use image::{DynamicImage, RgbaImage};
use koharu_diffusion::{Context, ContextParams, ImageParams};
use koharu_runtime::Backend;

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
}

impl Model {
    /// The model on `device`.
    pub(super) fn new(device: &crate::Device, paths: &ModelPaths) -> Result<Self> {
        let cuda = matches!(device.backend, Backend::Cuda { .. });
        let context = Context::new(&ContextParams {
            diffusion_model: Some(&paths.transformer),
            llm: Some(&paths.text_encoder),
            llm_vision: Some(&paths.vision),
            vae: Some(&paths.vae),
            device: Some(&device.name),
            mmap: true,
            flash_attention: cuda,
            conv_direct: cuda,
            ..ContextParams::default()
        })
        .context("failed to load Qwen-Image 2.1 components")?;
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    /// Qwen-Image 2.1 decodes four channels, so its images carry alpha.
    pub(super) fn forward(&self, params: &ImageParams) -> Result<Vec<RgbaImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("Qwen-Image 2.1 context lock was poisoned"))?;
        let images = context
            .generate_image(params)
            .context("Qwen-Image 2.1 inference failed")?;
        Ok(images.into_iter().map(DynamicImage::into_rgba8).collect())
    }
}
