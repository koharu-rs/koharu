//! Native RORem SDXL inpainting model assembly.
//!
//! Split checkpoint loading follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/stable-diffusion.cpp#L617-L673
//! SDXL inpainting is selected from the marker and nine-channel UNet at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/model_loader.cpp#L428-L588

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow};
use image::RgbImage;
use koharu_diffusion::{Context, ContextParams, ImageParams};
use koharu_runtime::Backend;

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
    /// The model on `device`, its weights placed to fit `max_vram` bytes.
    pub(super) fn new(device: &crate::Device, max_vram: u64, paths: &ModelPaths) -> Result<Self> {
        let cuda = matches!(device.backend, Backend::Cuda { .. });
        let context = Context::new(&ContextParams {
            model: Some(&paths.version_marker),
            diffusion_model: Some(&paths.diffusion_model),
            vae: Some(&paths.vae),
            clip_l: Some(&paths.clip_l),
            clip_g: Some(&paths.clip_g),
            device: Some(&device.name),
            max_vram: Some(max_vram),
            mmap: true,
            flash_attention: cuda,
            conv_direct: cuda,
            ..ContextParams::default()
        })
        .context("failed to load RORem mixed components")?;
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub(super) fn forward(&self, params: &ImageParams) -> Result<Vec<RgbImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("RORem mixed context lock was poisoned"))?;
        context
            .generate_image(params)
            .context("RORem mixed inference failed")
    }
}
