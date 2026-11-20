//! stable-diffusion.cpp for Koharu's diffusion models.

mod context;
mod options;
mod params;

use std::{ffi::CStr, path::PathBuf};

use koharu_diffusion_sys as sys;

pub use context::{Canceller, Context};
pub use params::{Cache, ContextParams, ImageParams, SampleMethod, Scheduler, VaeFormat};

/// A status stable-diffusion.cpp failed a call with, and the reason it gave.
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum StatusError {
    #[error("{0}")]
    InvalidArgument(String),
    #[error("{0}")]
    UnknownKey(String),
    #[error("{0}")]
    TypeMismatch(String),
    #[error("{0}")]
    OutOfRange(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    OutOfMemory(String),
    #[error("{0}")]
    LoadFailed(String),
    #[error("{0}")]
    GenerationFailed(String),
    #[error("{0}")]
    Busy(String),
    #[error("{0}")]
    Cancelled(String),
    #[error("status {0}: {1}")]
    Unknown(sys::sd_status_t, String),
}

/// The status sd.cpp failed this thread's last call with, by the reason it
/// keeps for that call until the next.
impl From<sys::sd_status_t> for StatusError {
    fn from(status: sys::sd_status_t) -> Self {
        let reason = unsafe { sys::sd_get_last_error() };
        let reason = match reason.is_null() {
            true => String::new(),
            false => unsafe { CStr::from_ptr(reason) }
                .to_string_lossy()
                .into_owned(),
        };
        match status {
            sys::SD_STATUS_INVALID_ARGUMENT => Self::InvalidArgument(reason),
            sys::SD_STATUS_UNKNOWN_KEY => Self::UnknownKey(reason),
            sys::SD_STATUS_TYPE_MISMATCH => Self::TypeMismatch(reason),
            sys::SD_STATUS_OUT_OF_RANGE => Self::OutOfRange(reason),
            sys::SD_STATUS_UNSUPPORTED => Self::Unsupported(reason),
            sys::SD_STATUS_OUT_OF_MEMORY => Self::OutOfMemory(reason),
            sys::SD_STATUS_LOAD_FAILED => Self::LoadFailed(reason),
            sys::SD_STATUS_GENERATION_FAILED => Self::GenerationFailed(reason),
            sys::SD_STATUS_BUSY => Self::Busy(reason),
            sys::SD_STATUS_CANCELLED => Self::Cancelled(reason),
            status => Self::Unknown(status, reason),
        }
    }
}

/// An option sd.cpp couldn't be given or wouldn't take.
#[derive(Debug, thiserror::Error)]
pub enum OptionError {
    #[error("{0} contains a NUL byte")]
    Nul(&'static str),
    #[error("{} isn't valid Unicode", .0.display())]
    PathToStr(PathBuf),
    #[error("stable-diffusion.cpp couldn't copy an image in")]
    Image,
    #[error("stable-diffusion.cpp refused an option")]
    Refused(#[from] StatusError),
}

/// Loading a model failed.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Option(#[from] OptionError),
    #[error("stable-diffusion.cpp couldn't load the model")]
    Status(#[from] StatusError),
    #[error("null result from stable-diffusion.cpp")]
    NullResult,
    #[error("stable-diffusion.cpp can't generate images with this model")]
    NoImages,
}

/// Generating images failed.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error(transparent)]
    Option(#[from] OptionError),
    #[error("stable-diffusion.cpp couldn't generate the images")]
    Status(#[from] StatusError),
    #[error("null result from stable-diffusion.cpp")]
    NullResult,
    #[error("stable-diffusion.cpp returned an image it can't have made")]
    Output,
}
