//! Native FLUX.3 Image Klein model assembly.
//!
//! Component mapping follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/58bb3c1b1cd4749e94e39f537e9adfce295dcaee/docs/flux3.md
//! which builds the Qwen3-VL text encoder, the FLUX.2 autoencoder, and the
//! flow-map denoiser in:
//! https://github.com/leejet/stable-diffusion.cpp/blob/58bb3c1b1cd4749e94e39f537e9adfce295dcaee/src/model/integrations/flux3.cpp

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
            vae: Some(&paths.vae),
            vae_format: Some(VaeFormat::Flux2),
            device: Some(&device.name),
            mmap: true,
            flash_attention: cuda,
            conv_direct: cuda,
            ..ContextParams::default()
        })
        .context("failed to load FLUX.3 Image Klein components")?;
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub(super) fn forward(&self, params: &ImageParams) -> Result<Vec<RgbImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("FLUX.3 Image Klein context lock was poisoned"))?;
        let images = context
            .generate_image(params)
            .context("FLUX.3 Image Klein inference failed")?;
        Ok(images.into_iter().map(DynamicImage::into_rgb8).collect())
    }
}
