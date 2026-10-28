use std::ffi::{CStr, c_char};

use crate::{Result, TchError};

/// Takes a string the shim allocated, or nothing for a null pointer.
pub(crate) unsafe fn ptr_to_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let string = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { libc::free(ptr.cast()) };
    Some(string)
}

/// The error the last call on this thread left.
pub(crate) fn read_and_clean_error() -> Result<()> {
    match unsafe { ptr_to_string(koharu_torch_sys::get_and_reset_last_err()) } {
        None => Ok(()),
        Some(message) => Err(TchError::Torch(message)),
    }
}

/// Calls the shim and returns its error.
macro_rules! unsafe_torch_err {
    ($e:expr) => {{
        let v = unsafe { $e };
        crate::wrappers::utils::read_and_clean_error()?;
        v
    }};
}

/// Calls the shim for something that can't fail on a tensor that exists,
/// such as its shape or kind; every `Tensor` holds one.
macro_rules! unsafe_torch {
    ($e:expr) => {{
        let v = unsafe { $e };
        if let Err(error) = crate::wrappers::utils::read_and_clean_error() {
            panic!("libtorch failed where it can't: {error}");
        }
        v
    }};
}

/// Sets the seed libtorch's random numbers start from.
pub fn manual_seed(seed: i64) -> Result<()> {
    unsafe_torch_err!(koharu_torch_sys::at_manual_seed(seed));
    Ok(())
}
