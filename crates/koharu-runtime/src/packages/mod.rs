//! The native packages a device needs, each pinned to a version.

mod cuda;
mod native;
mod rocm;
mod torch;

use std::{
    ffi::CStr,
    fs::{File, create_dir_all},
    io::copy,
    mem::forget,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use fast_glob::glob_match;
use flate2::read::GzDecoder;

use crate::{Backend, Device, store, transfer};

/// Why a candidate couldn't start, and whether another still can: once one
/// of a bound library's builds has loaded, the process keeps it, so no other
/// build of it can load.
pub(crate) struct Failure {
    pub(crate) error: anyhow::Error,
    pub(crate) last: bool,
}

/// Installs what `device` needs, then loads it in dependency order: CUDA or
/// ROCm, libtorch, llama.cpp, then stable-diffusion.cpp. Everything installs
/// before anything loads, so a transfer that fails leaves nothing loaded.
pub(crate) async fn start(device: &Device) -> Result<Device, Failure> {
    let retry = |error| Failure { error, last: false };
    let host = Host::current().map_err(retry)?;
    let mut device = device.clone();
    let cuda = match device.backend {
        Backend::Cuda { .. } => Some(cuda::install(host).await.map_err(retry)?),
        _ => None,
    };
    let rocm = match device.backend {
        Backend::Rocm { target } => {
            Some((rocm::install(host, target).await.map_err(retry)?, target))
        }
        _ => None,
    };
    let torch = torch::install(host, device.backend).await.map_err(retry)?;
    let llama = native::LLAMA
        .install(host, device.backend, |_| true)
        .await
        .map_err(retry)?;
    let diffusion = native::DIFFUSION
        .install(host, device.backend, |_| true)
        .await
        .map_err(retry)?;

    if let Some(cuda) = &cuda {
        cuda::load(cuda).map_err(retry)?;
    }
    if let Some((rocm, target)) = &rocm {
        device.index = rocm::load(rocm, target).map_err(retry)?;
        device.name = format!("ROCm{}", device.index);
    }
    let mut bound = false;
    let mut load = || -> Result<()> {
        if let Some(torch) = &torch {
            torch::load(device.backend, torch)?;
            bound = true;
        }
        native::load_llama(&llama)?;
        bound = true;
        // A device is one llama.cpp found, at the index libtorch addresses it
        // by: a Vulkan loader on disk without a Vulkan device doesn't count.
        let listed = unsafe {
            (device.index < koharu_llama_sys::ggml_backend_dev_count()).then(|| {
                CStr::from_ptr(koharu_llama_sys::ggml_backend_dev_name(
                    koharu_llama_sys::ggml_backend_dev_get(device.index),
                ))
            })
        };
        ensure!(
            listed.is_some_and(|name| name.to_bytes() == device.name.as_bytes()),
            "llama.cpp doesn't list {} at {}",
            device.name,
            device.index
        );
        native::load_diffusion(&diffusion)
    };
    load().map_err(|error| Failure { error, last: bound })?;

    // CUDA's wheels install at their version folders, and the other packages
    // one level below theirs.
    let mut versions = cuda.unwrap_or_default();
    versions.extend(
        rocm.iter()
            .map(|(folder, _)| folder)
            .chain(&torch)
            .chain([&llama, &diffusion])
            .filter_map(|folder| folder.parent().map(Path::to_owned)),
    );
    store::remove_others(versions).await;
    Ok(device)
}

/// The platforms Koharu ships native packages for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Host {
    WindowsX64,
    LinuxX64,
    LinuxArm64,
    MacosArm64,
}

impl Host {
    fn current() -> Result<Self> {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Ok(Self::WindowsX64)
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Ok(Self::LinuxX64)
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            Ok(Self::LinuxArm64)
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Ok(Self::MacosArm64)
        } else {
            anyhow::bail!("Koharu has no native packages for this platform")
        }
    }

    /// The target triple Koharu's own releases name their archives with.
    fn triple(self) -> &'static str {
        match self {
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::LinuxX64 => "x86_64-unknown-linux-gnu",
            Self::LinuxArm64 => "aarch64-unknown-linux-gnu",
            Self::MacosArm64 => "aarch64-apple-darwin",
        }
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
