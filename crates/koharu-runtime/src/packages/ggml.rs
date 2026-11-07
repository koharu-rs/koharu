//! llama.cpp and stable-diffusion.cpp, the libraries Koharu runs on ggml,
//! and the devices ggml lists.

use std::{ffi::CStr, path::PathBuf};

use anyhow::Result;
use futures::future::BoxFuture;
use koharu_llama_sys::ggml_backend_dev_t;

use super::{Host, Package, Release};
use crate::{Backend, Device};

pub(super) struct Llama;

pub(super) struct Diffusion;

/// llama.cpp at the release `koharu-llama-sys`'s headers come from.
static LLAMA: Release = Release {
    repository: "koharu-rs/llama",
    package: "llama",
    tag: koharu_llama_sys::RELEASE,
};

/// stable-diffusion.cpp at the commit `koharu-diffusion-sys`'s header comes
/// from.
static DIFFUSION: Release = Release {
    repository: "koharu-rs/diffusion",
    package: "diffusion",
    tag: koharu_diffusion_sys::RELEASE,
};

impl Package for Llama {
    fn download(&self, host: Host, backend: Backend) -> BoxFuture<'_, Result<()>> {
        Box::pin(LLAMA.download(host, backend, |_| true))
    }

    fn load(&self, host: Host, gpu: &mut Device) -> Result<()> {
        let folder = LLAMA.folder(host, gpu.backend);
        let llama = folder.join(libloading::library_filename("llama"));
        let mtmd = folder.join(libloading::library_filename("mtmd"));
        let common = folder.join(libloading::library_filename("llama-common"));
        koharu_llama_sys::load(&[&llama, &mtmd, &common])?;
        listed(gpu)
    }

    fn versions(&self) -> Vec<PathBuf> {
        vec![LLAMA.version()]
    }
}

impl Package for Diffusion {
    fn download(&self, host: Host, backend: Backend) -> BoxFuture<'_, Result<()>> {
        Box::pin(DIFFUSION.download(host, backend, |_| true))
    }

    fn load(&self, host: Host, gpu: &mut Device) -> Result<()> {
        let library = DIFFUSION
            .folder(host, gpu.backend)
            .join(libloading::library_filename("stable-diffusion"));
        koharu_diffusion_sys::load(&[&library])?;
        Ok(())
    }

    fn versions(&self) -> Vec<PathBuf> {
        vec![DIFFUSION.version()]
    }
}

/// Gives `gpu` llama.cpp's description, once llama.cpp lists it at its index
/// under its name: a GPU is one llama.cpp found, at the index libtorch
/// addresses it by, so a Vulkan loader on disk without a Vulkan device
/// doesn't count.
fn listed(gpu: &mut Device) -> Result<()> {
    let listed = unsafe {
        (gpu.index < koharu_llama_sys::ggml_backend_dev_count())
            .then(|| describe(koharu_llama_sys::ggml_backend_dev_get(gpu.index)))
    };
    match listed {
        Some((name, description)) if name == gpu.name => {
            gpu.description = description;
            Ok(())
        }
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
