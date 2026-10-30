use std::{
    ffi::{CString, c_char},
    mem::MaybeUninit,
    path::Path,
    ptr::{self, NonNull},
    slice,
};

use image::RgbImage;
use koharu_diffusion_sys as sys;

use crate::{
    ContextParams, Error, ImageParams, Result, SampleMethod, Scheduler, VaeFormat, logging,
};

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
        logging::install();
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

        let mut output = Images(ptr::null_mut());
        let generated = unsafe { sys::generate_image(self.pointer.as_ptr(), &raw, &mut output.0) };
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
struct Images(*mut sys::sd_image_generation_result_t);

impl Images {
    fn copy(&self) -> Result<Vec<RgbImage>> {
        // sd.cpp sets the result when it succeeds.
        let result = unsafe { self.0.as_ref() }.ok_or(Error::Output)?;
        let count = usize::try_from(result.count).map_err(|_| Error::Output)?;
        if result.items.is_null() || count == 0 {
            return Err(Error::Output);
        }
        unsafe { slice::from_raw_parts(result.items, count) }
            .iter()
            .map(|item| {
                let image = &item.image;
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
        if !self.0.is_null() {
            unsafe { sys::free_sd_image_generation_result(self.0) };
        }
    }
}
