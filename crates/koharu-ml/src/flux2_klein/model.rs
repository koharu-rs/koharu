//! Native FLUX.2 Klein model assembly.
//!
//! Component mapping follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/docs/flux2.md
//! Qwen3 chat templating and hidden-state layers 9, 18, and 27 are mapped by:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/conditioning/conditioner.hpp#L2287-L2301
//! The decoder width is inferred from the small-decoder checkpoint by:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/model/vae/auto_encoder_kl.hpp#L518-L569

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow};
use image::RgbImage;
use koharu_diffusion::{Context, ContextParams, ImageParams, VaeFormat};
use koharu_runtime::Backend;

#[derive(Debug)]
pub(super) struct ModelPaths {
    pub transformer: PathBuf,
    pub text_encoder: PathBuf,
    pub vae: PathBuf,
}

#[derive(Debug)]
pub(super) struct Model {
    context: Mutex<Context>,
}

impl Model {
    /// The model on `device`, its weights placed to fit `max_vram` bytes.
    pub fn new(device: &crate::Device, max_vram: u64, paths: &ModelPaths) -> Result<Self> {
        let cuda = matches!(device.backend, Backend::Cuda { .. });
        let context = Context::new(&ContextParams {
            diffusion_model: Some(&paths.transformer),
            llm: Some(&paths.text_encoder),
            vae: Some(&paths.vae),
            vae_format: Some(VaeFormat::Flux2),
            device: Some(&device.name),
            max_vram: Some(max_vram),
            mmap: true,
            flash_attention: cuda,
            conv_direct: cuda,
            ..ContextParams::default()
        })
        .context("failed to load FLUX.2 Klein components")?;
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub fn forward(&self, params: &ImageParams) -> Result<Vec<RgbImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("FLUX.2 Klein context lock was poisoned"))?;
        context
            .generate_image(params)
            .context("FLUX.2 Klein inference failed")
    }
}
