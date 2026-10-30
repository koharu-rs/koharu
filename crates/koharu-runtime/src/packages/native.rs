//! Koharu's own builds of native libraries, released on GitHub as one archive
//! per platform and backend.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{Host, LIBRARIES, install_archive};
use crate::{Backend, store};

pub(super) struct Release {
    /// The GitHub repository the release is in, whose name the store uses.
    pub(super) repository: &'static str,
    pub(super) tag: &'static str,
}

/// llama.cpp at the release `koharu-llama-sys`'s headers come from.
pub(super) static LLAMA: Release = Release {
    repository: "koharu-rs/llama",
    tag: koharu_llama_sys::RELEASE,
};

/// stable-diffusion.cpp at the commit `koharu-diffusion-sys`'s header comes
/// from.
pub(super) static DIFFUSION: Release = Release {
    repository: "koharu-rs/diffusion",
    tag: koharu_diffusion_sys::RELEASE,
};

impl Release {
    /// Installs the archive for `host` and `backend`, named
    /// `{triple}-{variant}.tar.gz`, which is complete when `complete` says so,
    /// and returns its folder.
    pub(super) async fn install(
        &self,
        host: Host,
        backend: Backend,
        complete: impl Fn(&Path) -> bool + Send + Sync,
    ) -> Result<PathBuf> {
        let variant = match backend {
            Backend::Cuda { .. } => "cuda",
            Backend::Rocm { .. } => "hip",
            Backend::Vulkan => "vulkan",
            Backend::Metal => "metal",
        };
        let archive = format!("{}-{variant}", host.triple());
        let url = format!(
            "https://github.com/{}/releases/download/{}/{archive}.tar.gz",
            self.repository, self.tag
        );
        let name = self
            .repository
            .rsplit('/')
            .next()
            .unwrap_or(self.repository);
        store::directory(
            store::root().join(name).join(self.tag).join(&archive),
            |folder| folder.is_dir() && complete(folder),
            move |stage| async move { install_archive(&url, &stage, LIBRARIES).await },
        )
        .await
    }
}

pub(super) fn load_llama(folder: &Path) -> Result<()> {
    let llama = folder.join(libloading::library_filename("llama"));
    let mtmd = folder.join(libloading::library_filename("mtmd"));
    koharu_llama_sys::load(&[&llama, &mtmd])?;
    Ok(())
}

pub(super) fn load_diffusion(folder: &Path) -> Result<()> {
    let library = folder.join(libloading::library_filename("stable-diffusion"));
    koharu_diffusion_sys::load(&[&library])?;
    // The header's structs must be the library's, so its commit must match.
    let commit = unsafe { std::ffi::CStr::from_ptr(koharu_diffusion_sys::sd_commit()) };
    let commit = commit.to_string_lossy();
    anyhow::ensure!(
        commit.starts_with(koharu_diffusion_sys::COMMIT),
        "stable-diffusion.cpp is at {commit}, but its bindings are for {}",
        koharu_diffusion_sys::COMMIT
    );
    Ok(())
}
