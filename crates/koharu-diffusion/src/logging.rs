use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::Once,
};

use koharu_diffusion_sys as sys;

/// Sends sd.cpp's log to `tracing`, installed once for the process.
pub(crate) fn install() {
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
