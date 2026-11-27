//! Gets Koharu ready to run models on this machine's GPU.

mod accelerator_lock;
mod device;
mod hardware;
mod huggingface;
mod packages;
mod store;
mod transfer;
mod turn;

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use tokio::sync::OnceCell;

pub use accelerator_lock::{AcceleratorLock, AcceleratorLocked};
pub use device::{Backend, Device, Memory};
pub use huggingface::{PinnedFile, Repository};
pub use transfer::{
    Cancelled, DownloadEntry, DownloadHandle, DownloadManager, DownloadState, downloads,
};
pub use turn::{Library, Turn};

/// Koharu's root: the folder setup put it in, in Windows release builds, where
/// this user can write it, which a per-machine install's, under Program Files,
/// isn't. The store goes in it, and the app's projects beside the store.
pub fn root() -> Option<&'static Path> {
    static ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
        if !cfg!(all(windows, not(debug_assertions))) {
            return None;
        }
        let executable = std::env::current_exe().expect("Koharu knows where its executable is");
        let folder = executable.parent().expect("an executable is in a folder");
        match tempfile::tempfile_in(folder) {
            Err(error) if error.kind() == ErrorKind::PermissionDenied => None,
            _ => Some(folder.to_owned()),
        }
    });
    ROOT.as_deref()
}

static DEVICE: OnceCell<Device> = OnceCell::const_new();

/// Where every model runs: the best GPU, once what it needs is downloaded and
/// loaded. Every caller shares that start, and a failed one is tried again on
/// the next call.
pub async fn device() -> anyhow::Result<&'static Device> {
    DEVICE
        .get_or_try_init(|| async {
            let gpu = hardware::candidates()
                .into_iter()
                .next()
                .context("GPU is not available")?;
            let device = packages::start(&gpu)
                .await
                .with_context(|| format!("{} can't be used", gpu.description))?;
            tracing::info!("models run on {}", device.description);
            Ok(device)
        })
        .await
}

/// Returns the active runtime device without downloading missing native
/// packages. Initializes and loads it if every required package is already on
/// disk, or returns `None` otherwise.
pub async fn try_device() -> Option<&'static Device> {
    if let Some(device) = DEVICE.get() {
        return Some(device);
    }
    let gpu = hardware::candidates().into_iter().next()?;
    if !packages::all_ready(&gpu) {
        return None;
    }
    device().await.ok()
}
