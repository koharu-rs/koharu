use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::Once,
};

use koharu_diffusion_sys as sys;

thread_local! {
    /// What sd.cpp warned of or failed on during this thread's `explained`
    /// call.
    static LOGGED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Sends sd.cpp's log to `tracing`, installed once for the process.
pub(crate) fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { sys::sd_set_log_callback(Some(log), ptr::null_mut()) });
}

/// `call`'s result, and what sd.cpp warned of or failed on as it ran on this
/// thread: sd.cpp's API answers only that a call failed, and its log says
/// why, such as the memory a step needed and the memory free.
pub(crate) fn explained<T>(call: impl FnOnce() -> T) -> (T, String) {
    LOGGED.set(Some(Vec::new()));
    let result = call();
    let lines = LOGGED.take().unwrap_or_default();
    let reason = match lines.is_empty() {
        true => "its log says why".to_owned(),
        false => lines.join("; "),
    };
    (result, reason)
}

unsafe extern "C" fn log(level: sys::sd_log_level_t, text: *const c_char, _: *mut c_void) {
    // A panic never unwinds into C.
    let _ = std::panic::catch_unwind(|| {
        if text.is_null() {
            return;
        }
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        let text = text.trim_end();
        if matches!(level, sys::SD_LOG_ERROR | sys::SD_LOG_WARN) {
            LOGGED.with_borrow_mut(|logged| {
                if let Some(logged) = logged {
                    logged.push(text.to_owned());
                }
            });
        }
        match level {
            sys::SD_LOG_ERROR => tracing::error!(target: "stable-diffusion.cpp", "{text}"),
            sys::SD_LOG_WARN => tracing::warn!(target: "stable-diffusion.cpp", "{text}"),
            sys::SD_LOG_INFO => tracing::info!(target: "stable-diffusion.cpp", "{text}"),
            _ => tracing::debug!(target: "stable-diffusion.cpp", "{text}"),
        }
    });
}
