//! Intel GPUs through Level Zero, which libtorch's XPU build runs on.

use std::{
    ffi::{CStr, c_char, c_void},
    sync::OnceLock,
};

use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};

use crate::{Backend, Device};

// From Level Zero's ze_api.h.
const ZE_RESULT_SUCCESS: u32 = 0;
const ZE_INIT_FLAG_GPU_ONLY: u32 = 1;
const ZE_STRUCTURE_TYPE_DEVICE_PROPERTIES: u32 = 0x3;
const ZE_STRUCTURE_TYPE_DEVICE_MEMORY_PROPERTIES: u32 = 0x7;
const ZE_DEVICE_TYPE_GPU: u32 = 1;
const ZE_DEVICE_PROPERTY_FLAG_INTEGRATED: u32 = 1 << 0;
const ZE_DEVICE_PROPERTY_FLAG_SUBDEVICE: u32 = 1 << 1;

type Handle = *mut c_void;

#[repr(C)]
struct DeviceProperties {
    stype: u32,
    next: *mut c_void,
    device_type: u32,
    vendor_id: u32,
    device_id: u32,
    flags: u32,
    subdevice_id: u32,
    core_clock_rate: u32,
    max_mem_alloc_size: u64,
    max_hardware_contexts: u32,
    max_command_queue_priority: u32,
    num_threads_per_eu: u32,
    physical_eu_simd_width: u32,
    num_eus_per_subslice: u32,
    num_subslices_per_slice: u32,
    num_slices: u32,
    timer_resolution: u64,
    timestamp_valid_bits: u32,
    kernel_timestamp_valid_bits: u32,
    uuid: [u8; 16],
    name: [c_char; 256],
}

#[repr(C)]
struct MemoryProperties {
    stype: u32,
    next: *mut c_void,
    flags: u32,
    max_clock_rate: u32,
    max_bus_width: u32,
    total_size: u64,
    name: [c_char; 256],
}

type Init = unsafe extern "C" fn(u32) -> u32;
type DriverGet = unsafe extern "C" fn(*mut u32, *mut Handle) -> u32;
type DeviceGet = unsafe extern "C" fn(Handle, *mut u32, *mut Handle) -> u32;
type DeviceGetProperties = unsafe extern "C" fn(Handle, *mut DeviceProperties) -> u32;
type DeviceGetMemoryProperties =
    unsafe extern "C" fn(Handle, *mut u32, *mut MemoryProperties) -> u32;

/// A GPU libtorch lists, at its place in libtorch's list.
struct Gpu {
    description: String,
    memory: u64,
}

/// The GPUs libtorch's XPU build lists, in its order: the discrete GPUs of
/// the first Level Zero driver with one, as PyTorch's `enumDevices` in
/// `c10/xpu/XPUFunctions.cpp` picks them. Integrated GPUs, which it falls
/// back to only without a discrete one, stay on Vulkan: PyTorch supports
/// none older than Arc's, and Level Zero doesn't say which are.
fn gpus() -> &'static [Gpu] {
    static GPUS: OnceLock<Vec<Gpu>> = OnceLock::new();
    GPUS.get_or_init(|| unsafe { enumerate() }.unwrap_or_default())
}

unsafe fn enumerate() -> Option<Vec<Gpu>> {
    // Exclude application-local and working-directory DLLs from driver
    // discovery; Intel's graphics driver installs the loader.
    let library =
        unsafe { Library::load_with_flags("ze_loader.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
    let init = unsafe { *library.get::<Init>(b"zeInit\0").ok()? };
    let get_drivers = unsafe { *library.get::<DriverGet>(b"zeDriverGet\0").ok()? };
    let get_devices = unsafe { *library.get::<DeviceGet>(b"zeDeviceGet\0").ok()? };
    let get_properties = unsafe {
        *library
            .get::<DeviceGetProperties>(b"zeDeviceGetProperties\0")
            .ok()?
    };
    let get_memory = unsafe {
        *library
            .get::<DeviceGetMemoryProperties>(b"zeDeviceGetMemoryProperties\0")
            .ok()?
    };
    if unsafe { init(ZE_INIT_FLAG_GPU_ONLY) } != ZE_RESULT_SUCCESS {
        return None;
    }
    // libtorch's SYCL runtime initializes the same loader later.
    std::mem::forget(library);

    let drivers = list(|count, handles| unsafe { get_drivers(count, handles) });
    for driver in drivers {
        let gpus: Vec<Gpu> = list(|count, handles| unsafe { get_devices(driver, count, handles) })
            .into_iter()
            .filter_map(|device| {
                let mut properties: DeviceProperties = unsafe { std::mem::zeroed() };
                properties.stype = ZE_STRUCTURE_TYPE_DEVICE_PROPERTIES;
                if unsafe { get_properties(device, &mut properties) } != ZE_RESULT_SUCCESS
                    || properties.device_type != ZE_DEVICE_TYPE_GPU
                    || properties.flags
                        & (ZE_DEVICE_PROPERTY_FLAG_INTEGRATED | ZE_DEVICE_PROPERTY_FLAG_SUBDEVICE)
                        != 0
                {
                    return None;
                }
                let name = unsafe { CStr::from_ptr(properties.name.as_ptr()) };
                Some(Gpu {
                    description: name.to_string_lossy().trim().to_owned(),
                    memory: unsafe { memory(get_memory, device) },
                })
            })
            .collect();
        if !gpus.is_empty() {
            return Some(gpus);
        }
    }
    Some(Vec::new())
}

/// A device's largest memory, which is its own on a discrete GPU.
unsafe fn memory(get_memory: DeviceGetMemoryProperties, device: Handle) -> u64 {
    let mut count = 0;
    if unsafe { get_memory(device, &mut count, std::ptr::null_mut()) } != ZE_RESULT_SUCCESS {
        return 0;
    }
    let mut memories: Vec<MemoryProperties> = (0..count)
        .map(|_| {
            let mut memory: MemoryProperties = unsafe { std::mem::zeroed() };
            memory.stype = ZE_STRUCTURE_TYPE_DEVICE_MEMORY_PROPERTIES;
            memory
        })
        .collect();
    if unsafe { get_memory(device, &mut count, memories.as_mut_ptr()) } != ZE_RESULT_SUCCESS {
        return 0;
    }
    memories
        .iter()
        .map(|memory| memory.total_size)
        .max()
        .unwrap_or(0)
}

/// Level Zero's two-call listing: the count, then the handles.
fn list(get: impl Fn(*mut u32, *mut Handle) -> u32) -> Vec<Handle> {
    let mut count = 0;
    if get(&mut count, std::ptr::null_mut()) != ZE_RESULT_SUCCESS {
        return Vec::new();
    }
    let mut handles = vec![std::ptr::null_mut(); count as usize];
    if get(&mut count, handles.as_mut_ptr()) != ZE_RESULT_SUCCESS {
        return Vec::new();
    }
    handles.truncate(count as usize);
    handles
}

/// Each GPU libtorch lists, with its memory. llama.cpp runs it through
/// Vulkan, where the ggml package finds its name and place.
pub(super) fn probe() -> Vec<(Device, u64)> {
    gpus()
        .iter()
        .enumerate()
        .map(|(index, gpu)| {
            let device = Device {
                name: format!("XPU{index}"),
                index,
                description: gpu.description.clone(),
                backend: Backend::Xpu { index },
                integrated: false,
            };
            (device, gpu.memory)
        })
        .collect()
}

/// How many GPUs before libtorch's `index` have its name. ggml's Vulkan
/// devices carry no PCI address on Intel's Windows driver, so the ggml
/// package finds an XPU GPU as the Vulkan device of its name and place among
/// namesakes; both APIs list adapters in the system's order.
pub(crate) fn namesakes_before(index: usize) -> usize {
    let gpus = gpus();
    gpus[..index]
        .iter()
        .filter(|gpu| gpu.description == gpus[index].description)
        .count()
}
