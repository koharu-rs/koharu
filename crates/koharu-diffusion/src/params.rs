use std::path::Path;

use image::{GrayImage, RgbImage};

/// How the VAE lays out its latents; unset, sd.cpp reads it from the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaeFormat {
    Flux2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scheduler {
    Discrete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleMethod {
    Euler,
}

/// sd.cpp's step caches, which skip steps that change little.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cache {
    /// EasyCache, for DiT models: reuses a step's output while its input
    /// changes little.
    EasyCache,
    /// Spectrum: forecasts a step's output from the steps before it.
    Spectrum,
}

/// A model's files and Koharu's settings for it. Whatever is left unset keeps
/// sd.cpp's own default.
#[derive(Clone, Debug, Default)]
pub struct ContextParams<'a> {
    pub model: Option<&'a Path>,
    pub diffusion_model: Option<&'a Path>,
    pub vae: Option<&'a Path>,
    pub clip_l: Option<&'a Path>,
    pub clip_g: Option<&'a Path>,
    pub llm: Option<&'a Path>,
    /// The text encoder's vision projector, for a model whose text encoder
    /// reads its condition images, such as Qwen-Image 2.1's Qwen3-VL.
    pub llm_vision: Option<&'a Path>,
    pub vae_format: Option<VaeFormat>,
    /// The device to run on, by its ggml name, such as `CUDA0`.
    pub device: Option<&'a str>,
    /// Memory-maps the weight files instead of reading them in.
    pub mmap: bool,
}

/// One generation. Whatever is left unset keeps sd.cpp's own default.
#[derive(Clone, Debug, Default)]
pub struct ImageParams<'a> {
    pub prompt: &'a str,
    /// What to steer away from, with classifier-free guidance.
    pub negative_prompt: &'a str,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The image to start from, for inpainting.
    pub init: Option<&'a RgbImage>,
    /// Where to paint: white is repainted.
    pub mask: Option<&'a GrayImage>,
    pub references: &'a [RgbImage],
    pub reference_args: Option<&'a str>,
    /// Classifier-free guidance on the text.
    pub cfg: Option<f32>,
    pub scheduler: Option<Scheduler>,
    /// The flow-matching time shift; unset keeps the model's own.
    pub flow_shift: Option<f32>,
    pub sample_method: Option<SampleMethod>,
    pub steps: Option<u32>,
    /// The noise levels to step through, ending in zero, in place of `steps`
    /// and the scheduler's own; empty keeps them.
    pub sigmas: &'a [f32],
    /// Reuses work between steps that change little.
    pub cache: Option<Cache>,
    /// Encodes and decodes in square tiles this many pixels a side, which
    /// takes less memory; unset runs the VAE whole.
    pub vae_tile: Option<u32>,
    pub strength: Option<f32>,
    pub seed: Option<i64>,
    /// How many images to generate.
    pub count: Option<u32>,
}
