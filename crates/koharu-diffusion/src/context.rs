use std::{
    cell::Cell,
    fmt,
    marker::PhantomData,
    ptr::{self, NonNull},
    slice,
    sync::Arc,
};

use crate::{
    Audio, CancelMode, ContextParams, Error, ImageGenerationParams, Result, RgbImage, SampleMethod,
    Scheduler, Video, VideoGenerationParams, ffi::NativeCall, image::copy_rgb_from_raw, sys,
};

struct ContextInner {
    pointer: NonNull<sys::sd_ctx_t>,
}

// Context operations require `&mut Context`. The only operation exposed on a
// shared inner handle is native cancellation, which upstream implements using
// an atomic cancellation flag.
unsafe impl Send for ContextInner {}
unsafe impl Sync for ContextInner {}

impl Drop for ContextInner {
    fn drop(&mut self) {
        let _call = NativeCall::enter();
        unsafe { sys::free_sd_ctx(self.pointer.as_ptr()) };
    }
}

/// An owning stable-diffusion.cpp model context.
pub struct Context {
    inner: Arc<ContextInner>,
    // A context may move between threads, but normal operations are not
    // available concurrently through shared references.
    _not_sync: PhantomData<Cell<()>>,
}

impl fmt::Debug for Context {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Context")
            .field("pointer", &self.inner.pointer)
            .finish_non_exhaustive()
    }
}

impl Context {
    /// Loads a model context with the supplied components and backend settings.
    #[tracing::instrument(skip_all)]
    pub fn new(params: &ContextParams) -> Result<Self> {
        let native = params.to_native()?;
        let _call = NativeCall::enter();
        let pointer = unsafe { sys::new_sd_ctx(&raw const native.raw) };
        let pointer = NonNull::new(pointer).ok_or(Error::ContextCreationFailed)?;
        Ok(Self {
            inner: Arc::new(ContextInner { pointer }),
            _not_sync: PhantomData,
        })
    }

    #[must_use]
    pub fn supports_image_generation(&self) -> bool {
        let _call = NativeCall::enter();
        unsafe { sys::sd_ctx_supports_image_generation(self.inner.pointer.as_ptr()) }
    }

    #[must_use]
    pub fn supports_video_generation(&self) -> bool {
        let _call = NativeCall::enter();
        unsafe { sys::sd_ctx_supports_video_generation(self.inner.pointer.as_ptr()) }
    }

    /// Returns the native model-specific default sampling method.
    pub fn default_sample_method(&self) -> Result<SampleMethod> {
        let _call = NativeCall::enter();
        let raw = unsafe { sys::sd_get_default_sample_method(self.inner.pointer.as_ptr()) };
        SampleMethod::try_from(raw)
    }

    /// Returns the native model-specific scheduler default for a sampler.
    pub fn default_scheduler(&self, sample_method: SampleMethod) -> Result<Scheduler> {
        let _call = NativeCall::enter();
        let raw = unsafe {
            sys::sd_get_default_scheduler(self.inner.pointer.as_ptr(), sample_method.as_raw())
        };
        Scheduler::try_from(raw)
    }

    /// Generates one or more owned images.
    pub fn generate_image(&mut self, params: &ImageGenerationParams) -> Result<Vec<RgbImage>> {
        let native = params.to_native()?;
        let _call = NativeCall::enter();
        let mut output = RawImages::default();
        let succeeded = unsafe {
            sys::generate_image(
                self.inner.pointer.as_ptr(),
                &raw const native.raw,
                &raw mut output.pointer,
                &raw mut output.count,
            )
        };
        if !succeeded {
            return Err(Error::ImageGenerationFailed);
        }
        output.copy("image")
    }

    /// Generates owned video frames and optional audio.
    pub fn generate_video(&mut self, params: &VideoGenerationParams) -> Result<Video> {
        let native = params.to_native()?;
        let _call = NativeCall::enter();
        let mut frames = RawImages::default();
        let mut audio = RawAudio::default();
        let mut fps = 0;
        let succeeded = unsafe {
            sys::generate_video(
                self.inner.pointer.as_ptr(),
                &raw const native.raw,
                &raw mut frames.pointer,
                &raw mut frames.count,
                &raw mut audio.pointer,
                &raw mut fps,
            )
        };
        if !succeeded {
            return Err(Error::VideoGenerationFailed);
        }
        let fps =
            u32::try_from(fps)
                .ok()
                .filter(|fps| *fps > 0)
                .ok_or(Error::InvalidNativeOutput {
                    kind: "video frame rate",
                })?;
        Ok(Video {
            frames: frames.copy("video frame")?,
            audio: audio.copy()?,
            fps,
        })
    }

    /// Creates a thread-safe handle that can cancel generation from another thread.
    #[must_use]
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Changes the cancellation state directly.
    pub fn cancel(&self, mode: CancelMode) {
        self.cancel_handle().cancel(mode);
    }
}

/// A cloneable handle to the atomic native cancellation flag.
#[derive(Clone)]
pub struct CancelHandle {
    inner: Arc<ContextInner>,
}

impl fmt::Debug for CancelHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancelHandle")
            .finish_non_exhaustive()
    }
}

impl CancelHandle {
    pub fn cancel(&self, mode: CancelMode) {
        let _call = NativeCall::enter();
        unsafe { sys::sd_cancel_generation(self.inner.pointer.as_ptr(), mode.as_raw()) };
    }
}

#[derive(Debug)]
pub(crate) struct RawImages {
    pub(crate) pointer: *mut sys::sd_image_t,
    pub(crate) count: i32,
}

impl Default for RawImages {
    fn default() -> Self {
        Self {
            pointer: ptr::null_mut(),
            count: 0,
        }
    }
}

impl RawImages {
    pub(crate) fn copy(&self, kind: &'static str) -> Result<Vec<RgbImage>> {
        let count = usize::try_from(self.count).map_err(|_| Error::InvalidNativeOutput { kind })?;
        if count == 0 || self.pointer.is_null() {
            return Err(Error::InvalidNativeOutput { kind });
        }
        if count > isize::MAX as usize / size_of::<sys::sd_image_t>() {
            return Err(Error::InvalidNativeOutput { kind });
        }
        let raw_images = unsafe { slice::from_raw_parts(self.pointer, count) };
        raw_images
            .iter()
            .map(|raw| unsafe { copy_rgb_from_raw(raw) })
            .collect()
    }
}

impl Drop for RawImages {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            unsafe { sys::free_sd_images(self.pointer, self.count.max(0)) };
        }
    }
}

#[derive(Debug)]
struct RawAudio {
    pointer: *mut sys::sd_audio_t,
}

impl Default for RawAudio {
    fn default() -> Self {
        Self {
            pointer: ptr::null_mut(),
        }
    }
}

impl RawAudio {
    fn copy(&self) -> Result<Option<Audio>> {
        let Some(audio) = NonNull::new(self.pointer) else {
            return Ok(None);
        };
        Ok(Some(unsafe { Audio::copy_from_raw(audio.as_ref()) }?))
    }
}

impl Drop for RawAudio {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            unsafe { sys::free_sd_audio(self.pointer) };
        }
    }
}
