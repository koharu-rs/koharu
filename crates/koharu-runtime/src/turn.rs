//! Turns on the device, for libraries that can't share it.
//!
//! ggml captures what it runs on CUDA and ROCm as a graph. A call that waits
//! for the whole device meanwhile fails and spoils the capture, and ggml aborts
//! the process at either. ggml holds off only its own threads, and Koharu loads
//! it twice, beside libtorch:
//! - stable-diffusion.cpp makes such a call as it frees a buffer
//!   (https://github.com/leejet/ggml/blob/eeda9ec48fc84a3df0bb9dcc19ae9053075a2539/src/ggml-cuda/ggml-cuda.cu#L1012-L1014),
//!   and spoils a capture as it frees a model, with its cuBLAS handles;
//! - libtorch does neither on CUDA, but on ROCm through 10.2 any copy it waits
//!   for spoils one (https://github.com/ROCm/rocm-systems/pull/14449).

use std::time::{Duration, Instant};

use parking_lot::{FairMutex, FairMutexGuard};

use crate::{Backend, DEVICE};

/// How long a turn lasts while another library waits: long enough for most
/// pages' translations, short enough to wait for at a brush stroke.
const LENGTH: Duration = Duration::from_secs(5);

/// What runs on the device: llama.cpp and stable-diffusion.cpp, on ggml, or
/// libtorch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Library {
    Ggml,
    Torch,
}

/// A library's turn on the device, until it drops. A thread in one can't take
/// another.
#[must_use]
pub struct Turn {
    /// `None` where the library takes no turns.
    device: Option<FairMutexGuard<'static, ()>>,
    since: Instant,
}

impl Turn {
    /// `library`'s turn, once the one under way ends.
    pub fn take(library: Library) -> Self {
        // Fair, or a library with call after call to make would keep the
        // device.
        static TAKEN: FairMutex<()> = FairMutex::new(());
        let turns = DEVICE.get().is_some_and(|device| match device.backend {
            Backend::Cuda { .. } => library == Library::Ggml,
            Backend::Rocm { .. } => true,
            Backend::Cpu | Backend::Metal | Backend::Vulkan => false,
        });
        Self {
            device: turns.then(|| TAKEN.lock()),
            since: Instant::now(),
        }
    }

    /// Between two steps of long work: lets a library that waits go first,
    /// once this turn has lasted `LENGTH`.
    pub fn pass(&mut self) {
        if let Some(device) = &mut self.device
            && self.since.elapsed() >= LENGTH
        {
            FairMutexGuard::bump(device);
            self.since = Instant::now();
        }
    }
}
