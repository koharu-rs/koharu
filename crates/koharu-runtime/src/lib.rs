//! Gets Koharu ready to run models on this machine's GPU.

mod device;
mod hardware;
mod huggingface;
mod packages;
mod store;
mod transfer;

use anyhow::Context;
use tokio::sync::OnceCell;

pub use device::{Backend, Device, Memory};
pub use huggingface::Repository;
pub use transfer::{State, Transfer, transfers};

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
