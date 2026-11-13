//! Native MOSS-Image model assembly.
//!
//! Component mapping follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/58bb3c1b1cd4749e94e39f537e9adfce295dcaee/docs/moss_image.md
//! which builds the Qwen3.5 text encoder, read with MOSS-Image's own
//! tokenizer, the FLUX.2 autoencoder, and the denoiser in:
//! https://github.com/leejet/stable-diffusion.cpp/blob/58bb3c1b1cd4749e94e39f537e9adfce295dcaee/src/model/integrations/moss_image.cpp

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow};
use image::{DynamicImage, RgbImage};
use koharu_diffusion::{Context, ContextParams, ImageParams, VaeFormat};
use koharu_runtime::Backend;

#[derive(Debug)]
pub(super) struct ModelPaths {
    pub(super) transformer: PathBuf,
    pub(super) text_encoder: PathBuf,
    pub(super) vae: PathBuf,
    pub(super) tokenizer: PathBuf,
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
            // Qwen3.5's vocabulary isn't the Qwen one sd.cpp holds.
            tokenizer: Some(&paths.tokenizer),
            vae: Some(&paths.vae),
            vae_format: Some(VaeFormat::Flux2),
            device: Some(&device.name),
            mmap: true,
            flash_attention: cuda,
            conv_direct: cuda,
            ..ContextParams::default()
        })
        .context("failed to load MOSS-Image components")?;
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub(super) fn forward(&self, params: &ImageParams) -> Result<Vec<RgbImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("MOSS-Image context lock was poisoned"))?;
        let images = context
            .generate_image(params)
            .context("MOSS-Image inference failed")?;
        Ok(images.into_iter().map(DynamicImage::into_rgb8).collect())
    }
}
