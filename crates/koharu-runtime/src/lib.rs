//! Gets Koharu ready to run models on this machine's GPU. The README is the
//! design.

mod device;
pub mod download;
mod hardware;
mod hugging_face;
mod network;
mod packages;
mod store;

use tokio::sync::OnceCell;

pub use device::{Backend, Device, Memory};
pub use hugging_face::HuggingFaceFile;
pub use store::Store;

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

/// The native libraries, installed and loaded for the one device the runtime
/// started on.
#[derive(Debug)]
pub struct Runtime {
    device: Device,
}

impl Runtime {
    /// Installs and loads what the best GPU needs, falling back to the next
    /// when one fails before any of its bound libraries loaded. Every caller
    /// shares one start; a failed one can be tried again.
    pub async fn start() -> Result<&'static Self, Error> {
        static RUNTIME: OnceCell<Runtime> = OnceCell::const_new();
        RUNTIME
            .get_or_try_init(|| async {
                let mut failures = Vec::new();
                for candidate in hardware::candidates() {
                    match packages::start(&candidate).await {
                        Ok(device) => {
                            tracing::info!("running on {} ({})", device.description, device.name);
                            return Ok(Self { device });
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

    /// The device every model runs on.
    #[must_use]
    pub fn device(&self) -> &Device {
        &self.device
    }
}

/// The app's one HTTP client, which `koharu-llm` sends hosted requests
/// through. A response that stops arriving fails after `[http]`'s read
/// timeout; nothing is retried, as a generation request mustn't be replayed.
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    network::http()
}
