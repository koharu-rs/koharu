use crate::{Backend, Device};

/// The first Vulkan device, when a Vulkan loader is installed; llama.cpp
/// says later whether the loader finds one.
pub(super) fn probe() -> Option<Device> {
    let loader = if cfg!(target_os = "windows") {
        "vulkan-1.dll"
    } else {
        "libvulkan.so.1"
    };
    unsafe { libloading::Library::new(loader) }.ok()?;
    Some(Device {
        name: "Vulkan0".to_owned(),
        description: "Vulkan".to_owned(),
        backend: Backend::Vulkan,
        index: 0,
        integrated: false,
    })
}
