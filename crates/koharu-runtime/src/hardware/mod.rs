mod cuda;
mod hip;
mod vulkan;

use std::cmp::Reverse;

use crate::{Backend, Device};

/// The GPUs Koharu supports on this machine, best first: Metal or a discrete
/// NVIDIA GPU, a discrete AMD GPU, integrated GPUs, then Vulkan. Within a
/// rank, more memory and then a newer CUDA architecture come first.
pub(crate) fn candidates() -> Vec<Device> {
    let mut devices = Vec::new();
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let metal = Device {
            name: "MTL0".to_owned(),
            description: "Apple silicon".to_owned(),
            backend: Backend::Metal,
            index: 0,
            integrated: true,
            compute_capability: 0,
            target: None,
        };
        devices.push((metal, 0));
    } else if cfg!(any(target_os = "linux", target_os = "windows")) {
        devices.extend(cuda::probe());
        devices.extend(hip::probe().into_iter().map(|device| (device, 0)));
        if vulkan::probe() {
            let vulkan = Device {
                name: "Vulkan0".to_owned(),
                description: "Vulkan".to_owned(),
                backend: Backend::Vulkan,
                index: 0,
                integrated: false,
                compute_capability: 0,
                target: None,
            };
            devices.push((vulkan, 0));
        }
    }
    devices.sort_by_key(|(device, memory)| {
        let rank = match (device.backend, device.integrated) {
            (Backend::Metal, _) | (Backend::Cuda, false) => 0,
            (Backend::Rocm, false) => 1,
            (Backend::Cuda, true) => 2,
            (Backend::Rocm, true) => 3,
            (Backend::Vulkan, _) => 4,
        };
        (
            rank,
            Reverse(*memory),
            Reverse(device.compute_capability),
            device.index,
        )
    });
    devices.into_iter().map(|(device, _)| device).collect()
}
