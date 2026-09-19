//! Native RORem SDXL inpainting model assembly.
//!
//! Split checkpoint loading follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/stable-diffusion.cpp#L617-L673
//! SDXL inpainting is selected from the marker and nine-channel UNet at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/model_loader.cpp#L428-L588

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow, ensure};
use koharu_diffusion::{Context, ContextParams, ImageGenerationParams, RgbImage};

use crate::Backend;

#[derive(Debug)]
pub(super) struct ModelPaths {
    pub diffusion_model: PathBuf,
    pub version_marker: PathBuf,
    pub vae: PathBuf,
    pub clip_l: PathBuf,
    pub clip_g: PathBuf,
}

#[derive(Debug)]
pub(super) struct Model {
    context: Mutex<Context>,
}

impl Model {
    pub(super) fn new(device: &crate::Device, paths: ModelPaths) -> Result<Self> {
        let context = Context::new(&context_params(device, paths))
            .context("failed to load RORem mixed components")?;
        ensure!(
            context.supports_image_generation(),
            "the loaded RORem mixed context does not support image generation"
        );
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub(super) fn forward(&self, params: &ImageGenerationParams) -> Result<Vec<RgbImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("RORem mixed context lock was poisoned"))?;
        context
            .generate_image(params)
            .context("RORem mixed inference failed")
    }
}

fn context_params(device: &crate::Device, paths: ModelPaths) -> ContextParams {
    let use_cuda = device.backend == Backend::Cuda;
    ContextParams {
        model_path: Some(paths.version_marker),
        diffusion_model_path: Some(paths.diffusion_model),
        vae_path: Some(paths.vae),
        clip_l_path: Some(paths.clip_l),
        clip_g_path: Some(paths.clip_g),
        enable_mmap: true,
        flash_attention: use_cuda,
        diffusion_flash_attention: use_cuda,
        diffusion_conv_direct: use_cuda,
        vae_conv_direct: use_cuda,
        backend: Some(device.name.clone()),
        ..ContextParams::default()
    }
}
