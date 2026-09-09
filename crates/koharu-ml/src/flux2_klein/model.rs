//! Native FLUX.2 Klein model assembly.
//!
//! Component mapping follows stable-diffusion.cpp at:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/docs/flux2.md
//! Qwen3 chat templating and hidden-state layers 9, 18, and 27 are mapped by:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/conditioning/conditioner.hpp#L2287-L2301
//! The decoder width is inferred from the small-decoder checkpoint by:
//! https://github.com/leejet/stable-diffusion.cpp/blob/cc734292286f85f9c48305d94d7fd22f42838522/src/model/vae/auto_encoder_kl.hpp#L518-L569

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, anyhow, ensure};
use koharu_diffusion::{Context, ContextParams, ImageGenerationParams, RgbImage, VaeFormat};

use crate::Backend;

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
    pub fn new(device: &crate::Device, paths: ModelPaths) -> Result<Self> {
        let context = Context::new(&context_params(device, paths))
            .context("failed to load FLUX.2 Klein components")?;
        ensure!(
            context.supports_image_generation(),
            "the loaded FLUX.2 Klein context does not support image generation"
        );
        Ok(Self {
            context: Mutex::new(context),
        })
    }

    pub fn forward(&self, params: &ImageGenerationParams) -> Result<Vec<RgbImage>> {
        let mut context = self
            .context
            .lock()
            .map_err(|_| anyhow!("FLUX.2 Klein context lock was poisoned"))?;
        context
            .generate_image(params)
            .context("FLUX.2 Klein inference failed")
    }
}

fn context_params(device: &crate::Device, paths: ModelPaths) -> ContextParams {
    let use_accelerator = device.backend != Backend::Cpu;
    let use_cuda = device.backend == Backend::Cuda;
    let keep_parameters_resident = use_accelerator && device.memory_free >= 20 * 1024 * 1024 * 1024;
    ContextParams {
        diffusion_model_path: Some(paths.transformer),
        llm_path: Some(paths.text_encoder),
        vae_path: Some(paths.vae),
        enable_mmap: true,
        flash_attention: use_cuda,
        diffusion_flash_attention: use_cuda,
        diffusion_conv_direct: use_cuda,
        vae_conv_direct: use_cuda,
        vae_format: VaeFormat::Flux2,
        backend: Some(device.name.clone()),
        // The text encoder and denoiser run in separate phases. Cards with less
        // headroom keep source parameters in RAM; high-VRAM cards avoid repeated
        // staging by retaining both quantized models on the accelerator.
        params_backend: (use_accelerator && !keep_parameters_resident).then(|| "*=cpu".to_owned()),
        ..ContextParams::default()
    }
}
