//! Gets Koharu ready to run models on this machine's GPU.

mod device;
mod hardware;
mod huggingface;
mod packages;
mod store;
mod transfer;

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use tokio::sync::OnceCell;

pub use device::{Backend, Device, Memory};
pub use huggingface::Repository;
pub use transfer::{State, Transfer, transfers};

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

/// Where every model runs: the best GPU, once what it needs is downloaded and
/// loaded. Every caller shares that start, and a failed one is tried again on
/// the next call.
pub async fn device() -> anyhow::Result<&'static Device> {
    static DEVICE: OnceCell<Device> = OnceCell::const_new();
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
