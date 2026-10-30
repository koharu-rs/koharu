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
    Flux2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleMethod {
    Euler,
}

/// A model's files and Koharu's settings for it. Whatever is left unset keeps
/// sd.cpp's own default, from `sd_ctx_params_init`.
#[derive(Clone, Debug, Default)]
pub struct ContextParams<'a> {
    pub model: Option<&'a Path>,
    pub diffusion_model: Option<&'a Path>,
    pub vae: Option<&'a Path>,
    pub clip_l: Option<&'a Path>,
    pub clip_g: Option<&'a Path>,
    pub llm: Option<&'a Path>,
    pub vae_format: Option<VaeFormat>,
    /// The device to run on, by its ggml name, such as `CUDA0`.
    pub device: Option<&'a str>,
    /// The memory the model may use on its device, in bytes; sd.cpp places
    /// its weights to fit it.
    pub max_vram: Option<u64>,
    /// Memory-maps the weight files instead of reading them in.
    pub mmap: bool,
    /// Flash attention, in the text encoder and the denoiser.
    pub flash_attention: bool,
    /// Direct convolution, in the denoiser and the VAE.
    pub conv_direct: bool,
}

/// One generation. Whatever is left unset keeps sd.cpp's own default, from
/// `sd_img_gen_params_init`.
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
    pub sample_method: Option<SampleMethod>,
    pub steps: Option<u32>,
    pub strength: Option<f32>,
    pub seed: Option<i64>,
    /// How many images to generate.
    pub count: Option<u32>,
}
