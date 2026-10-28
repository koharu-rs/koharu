//! stable-diffusion.cpp for Koharu's diffusion models. The README is the
//! design.

use std::{
    ffi::{CStr, CString, c_char, c_void},
    mem::MaybeUninit,
    path::{Path, PathBuf},
    ptr::{self, NonNull},
    slice,
    sync::Once,
};

use image::{GrayImage, RgbImage};
use koharu_diffusion_sys as sys;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} contains a NUL byte")]
    Nul(&'static str),
    #[error("{} isn't valid Unicode", .0.display())]
    NonUnicodePath(PathBuf),
    #[error("{0} is out of range for stable-diffusion.cpp")]
    OutOfRange(&'static str),
    #[error("stable-diffusion.cpp couldn't load the model; its log says why")]
    Load,
    #[error("stable-diffusion.cpp couldn't generate the image; its log says why")]
    Generate,
    #[error("stable-diffusion.cpp returned an image it can't have made")]
    Output,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

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

/// A loaded model. sd.cpp keeps its last few prompt encodings
/// (`conditioning_cache_size`, 4 by default), so the same prompt for every
/// crop is encoded once.
#[derive(Debug)]
pub struct Context {
    pointer: NonNull<sys::sd_ctx_t>,
}

// A context may move between threads; every call takes `&mut self`.
unsafe impl Send for Context {}

const GIB: f64 = (1u64 << 30) as f64;

impl Context {
    pub fn new(params: &ContextParams) -> Result<Self> {
        install_logging();
        let mut raw = unsafe {
            let mut raw = MaybeUninit::<sys::sd_ctx_params_t>::zeroed();
            sys::sd_ctx_params_init(raw.as_mut_ptr());
            raw.assume_init()
        };
        // The strings sd.cpp reads while it loads.
        let mut strings = Vec::new();
        let mut path = |path: Option<&Path>, field: &'static str| -> Result<*const c_char> {
            let Some(path) = path else {
                return Ok(ptr::null());
            };
            let text = path
                .to_str()
                .ok_or_else(|| Error::NonUnicodePath(path.to_owned()))?;
            strings.push(CString::new(text).map_err(|_| Error::Nul(field))?);
            Ok(strings.last().map_or(ptr::null(), |text| text.as_ptr()))
        };
        raw.model_path = path(params.model, "the model path")?;
        raw.diffusion_model_path = path(params.diffusion_model, "the diffusion model path")?;
        raw.vae_path = path(params.vae, "the VAE path")?;
        raw.clip_l_path = path(params.clip_l, "the CLIP-L path")?;
        raw.clip_g_path = path(params.clip_g, "the CLIP-G path")?;
        raw.llm_path = path(params.llm, "the text encoder path")?;
        let device = params
            .device
            .map(|device| CString::new(device).map_err(|_| Error::Nul("the device")))
            .transpose()?;
        if let Some(device) = &device {
            raw.backend = device.as_ptr();
        }
        // sd.cpp reads the budget as GiB.
        let max_vram = params
            .max_vram
            .map(|bytes| CString::new(format!("{:.3}", bytes as f64 / GIB)))
            .transpose()
            .map_err(|_| Error::Nul("the memory budget"))?;
        if let Some(max_vram) = &max_vram {
            raw.max_vram = max_vram.as_ptr();
        }
        if params.vae_format == Some(VaeFormat::Flux2) {
            raw.vae_format = sys::SD_VAE_FORMAT_FLUX2;
        }
        raw.enable_mmap = params.mmap;
        raw.flash_attn = params.flash_attention;
        raw.diffusion_flash_attn = params.flash_attention;
        raw.diffusion_conv_direct = params.conv_direct;
        raw.vae_conv_direct = params.conv_direct;
        // Weights load now, so a model that doesn't fit fails here rather
        // than partway through a page.
        raw.eager_load = true;

        let pointer = NonNull::new(unsafe { sys::new_sd_ctx(&raw) }).ok_or(Error::Load)?;
        let context = Self { pointer };
        if !unsafe { sys::sd_ctx_supports_image_generation(pointer.as_ptr()) } {
            return Err(Error::Load);
        }
        Ok(context)
    }

    pub fn generate_image(&mut self, params: &ImageParams) -> Result<Vec<RgbImage>> {
        let mut raw = unsafe {
            let mut raw = MaybeUninit::<sys::sd_img_gen_params_t>::zeroed();
            sys::sd_img_gen_params_init(raw.as_mut_ptr());
            raw.assume_init()
        };
        let prompt = CString::new(params.prompt).map_err(|_| Error::Nul("the prompt"))?;
        raw.prompt = prompt.as_ptr();
        let negative_prompt =
            CString::new(params.negative_prompt).map_err(|_| Error::Nul("the negative prompt"))?;
        raw.negative_prompt = negative_prompt.as_ptr();
        let reference_args = params
            .reference_args
            .map(|args| CString::new(args).map_err(|_| Error::Nul("the reference arguments")))
            .transpose()?;
        if let Some(args) = &reference_args {
            raw.ref_image_args = args.as_ptr();
        }
        if let Some(width) = params.width {
            raw.width = int(width, "the width")?;
        }
        if let Some(height) = params.height {
            raw.height = int(height, "the height")?;
        }
        if let Some(init) = params.init {
            raw.init_image = image(init.width(), init.height(), 3, init.as_raw());
        }
        if let Some(mask) = params.mask {
            raw.mask_image = image(mask.width(), mask.height(), 1, mask.as_raw());
        }
        let mut references = params
            .references
            .iter()
            .map(|reference| image(reference.width(), reference.height(), 3, reference.as_raw()))
            .collect::<Vec<_>>();
        if !references.is_empty() {
            raw.ref_images = references.as_mut_ptr();
            raw.ref_images_count = int(references.len(), "the reference count")?;
        }
        if let Some(cfg) = params.cfg {
            raw.sample_params.guidance.txt_cfg = cfg;
        }
        if let Some(scheduler) = params.scheduler {
            raw.sample_params.scheduler = match scheduler {
                Scheduler::Discrete => sys::DISCRETE_SCHEDULER,
                Scheduler::Flux2 => sys::FLUX2_SCHEDULER,
            };
        }
        if let Some(SampleMethod::Euler) = params.sample_method {
            raw.sample_params.sample_method = sys::EULER_SAMPLE_METHOD;
        }
        if let Some(steps) = params.steps {
            raw.sample_params.sample_steps = int(steps, "the step count")?;
        }
        if let Some(strength) = params.strength {
            raw.strength = strength;
        }
        if let Some(seed) = params.seed {
            raw.seed = seed;
        }
        if let Some(count) = params.count {
            raw.batch_count = int(count, "the image count")?;
        }

        let mut output = Images {
            pointer: ptr::null_mut(),
            count: 0,
        };
        let generated = unsafe {
            sys::generate_image(
                self.pointer.as_ptr(),
                &raw,
                &mut output.pointer,
                &mut output.count,
            )
        };
        if !generated {
            return Err(Error::Generate);
        }
        output.copy()
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { sys::free_sd_ctx(self.pointer.as_ptr()) };
    }
}

fn int(value: impl TryInto<i32>, field: &'static str) -> Result<i32> {
    value.try_into().map_err(|_| Error::OutOfRange(field))
}

/// An input image, borrowed: sd.cpp only reads its pixels, though the C
/// struct predates `const` data.
fn image(width: u32, height: u32, channel: u32, pixels: &[u8]) -> sys::sd_image_t {
    sys::sd_image_t {
        width,
        height,
        channel,
        data: pixels.as_ptr().cast_mut(),
    }
}

/// The images sd.cpp allocated, freed by sd.cpp's own function so the C
/// runtime that allocated them frees them.
struct Images {
    pointer: *mut sys::sd_image_t,
    count: i32,
}

impl Images {
    fn copy(&self) -> Result<Vec<RgbImage>> {
        let count = usize::try_from(self.count).map_err(|_| Error::Output)?;
        if self.pointer.is_null() || count == 0 {
            return Err(Error::Output);
        }
        unsafe { slice::from_raw_parts(self.pointer, count) }
            .iter()
            .map(|image| {
                if image.channel != 3 || image.data.is_null() {
                    return Err(Error::Output);
                }
                let length = (image.width as usize)
                    .checked_mul(image.height as usize)
                    .and_then(|pixels| pixels.checked_mul(3))
                    .ok_or(Error::Output)?;
                let pixels = unsafe { slice::from_raw_parts(image.data, length) }.to_vec();
                RgbImage::from_raw(image.width, image.height, pixels).ok_or(Error::Output)
            })
            .collect()
    }
}

impl Drop for Images {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            unsafe { sys::free_sd_images(self.pointer, self.count.max(0)) };
        }
    }
}

/// Sends sd.cpp's log to `tracing`, installed once for the process.
fn install_logging() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { sys::sd_set_log_callback(Some(log), ptr::null_mut()) });
}

unsafe extern "C" fn log(level: sys::sd_log_level_t, text: *const c_char, _: *mut c_void) {
    // A panic never unwinds into C.
    let _ = std::panic::catch_unwind(|| {
        if text.is_null() {
            return;
        }
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        let text = text.trim_end();
        match level {
            sys::SD_LOG_ERROR => tracing::error!(target: "stable-diffusion.cpp", "{text}"),
            sys::SD_LOG_WARN => tracing::warn!(target: "stable-diffusion.cpp", "{text}"),
            sys::SD_LOG_INFO => tracing::info!(target: "stable-diffusion.cpp", "{text}"),
            _ => tracing::debug!(target: "stable-diffusion.cpp", "{text}"),
        }
    });
}
