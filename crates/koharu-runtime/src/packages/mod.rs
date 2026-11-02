//! The native packages a device needs, each pinned to a version.

mod cuda;
mod ggml;
mod rocm;
mod torch;

use std::{
    fs::{File, create_dir_all},
    io::copy,
    mem::forget,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use fast_glob::glob_match;
use flate2::read::GzDecoder;

use crate::{Backend, Device, Devices, store, transfer};

/// Why a candidate couldn't start, and whether another still can: once one
/// of a bound library's builds has loaded, the process keeps it, so no other
/// build of it can load.
pub(crate) struct Failure {
    pub(crate) error: anyhow::Error,
    pub(crate) last: bool,
}

/// Installs what `gpu` needs, then loads it in dependency order: CUDA or
/// ROCm, libtorch, llama.cpp, then stable-diffusion.cpp. Everything installs
/// before anything loads, so a transfer that fails leaves nothing loaded.
pub(crate) async fn start(gpu: &Device) -> Result<Devices, Failure> {
    let retry = |error| Failure { error, last: false };
    let host = Host::current().map_err(retry)?;
    let mut gpu = gpu.clone();
    let torch_backend = torch::backend(gpu.backend);
    let cuda = match gpu.backend {
        Backend::Cuda { .. } => Some(cuda::install(host).await.map_err(retry)?),
        _ => None,
    };
    let rocm = match gpu.backend {
        Backend::Rocm { target } => {
            Some((rocm::install(host, target).await.map_err(retry)?, target))
        }
        _ => None,
    };
    let torch = torch::install(host, torch_backend).await.map_err(retry)?;
    let llama = ggml::LLAMA
        .install(host, gpu.backend, |_| true)
        .await
        .map_err(retry)?;
    let diffusion = ggml::DIFFUSION
        .install(host, gpu.backend, |_| true)
        .await
        .map_err(retry)?;

    if let Some(cuda) = &cuda {
        cuda::load(cuda).map_err(retry)?;
    }
    if let Some((rocm, target)) = &rocm {
        gpu.index = rocm::load(rocm, target).map_err(retry)?;
        gpu.name = format!("ROCm{}", gpu.index);
    }
    let mut bound = false;
    let mut load = || -> Result<Devices> {
        torch::load(torch_backend, &torch)?;
        bound = true;
        ggml::load_llama(&llama)?;
        let ggml = ggml::listed(&gpu)?;
        let torch = if torch_backend == gpu.backend {
            ggml.clone()
        } else {
            ggml::cpu()?
        };
        ggml::load_diffusion(&diffusion)?;
        Ok(Devices { torch, ggml })
    };
    let devices = load().map_err(|error| Failure { error, last: bound })?;

    // CUDA's wheels install at their version folders, and the other packages
    // one level below theirs.
    let mut versions = cuda.unwrap_or_default();
    versions.extend(
        rocm.iter()
            .map(|(folder, _)| folder)
            .chain([&torch, &llama, &diffusion])
            .filter_map(|folder| folder.parent().map(Path::to_owned)),
    );
    store::remove_others(versions).await;
    Ok(devices)
}

/// The platforms Koharu ships native packages for, each named by the target
/// triple Koharu's own releases name their archives with.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumIter, strum::EnumProperty, strum::IntoStaticStr,
)]
enum Host {
    #[strum(
        serialize = "x86_64-pc-windows-msvc",
        props(os = "windows", arch = "x86_64")
    )]
    WindowsX64,
    #[strum(
        serialize = "x86_64-unknown-linux-gnu",
        props(os = "linux", arch = "x86_64")
    )]
    LinuxX64,
    #[strum(
        serialize = "aarch64-unknown-linux-gnu",
        props(os = "linux", arch = "aarch64")
    )]
    LinuxArm64,
    #[strum(
        serialize = "aarch64-apple-darwin",
        props(os = "macos", arch = "aarch64")
    )]
    MacosArm64,
}

impl Host {
    fn current() -> Result<Self> {
        use strum::{EnumProperty, IntoEnumIterator};
        Self::iter()
            .find(|host| {
                host.get_str("os") == Some(std::env::consts::OS)
                    && host.get_str("arch") == Some(std::env::consts::ARCH)
            })
            .context("Koharu has no native packages for this platform")
    }
}

/// One of Koharu's own builds of a native library, released on GitHub as
/// one archive per platform and backend.
struct Release {
    /// The GitHub repository the release is in, whose name the store uses.
    repository: &'static str,
    tag: &'static str,
}

impl Release {
    /// Installs the archive for `host` and `backend`, named
    /// `{triple}-{variant}.tar.gz`, which is complete when `complete` says so,
    /// and returns its folder.
    async fn install(
        &self,
        host: Host,
        backend: Backend,
        complete: impl Fn(&Path) -> bool + Send + Sync,
    ) -> Result<PathBuf> {
        let (triple, variant): (&str, &str) = (host.into(), backend.into());
        let archive = format!("{triple}-{variant}");
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

/// Fetches an archive into `stage`, on the store's own volume, and extracts
/// the files matching `patterns` off the async threads.
async fn install_archive(url: &str, stage: &Path, patterns: &'static [&'static str]) -> Result<()> {
    let name = url
        .rsplit('/')
        .next()
        .context("an archive URL has no name")?;
    let archive = stage.join(name);
    transfer::fetch(url, &archive).await?;
    let stage = stage.to_owned();
    tokio::task::spawn_blocking(move || {
        extract(&archive, &stage, patterns)?;
        std::fs::remove_file(&archive)?;
        Ok(())
    })
    .await?
}

/// The files an archive of libraries is extracted for.
const LIBRARIES: &[&str] = &["**/*.dll", "**/*.dylib", "**/*.so", "**/*.so.*"];

fn extract(archive: &Path, destination: &Path, patterns: &[&str]) -> Result<()> {
    let selected = |path: &Path| {
        patterns
            .iter()
            .any(|pattern| glob_match(pattern, path.to_string_lossy().as_bytes()))
    };
    match archive.extension().and_then(|extension| extension.to_str()) {
        Some("whl") => {
            let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index)?;
                let Some(path) = entry.enclosed_name() else {
                    continue;
                };
                if entry.is_dir() || !selected(&path) {
                    continue;
                }
                let output = destination.join(path);
                create_dir_all(output.parent().context("an archive entry has no parent")?)?;
                copy(&mut entry, &mut File::create(output)?)?;
            }
        }
        Some("gz") => {
            let mut archive = tar::Archive::new(GzDecoder::new(File::open(archive)?));
            for entry in archive.entries()? {
                let mut entry = entry?;
                if selected(&entry.path()?) {
                    entry.unpack_in(destination)?;
                }
            }
        }
        _ => anyhow::bail!("{} isn't an archive Koharu reads", archive.display()),
    }
    Ok(())
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
        unsafe {
            Library::load_with_flags(
                path,
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

/// The path `name` has inside `root`, wherever a wheel put it.
fn find(root: &Path, name: &str) -> Result<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| entry.file_type().is_file() && entry.file_name() == name)
        .map(walkdir::DirEntry::into_path)
        .with_context(|| format!("{} has no {name}", root.display()))
}
