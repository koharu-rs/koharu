#[cfg(any(target_os = "linux", target_os = "windows"))]
mod cuda;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
))]
mod hip;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod metal;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod vulkan;

use std::cmp::Reverse;

use crate::{Backend, Device};

/// The GPUs Koharu supports on this machine, best first.
pub(crate) fn candidates() -> Vec<Device> {
    let mut devices = probe();
    devices.sort_by_key(|(device, memory)| rank(device, *memory));
    devices.into_iter().map(|(device, _)| device).collect()
}

/// Every supported GPU each probe finds, with its total memory where the
/// probe reads it.
fn probe() -> Vec<(Device, u64)> {
    let mut devices = Vec::new();
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    devices.push((metal::probe(), 0));
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        devices.extend(cuda::probe());
        #[cfg(target_arch = "x86_64")]
        devices.extend(hip::probe().into_iter().map(|device| (device, 0)));
        devices.extend(vulkan::probe().map(|device| (device, 0)));
    }
    devices
}

/// Where `device` ranks, lowest first: Metal or a discrete NVIDIA GPU, a
/// discrete AMD GPU, integrated GPUs, then Vulkan. Within a rank, more memory
/// and then a newer CUDA architecture come first.
fn rank(device: &Device, memory: u64) -> (u8, Reverse<u64>, Reverse<u32>, usize) {
    let (rank, compute_capability) = match (device.backend, device.integrated) {
        (Backend::Metal, _) => (0, 0),
        (Backend::Cuda { compute_capability }, false) => (0, compute_capability),
        (Backend::Rocm { .. }, false) => (1, 0),
        (Backend::Cuda { compute_capability }, true) => (2, compute_capability),
        (Backend::Rocm { .. }, true) => (3, 0),
        (Backend::Vulkan | Backend::Cpu, _) => (4, 0),
    };
    (
        rank,
        Reverse(memory),
        Reverse(compute_capability),
        device.index,
    )
}
