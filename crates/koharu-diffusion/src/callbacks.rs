use std::{
    ffi::{CStr, c_void},
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    slice,
    sync::{Arc, OnceLock, RwLock},
};

use crate::{
    Error, LogLevel, PreviewMode, Result, RgbImageView, ffi::configure_native,
    image::rgb_view_from_raw, sys,
};

/// A log message copied from the native callback buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMessage {
    pub level: LogLevel,
    pub text: String,
}

/// Sampling progress reported by stable-diffusion.cpp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub step: i32,
    pub steps: i32,
    /// Seconds per iteration, or the analogous native operation time.
    pub time: f32,
}

/// Preview frames borrowed from stable-diffusion.cpp for one callback invocation.
#[derive(Debug)]
pub struct Preview<'a> {
    pub step: i32,
    pub is_noisy: bool,
    pub frames: Vec<RgbImageView<'a>>,
}

/// Controls which intermediate previews stable-diffusion.cpp computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewOptions {
    pub mode: PreviewMode,
    pub interval: i32,
    pub denoised: bool,
    pub noisy: bool,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            mode: PreviewMode::None,
            interval: 1,
            denoised: true,
            noisy: false,
        }
    }
}

/// Opaque graph evaluation information passed by the backend.
#[derive(Debug, Clone, Copy)]
pub struct GraphEvaluation<'a> {
    tensor: NonNull<sys::ggml_tensor>,
    pub ask: bool,
    _lifetime: PhantomData<&'a mut sys::ggml_tensor>,
}

impl GraphEvaluation<'_> {
    /// Address of the opaque native tensor, useful for identity tracking.
    #[must_use]
    pub fn tensor_address(self) -> usize {
        self.tensor.as_ptr() as usize
    }
}

type LogCallback = dyn Fn(LogMessage) + Send + Sync + 'static;
type ProgressCallback = dyn Fn(Progress) + Send + Sync + 'static;
type PreviewCallback = dyn for<'a> Fn(Preview<'a>) + Send + Sync + 'static;
type GraphCallback = dyn for<'a> Fn(GraphEvaluation<'a>) -> bool + Send + Sync + 'static;

fn log_slot() -> &'static RwLock<Option<Arc<LogCallback>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<LogCallback>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

fn progress_slot() -> &'static RwLock<Option<Arc<ProgressCallback>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<ProgressCallback>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

fn preview_slot() -> &'static RwLock<Option<Arc<PreviewCallback>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<PreviewCallback>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

fn graph_slot() -> &'static RwLock<Option<Arc<GraphCallback>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<GraphCallback>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

fn read_callback<T: ?Sized>(slot: &RwLock<Option<Arc<T>>>) -> Option<Arc<T>> {
    slot.read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

unsafe extern "C" fn log_trampoline(
    level: sys::sd_log_level_t,
    text: *const std::os::raw::c_char,
    _data: *mut c_void,
) {
    let Some(callback) = read_callback(log_slot()) else {
        return;
    };
    let Ok(level) = LogLevel::try_from(level) else {
        return;
    };
    if text.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    let _ = catch_unwind(AssertUnwindSafe(|| callback(LogMessage { level, text })));
}

unsafe extern "C" fn progress_trampoline(step: i32, steps: i32, time: f32, _data: *mut c_void) {
    let Some(callback) = read_callback(progress_slot()) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        callback(Progress { step, steps, time });
    }));
}

unsafe extern "C" fn preview_trampoline(
    step: i32,
    frame_count: i32,
    frames: *mut sys::sd_image_t,
    is_noisy: bool,
    _data: *mut c_void,
) {
    let Some(callback) = read_callback(preview_slot()) else {
        return;
    };
    let Ok(frame_count) = usize::try_from(frame_count) else {
        return;
    };
    if frame_count > isize::MAX as usize / size_of::<sys::sd_image_t>()
        || (frames.is_null() && frame_count != 0)
    {
        return;
    }
    let raw_frames = if frame_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(frames, frame_count) }
    };
    let mut image_views = Vec::with_capacity(frame_count);
    for frame in raw_frames {
        let Ok(frame) = (unsafe { rgb_view_from_raw(frame) }) else {
            return;
        };
        image_views.push(frame);
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        callback(Preview {
            step,
            is_noisy,
            frames: image_views,
        });
    }));
}

unsafe extern "C" fn graph_trampoline(
    tensor: *mut sys::ggml_tensor,
    ask: bool,
    _data: *mut c_void,
) -> bool {
    let Some(callback) = read_callback(graph_slot()) else {
        return true;
    };
    let Some(tensor) = NonNull::new(tensor) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        callback(GraphEvaluation {
            tensor,
            ask,
            _lifetime: PhantomData,
        })
    }))
    .unwrap_or(false)
}

/// Replaces the process-wide native log callback.
pub fn set_log_callback(callback: impl Fn(LogMessage) + Send + Sync + 'static) -> Result<()> {
    let callback: Arc<LogCallback> = Arc::new(callback);
    configure_native(|| {
        *log_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(callback);
        unsafe { sys::sd_set_log_callback(Some(log_trampoline), std::ptr::null_mut()) };
    })
}

/// Redirects stable-diffusion.cpp logs to `tracing`.
pub fn send_logs_to_tracing() -> Result<()> {
    set_log_callback(|message| {
        let text = message.text.trim_end();
        match message.level {
            LogLevel::Debug | LogLevel::Verbose => {
                tracing::debug!(target: "stable-diffusion.cpp", "{text}")
            }
            LogLevel::Info => tracing::info!(target: "stable-diffusion.cpp", "{text}"),
            LogLevel::Warn => tracing::warn!(target: "stable-diffusion.cpp", "{text}"),
            LogLevel::Error => tracing::error!(target: "stable-diffusion.cpp", "{text}"),
        }
    })
}

/// Removes the process-wide native log callback.
pub fn clear_log_callback() -> Result<()> {
    configure_native(|| {
        unsafe { sys::sd_set_log_callback(None, std::ptr::null_mut()) };
        *log_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    })
}

/// Replaces the process-wide native progress callback.
pub fn set_progress_callback(callback: impl Fn(Progress) + Send + Sync + 'static) -> Result<()> {
    let callback: Arc<ProgressCallback> = Arc::new(callback);
    configure_native(|| {
        *progress_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(callback);
        unsafe { sys::sd_set_progress_callback(Some(progress_trampoline), std::ptr::null_mut()) };
    })
}

/// Removes the process-wide native progress callback.
pub fn clear_progress_callback() -> Result<()> {
    configure_native(|| {
        unsafe { sys::sd_set_progress_callback(None, std::ptr::null_mut()) };
        *progress_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    })
}

/// Configures and replaces the process-wide preview callback.
pub fn set_preview_callback<F>(options: PreviewOptions, callback: F) -> Result<()>
where
    F: for<'a> Fn(Preview<'a>) + Send + Sync + 'static,
{
    if options.interval <= 0 {
        return Err(Error::InvalidPreviewInterval);
    }
    let callback: Arc<PreviewCallback> = Arc::new(callback);
    configure_native(|| {
        *preview_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(callback);
        unsafe {
            sys::sd_set_preview_callback(
                Some(preview_trampoline),
                options.mode.as_raw(),
                options.interval,
                options.denoised,
                options.noisy,
                std::ptr::null_mut(),
            );
        }
    })
}

/// Disables native previews and removes the callback.
pub fn clear_preview_callback() -> Result<()> {
    configure_native(|| {
        unsafe {
            sys::sd_set_preview_callback(
                None,
                sys::PREVIEW_NONE,
                1,
                true,
                false,
                std::ptr::null_mut(),
            );
        }
        *preview_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    })
}

/// Replaces the process-wide backend graph-evaluation callback.
///
/// Enabling importance-matrix collection replaces this callback in the native
/// library. Install it again after the collector is dropped if both features
/// are used sequentially.
pub fn set_graph_evaluation_callback<F>(callback: F) -> Result<()>
where
    F: for<'a> Fn(GraphEvaluation<'a>) -> bool + Send + Sync + 'static,
{
    let callback: Arc<GraphCallback> = Arc::new(callback);
    configure_native(|| {
        *graph_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(callback);
        unsafe {
            sys::sd_set_backend_eval_callback(Some(graph_trampoline), std::ptr::null_mut());
        }
    })
}

/// Removes the process-wide backend graph-evaluation callback.
pub fn clear_graph_evaluation_callback() -> Result<()> {
    configure_native(|| {
        unsafe { sys::sd_set_backend_eval_callback(None, std::ptr::null_mut()) };
        *graph_slot()
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    })
}
