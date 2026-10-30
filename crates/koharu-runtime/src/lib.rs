//! Gets Koharu ready to run models on this machine's GPU. The README is the
//! design.

mod device;
mod hardware;
mod huggingface;
mod network;
mod packages;
mod store;
mod transfer;

use tokio::sync::OnceCell;

pub use device::{Backend, Device, Memory};
pub use huggingface::Repository;
pub use network::http_client;
pub use transfer::{State, Transfer, Transfers, transfers};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No GPU Koharu supports could be used; each one tried says why.
    #[error(
        "Koharu needs a GPU: an NVIDIA GPU with CUDA driver 13.3 or later and compute capability 7.5 or later, an AMD GPU ROCm supports, a GPU with Vulkan, or Apple silicon{}",
        tried(.0)
    )]
    NoGpu(Vec<String>),
}

fn tried(failures: &[String]) -> String {
    if failures.is_empty() {
        String::new()
    } else {
        format!(". Tried {}", failures.join("; "))
    }
}

/// The device every model runs on. The first call installs and loads what
/// the best GPU needs, falling back to the next when one fails before any of
/// its bound libraries loaded; every caller shares that start, and a failed
/// one is tried again on the next call.
pub async fn device() -> Result<&'static Device, Error> {
    static DEVICE: OnceCell<Device> = OnceCell::const_new();
    DEVICE
        .get_or_try_init(|| async {
            let mut failures = Vec::new();
            for candidate in hardware::candidates() {
                match packages::start(&candidate).await {
                    Ok(device) => {
                        tracing::info!("running on {} ({})", device.description, device.name);
                        return Ok(device);
                    }
                    Err(packages::Failure { error, last }) => {
                        tracing::warn!("{} can't be used: {error:#}", candidate.description);
                        failures.push(format!("{}: {error:#}", candidate.description));
                        if last {
                            break;
                        }
                    }
                }
            }
            Err(Error::NoGpu(failures))
        })
        .await
}
