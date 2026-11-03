//! The native packages a device needs, each pinned to a version.

mod archive;
mod cuda;
mod ggml;
mod host;
mod release;
mod rocm;
mod torch;

use std::{
    mem::forget,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use self::{
    archive::{LIBRARIES, install_archive},
    host::Host,
    release::Release,
};
use crate::{Backend, Device, store};

/// Installs what `gpu` needs, loads it, and removes the other versions the
/// store holds of what it installed.
pub(crate) async fn start(gpu: &Device) -> Result<Device> {
    let installed = Installed::install(Host::current()?, gpu.backend).await?;
    let device = installed.load(gpu)?;
    store::remove_others(installed.versions()).await;
    Ok(device)
}

/// The folders of the packages a GPU needs. Everything installs before
/// anything loads, so a transfer that fails leaves nothing loaded.
struct Installed {
    /// CUDA's wheels, for an NVIDIA GPU.
    cuda: Option<Vec<PathBuf>>,
    /// ROCm, for an AMD GPU.
    rocm: Option<PathBuf>,
    libtorch: PathBuf,
    /// Koharu's C shim to libtorch.
    shim: PathBuf,
    llama: PathBuf,
    diffusion: PathBuf,
}

impl Installed {
    async fn install(host: Host, backend: Backend) -> Result<Self> {
        let cuda = match backend {
            Backend::Cuda { .. } => Some(cuda::install(host).await?),
            _ => None,
        };
        let rocm = match backend {
            Backend::Rocm { target } => Some(rocm::install(host, target).await?),
            _ => None,
        };
        let (libtorch, shim) = torch::install(host, backend).await?;
        Ok(Self {
            cuda,
            rocm,
            libtorch,
            shim,
            llama: ggml::LLAMA.install(host, backend, |_| true).await?,
            diffusion: ggml::DIFFUSION.install(host, backend, |_| true).await?,
        })
    }

    /// Loads the packages in dependency order: CUDA or ROCm, libtorch,
    /// llama.cpp, then stable-diffusion.cpp. Returns `gpu` as llama.cpp lists
    /// it.
    fn load(&self, gpu: &Device) -> Result<Device> {
        let mut gpu = gpu.clone();
        if let Some(cuda) = &self.cuda {
            cuda::load(cuda)?;
        }
        if let (Some(rocm), Backend::Rocm { target }) = (&self.rocm, gpu.backend) {
            gpu.index = rocm::load(rocm, target)?;
            gpu.name = format!("ROCm{}", gpu.index);
        }
        torch::load(gpu.backend, &self.libtorch, &self.shim)?;
        ggml::load_llama(&self.llama)?;
        let device = ggml::listed(&gpu)?;
        ggml::load_diffusion(&self.diffusion)?;
        Ok(device)
    }

    /// The version folders installed: CUDA's wheels install at theirs, and
    /// the other packages one level below theirs.
    fn versions(self) -> Vec<PathBuf> {
        let mut versions = self.cuda.unwrap_or_default();
        versions.extend(
            self.rocm
                .iter()
                .chain([&self.libtorch, &self.shim, &self.llama, &self.diffusion])
                .filter_map(|folder| folder.parent().map(Path::to_owned)),
        );
        versions
    }
}

/// Loads a library that bound ones depend on (CUDA's, ROCm's, and libtorch's
/// own) for the life of the process; a bound library's own load finds it
/// already loaded.
fn preload(path: &Path, global: bool) -> Result<()> {
    #[cfg(windows)]
    let library: libloading::Library = {
        let _ = global;
        use libloading::os::windows::{
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library,
        };
        // The wheels' paths use forward slashes, but the loader finds the
        // library's folder, where its dependencies are, only in a path with
        // backslashes; rebuilt from its components, the path has them.
        let path: std::path::PathBuf = path.components().collect();
        unsafe {
            Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .with_context(|| format!("couldn't load {}", path.display()))?
        .into()
    };
    #[cfg(unix)]
    let library: libloading::Library = {
        use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_LOCAL, RTLD_NOW};
        let visibility = if global { RTLD_GLOBAL } else { RTLD_LOCAL };
        unsafe { Library::open(Some(path), RTLD_NOW | visibility) }
            .with_context(|| format!("couldn't load {}", path.display()))?
            .into()
    };
    forget(library);
    Ok(())
}
