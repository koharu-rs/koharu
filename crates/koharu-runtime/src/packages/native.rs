//! Koharu's own builds of native libraries, released on GitHub as one archive
//! per platform and backend and pinned to the SHA-256 GitHub records for each.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Host, LIBRARIES, install_archive};
use crate::{
    Backend,
    store::{Store, remove_others},
};

pub(super) struct Release {
    /// The GitHub repository the release is in, whose name the store uses.
    pub(super) repository: &'static str,
    pub(super) tag: &'static str,
    /// Each archive, named `{triple}-{variant}.tar.gz`, and its SHA-256.
    pub(super) archives: &'static [(&'static str, &'static str)],
}

/// llama.cpp at b11565, the build `llama-cpp-sys-2`'s headers come from.
pub(super) static LLAMA: Release = Release {
    repository: "koharu-rs/llama",
    tag: "b11565",
    archives: &[
        (
            "aarch64-apple-darwin-metal",
            "917ae0ddfd97f1a518293d851554b14273cd42a0027c5afae3900fb94d435a59",
        ),
        (
            "aarch64-unknown-linux-gnu-cuda",
            "7d6a4c634e3712750fbe14027b47623e8f713d6199357ef7523893bfc83d6651",
        ),
        (
            "x86_64-pc-windows-msvc-cuda",
            "c59300d8556c13ea1fc32f057af33832ca128425f714b27a7477d71571e2dae8",
        ),
        (
            "x86_64-pc-windows-msvc-hip",
            "c99b9ddf6d9a3d995afdb5005db4b27a023c28630eb478bb49d8419efa62d099",
        ),
        (
            "x86_64-pc-windows-msvc-vulkan",
            "c49f6d5615d6c08465e0486871aec956d397aafdb4712c57cc7f036ad6d6c954",
        ),
        (
            "x86_64-unknown-linux-gnu-cuda",
            "04ecda3be421f946c1033911b608bf870532968c0005db433b088a9ae371caff",
        ),
        (
            "x86_64-unknown-linux-gnu-hip",
            "6816ec5cc2443bf1ef5dd8f6d64bd7f8a743734d57d5aa1d8db0608a8fb81075",
        ),
        (
            "x86_64-unknown-linux-gnu-vulkan",
            "a9c24500dd27d663b32f3da5e300097ebf6a4d559d136b456de975cacb2b466f",
        ),
    ],
};

/// stable-diffusion.cpp at 1d0a58a, the commit `koharu-diffusion-sys`'s
/// header comes from.
pub(super) static DIFFUSION: Release = Release {
    repository: "koharu-rs/diffusion",
    tag: "master-964-1d0a58a.1",
    archives: &[
        (
            "aarch64-apple-darwin-metal",
            "b0056f907aea8ae0186e776304d4fcb0566d6aa4cbf1d8efa5e2c0dcd68e8429",
        ),
        (
            "aarch64-unknown-linux-gnu-cuda",
            "c6ef216f8b6d3b1174b26b59f1afc6e6fe331a1df100699fd910b571cd492d2a",
        ),
        (
            "x86_64-pc-windows-msvc-cuda",
            "723f51d0f84e55b2b12978dc25c2706a0f2e7af99da0de1b99876446d1ec424e",
        ),
        (
            "x86_64-pc-windows-msvc-hip",
            "d6e2367d2c719b4844eedb9d0ca6badb6e5e95ef6bbeeaa5329b3ac74eb344d8",
        ),
        (
            "x86_64-pc-windows-msvc-vulkan",
            "17fedf5e8db42beaacccee5a5695d537b0a9d1d86fc2b2fa0a03aa2d134f862a",
        ),
        (
            "x86_64-unknown-linux-gnu-cuda",
            "2e57614d5b5edc71cb475bad5bc26e59fbc6b294ebf0313b330f91fdbd97552d",
        ),
        (
            "x86_64-unknown-linux-gnu-hip",
            "92258d8ddb20d022d807fa2de913d320473404a76aec3c725110848a19a8ab87",
        ),
        (
            "x86_64-unknown-linux-gnu-vulkan",
            "1fd7c2ef45db0635fde1caa010ff8430f5ce673be860a2b9fe7d45b4dd7977aa",
        ),
    ],
};

impl Release {
    /// Installs the archive for `host` and `backend` and returns its folder.
    pub(super) async fn install(&self, host: Host, backend: Backend) -> Result<PathBuf> {
        let variant = match backend {
            Backend::Cuda => "cuda",
            Backend::Rocm => "hip",
            Backend::Vulkan => "vulkan",
            Backend::Metal => "metal",
        };
        self.install_variant(host, variant, |_| true).await
    }

    /// Installs the archive for `host` and `variant`, which is complete when
    /// `complete` says so.
    pub(super) async fn install_variant(
        &self,
        host: Host,
        variant: &str,
        complete: impl Fn(&Path) -> bool + Send + Sync,
    ) -> Result<PathBuf> {
        let archive = format!("{}-{variant}", host.triple());
        let (_, sha256) = self
            .archives
            .iter()
            .find(|(name, _)| *name == archive)
            .with_context(|| format!("{} {} has no {archive}", self.repository, self.tag))?;
        let url = format!(
            "https://github.com/{}/releases/download/{}/{archive}.tar.gz",
            self.repository, self.tag
        );
        Store::directory(
            self.folder().join(self.tag).join(&archive),
            |folder| folder.is_dir() && complete(folder),
            move |stage| async move { install_archive(&url, sha256, &stage, LIBRARIES).await },
        )
        .await
    }

    fn folder(&self) -> PathBuf {
        let name = self
            .repository
            .rsplit('/')
            .next()
            .unwrap_or(self.repository);
        Store::root().join(name)
    }

    pub(super) async fn remove_older(&self) {
        remove_others(self.folder(), String::new(), self.tag.to_owned()).await;
    }
}

pub(super) fn load_llama(folder: &Path) -> Result<()> {
    let llama = folder.join(libloading::library_filename("llama"));
    let mtmd = folder.join(libloading::library_filename("mtmd"));
    llama_cpp_sys_2::load(&[&llama, &mtmd])?;
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
