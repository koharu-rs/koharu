use std::{
    ffi::{CStr, c_char, c_int, c_uint},
    sync::OnceLock,
};

use libloading::Library;

use crate::{Backend, Device, DeviceType};

const ATTRIBUTE_INTEGRATED: c_int = 18;
const ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR: c_int = 75;
const ATTRIBUTE_COMPUTE_CAPABILITY_MINOR: c_int = 76;

// The bundled CUDA 13 runtimes support compute capability 7.5 and newer.
const MIN_DRIVER_VERSION: c_int = 13000;
const MIN_COMPUTE_CAPABILITY: u32 = 75;

type Init = unsafe extern "C" fn(c_uint) -> c_int;
type DeviceGetCount = unsafe extern "C" fn(*mut c_int) -> c_int;
type DeviceGet = unsafe extern "C" fn(*mut c_int, c_int) -> c_int;
type DeviceGetAttribute = unsafe extern "C" fn(*mut c_int, c_int, c_int) -> c_int;
type DeviceGetName = unsafe extern "C" fn(*mut c_char, c_int, c_int) -> c_int;
type DeviceTotalMemory = unsafe extern "C" fn(*mut usize, c_int) -> c_int;
type DriverGetVersion = unsafe extern "C" fn(*mut c_int) -> c_int;

struct Cuda {
    // Keep the library loaded for as long as its function pointers are used.
    _library: Library,
    device_count: c_int,
    get_device: DeviceGet,
    get_device_attribute: DeviceGetAttribute,
    get_device_name: DeviceGetName,
    get_device_memory: DeviceTotalMemory,
}

impl Cuda {
    fn load() -> Option<Self> {
        let library = load_library()?;
        unsafe {
            let init = *library.get::<Init>(b"cuInit\0").ok()?;
            let get_device_count = *library.get::<DeviceGetCount>(b"cuDeviceGetCount\0").ok()?;
            let get_driver_version = *library
                .get::<DriverGetVersion>(b"cuDriverGetVersion\0")
                .ok()?;

            if init(0) != 0 {
                return None;
            }

            let mut driver_version = 0;
            if get_driver_version(&mut driver_version) != 0 {
                return None;
            }
            if driver_version < MIN_DRIVER_VERSION {
                tracing::warn!(
                    driver_version,
                    required = MIN_DRIVER_VERSION,
                    "CUDA driver is too old"
                );
                return None;
            }

            let mut device_count = 0;
            if get_device_count(&mut device_count) != 0 || device_count <= 0 {
                return None;
            }

            Some(Self {
                get_device: *library.get::<DeviceGet>(b"cuDeviceGet\0").ok()?,
                get_device_attribute: *library
                    .get::<DeviceGetAttribute>(b"cuDeviceGetAttribute\0")
                    .ok()?,
                get_device_name: *library.get::<DeviceGetName>(b"cuDeviceGetName\0").ok()?,
                get_device_memory: *library
                    .get::<DeviceTotalMemory>(b"cuDeviceTotalMem_v2\0")
                    .ok()?,
                _library: library,
                device_count,
            })
        }
    }

    fn attribute(&self, handle: c_int, attribute: c_int) -> Option<u32> {
        let mut value = 0;
        if unsafe { (self.get_device_attribute)(&mut value, attribute, handle) } != 0 {
            return None;
        }
        u32::try_from(value).ok()
    }

    fn description(&self, handle: c_int) -> Option<String> {
        let mut buffer = [0u8; 256];
        if unsafe {
            (self.get_device_name)(buffer.as_mut_ptr().cast(), buffer.len() as c_int, handle)
        } != 0
        {
            return None;
        }
        let name = CStr::from_bytes_until_nul(&buffer).ok()?;
        Some(name.to_string_lossy().into_owned())
    }

    fn device(&self, index: c_int) -> Option<Device> {
        let mut handle = 0;
        if unsafe { (self.get_device)(&mut handle, index) } != 0 {
            return None;
        }

        let major = self.attribute(handle, ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR)?;
        let minor = self.attribute(handle, ATTRIBUTE_COMPUTE_CAPABILITY_MINOR)?;
        let compute_capability = major * 10 + minor;
        if compute_capability < MIN_COMPUTE_CAPABILITY {
            return None;
        }

        let name = format!("CUDA{index}");
        let description = self.description(handle).unwrap_or_else(|| name.clone());
        let mut memory_total = 0;
        if unsafe { (self.get_device_memory)(&mut memory_total, handle) } != 0 {
            memory_total = 0;
        }
        let integrated = self.attribute(handle, ATTRIBUTE_INTEGRATED).unwrap_or(0);

        Some(Device {
            index: index as usize,
            name,
            description,
            backend: Backend::Cuda,
            device_type: if integrated != 0 {
                DeviceType::IntegratedGpu
            } else {
                DeviceType::Gpu
            },
            memory_total,
            memory_free: 0,
            compute_capability,
            target: None,
        })
    }
}

pub(super) fn probe() -> Vec<Device> {
    static DRIVER: OnceLock<Option<Cuda>> = OnceLock::new();
    let Some(driver) = DRIVER.get_or_init(Cuda::load) else {
        return Vec::new();
    };
    (0..driver.device_count)
        .filter_map(|index| driver.device(index))
        .collect()
}

#[cfg(target_os = "windows")]
fn load_library() -> Option<Library> {
    // Exclude application-local and working-directory DLLs from driver discovery.
    unsafe {
        libloading::os::windows::Library::load_with_flags(
            "nvcuda.dll",
            libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
        .ok()
        .map(Into::into)
    }
}

#[cfg(target_os = "linux")]
fn load_library() -> Option<Library> {
    // The unversioned name may resolve to a toolkit stub instead of the driver.
    unsafe { Library::new("libcuda.so.1").ok() }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn load_library() -> Option<Library> {
    None
}
