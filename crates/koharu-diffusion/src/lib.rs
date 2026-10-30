//! stable-diffusion.cpp for Koharu's diffusion models. The README is the
//! design.

mod context;
mod logging;
mod params;

use std::path::PathBuf;

pub use context::Context;
pub use params::{ContextParams, ImageParams, SampleMethod, Scheduler, VaeFormat};

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
