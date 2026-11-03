//! llama.cpp and stable-diffusion.cpp, the libraries Koharu runs on ggml,
//! and the devices ggml lists.

use std::{ffi::CStr, path::Path};

use anyhow::Result;
use koharu_llama_sys::ggml_backend_dev_t;

use super::Release;
use crate::Device;

/// llama.cpp at the release `koharu-llama-sys`'s headers come from.
pub(super) static LLAMA: Release = Release {
    repository: "koharu-rs/llama",
    package: "llama",
    tag: koharu_llama_sys::RELEASE,
};

/// stable-diffusion.cpp at the commit `koharu-diffusion-sys`'s header comes
/// from.
pub(super) static DIFFUSION: Release = Release {
    repository: "koharu-rs/diffusion",
    package: "diffusion",
    tag: koharu_diffusion_sys::RELEASE,
};

pub(super) fn load_llama(folder: &Path) -> Result<()> {
    let llama = folder.join(libloading::library_filename("llama"));
    let mtmd = folder.join(libloading::library_filename("mtmd"));
    koharu_llama_sys::load(&[&llama, &mtmd])?;
    Ok(())
}

pub(super) fn load_diffusion(folder: &Path) -> Result<()> {
    let library = folder.join(libloading::library_filename("stable-diffusion"));
    koharu_diffusion_sys::load(&[&library])?;
    Ok(())
}

/// `gpu` with llama.cpp's description, once llama.cpp lists it at its index
/// under its name: a GPU is one llama.cpp found, at the index libtorch
/// addresses it by, so a Vulkan loader on disk without a Vulkan device
/// doesn't count.
pub(super) fn listed(gpu: &Device) -> Result<Device> {
    let listed = unsafe {
        (gpu.index < koharu_llama_sys::ggml_backend_dev_count())
            .then(|| describe(koharu_llama_sys::ggml_backend_dev_get(gpu.index)))
    };
    match listed {
        Some((name, description)) if name == gpu.name => Ok(Device {
            description,
            ..gpu.clone()
        }),
        _ => anyhow::bail!("llama.cpp doesn't list {} at {}", gpu.name, gpu.index),
    }
}

/// A listed device's name and description.
unsafe fn describe(device: ggml_backend_dev_t) -> (String, String) {
    let text = |text| {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    unsafe {
        (
            text(koharu_llama_sys::ggml_backend_dev_name(device)),
            text(koharu_llama_sys::ggml_backend_dev_description(device)),
        )
    }
}
