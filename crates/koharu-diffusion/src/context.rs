use std::{
    ffi::{CStr, c_char, c_void},
    ptr::{self, NonNull},
    slice,
    sync::{Arc, Once},
};

use image::{DynamicImage, RgbImage, RgbaImage};
use koharu_diffusion_sys as sys;

use crate::{ContextParams, GenerateError, ImageParams, LoadError, StatusError, options::Options};

/// sd.cpp's log goes to `tracing` from the first model loaded on: sd.cpp
/// keeps one log callback for the process.
static LOG: Once = Once::new();

/// A loaded model. sd.cpp keeps its last few prompt encodings
/// (`conditioning_cache_size`, 4 by default), so the same prompt for every
/// crop is encoded once.
#[derive(Debug)]
pub struct Context {
    context: Arc<Raw>,
}

/// Stops the generation its model is running, from any thread.
#[derive(Clone, Debug)]
pub struct Canceller(Arc<Raw>);

/// sd.cpp's context, freed once neither its model nor a canceller holds it.
#[derive(Debug)]
struct Raw(NonNull<sys::sd_ctx_t>);

// Generating takes the model whole, and a canceller only sets sd.cpp's
// atomic cancel flag, so the context may be shared between threads.
unsafe impl Send for Raw {}
unsafe impl Sync for Raw {}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe { sys::sd_ctx_free(self.0.as_ptr()) };
    }
}

impl Canceller {
    /// Stops the model's generation as soon as sd.cpp can, which then fails
    /// as cancelled. sd.cpp clears the request as a generation starts, so it
    /// stops only one under way.
    pub fn cancel(&self) {
        unsafe { sys::sd_cancel_generation(self.0.0.as_ptr(), sys::SD_CANCEL_ALL) };
    }
}

impl Context {
    pub fn canceller(&self) -> Canceller {
        Canceller(Arc::clone(&self.context))
    }

    pub fn new(params: &ContextParams) -> Result<Self, LoadError> {
        LOG.call_once(|| unsafe { sys::sd_set_log_callback(Some(log), ptr::null_mut()) });
        let options = Options::context().ok_or(LoadError::NullResult)?;
        options.set("model_path", params.model)?;
        options.set("diffusion_model_path", params.diffusion_model)?;
        options.set("vae_path", params.vae)?;
        options.set("clip_l_path", params.clip_l)?;
        options.set("clip_g_path", params.clip_g)?;
        options.set("llm_path", params.llm)?;
        options.set("llm_vision_path", params.llm_vision)?;
        options.set("vae_format", params.vae_format)?;
        options.set("backend", params.device)?;
        options.set("enable_mmap", params.mmap)?;
        // Weights load now, so a model that doesn't fit fails here rather
        // than partway through a page.
        options.set("eager_load", true)?;

        let mut context = ptr::null_mut();
        match unsafe { sys::sd_ctx_create(options.as_ptr(), &mut context) } {
            sys::SD_STATUS_OK => {}
            status => return Err(StatusError::from(status).into()),
        }
        let context = Self {
            context: Arc::new(Raw(NonNull::new(context).ok_or(LoadError::NullResult)?)),
        };
        if !unsafe { sys::sd_ctx_supports_image_generation(context.context.0.as_ptr()) } {
            return Err(LoadError::NoImages);
        }
        Ok(context)
    }

    /// The images generated, RGB or RGBA as the model makes them.
    pub fn generate_image(
        &mut self,
        params: &ImageParams,
    ) -> Result<Vec<DynamicImage>, GenerateError> {
        let options = Options::image().ok_or(GenerateError::NullResult)?;
        options.set("prompt", params.prompt)?;
        options.set("negative_prompt", params.negative_prompt)?;
        options.set("width", params.width)?;
        options.set("height", params.height)?;
        options.set("init_image", params.init)?;
        options.set("mask_image", params.mask)?;
        options.set("ref_images", params.references)?;
        options.set("ref_image_args", params.reference_args)?;
        options.set("sample.guidance.txt_cfg", params.cfg)?;
        options.set("sample.scheduler", params.scheduler)?;
        options.set("sample.flow_shift", params.flow_shift)?;
        options.set("sample.method", params.sample_method)?;
        options.set("sample.steps", params.steps)?;
        options.set("sample.sigmas", params.sigmas)?;
        options.set("strength", params.strength)?;
        options.set("seed", params.seed)?;
        options.set("batch_count", params.count)?;
        options.set("cache.mode", params.cache)?;
        options.set("vae_tiling.enabled", params.vae_tile.map(|_| true))?;
        options.set("vae_tiling.tile_size_w", params.vae_tile)?;
        options.set("vae_tiling.tile_size_h", params.vae_tile)?;

        let mut result = ptr::null_mut();
        match unsafe {
            sys::sd_generate_image(self.context.0.as_ptr(), options.as_ptr(), &mut result)
        } {
            sys::SD_STATUS_OK => {}
            status => return Err(StatusError::from(status).into()),
        }
        Generated(NonNull::new(result).ok_or(GenerateError::NullResult)?).images()
    }
}

/// A generation's result, which owns its images until it's freed.
struct Generated(NonNull<sys::sd_result_t>);

impl Generated {
    /// Its images, copied out.
    fn images(&self) -> Result<Vec<DynamicImage>, GenerateError> {
        let count = unsafe { sys::sd_result_get_image_count(self.0.as_ptr()) };
        if count == 0 {
            return Err(GenerateError::Output);
        }
        (0..count)
            .map(|index| {
                copied(unsafe { sys::sd_result_get_image(self.0.as_ptr(), index) })
                    .ok_or(GenerateError::Output)
            })
            .collect()
    }
}

impl Drop for Generated {
    fn drop(&mut self) {
        unsafe { sys::sd_result_free(self.0.as_ptr()) };
    }
}

/// A result's image, copied out; `None` when it isn't the `[channels, width,
/// height]` RGB or RGBA image sd.cpp makes.
fn copied(tensor: *const sys::sd_tensor_t) -> Option<DynamicImage> {
    if tensor.is_null()
        || unsafe { sys::sd_tensor_type(tensor) } != sys::SD_TENSOR_U8 as sys::sd_tensor_type_t
        || unsafe { sys::sd_tensor_rank(tensor) } != 3
    {
        return None;
    }
    let [channels, width, height] =
        [0, 1, 2].map(|axis| u32::try_from(unsafe { sys::sd_tensor_dim(tensor, axis) }).ok());
    let (channels, width, height) = (channels?, width?, height?);
    let data = unsafe { sys::sd_tensor_data(tensor) }.cast::<u8>();
    if data.is_null() {
        return None;
    }
    let length = unsafe { sys::sd_tensor_byte_size(tensor) };
    let pixels = unsafe { slice::from_raw_parts(data, length) }.to_vec();
    match channels {
        3 => RgbImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8),
        4 => RgbaImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgba8),
        _ => None,
    }
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
