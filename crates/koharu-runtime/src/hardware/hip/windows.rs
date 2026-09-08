//! AMD discovery through the Windows OpenCL ICD.

use std::ffi::c_void;

use strum::EnumProperty;

use super::GfxTarget;
use crate::{Backend, Device, DeviceType};

type ClInt = i32;
type ClUint = u32;
type ClDeviceType = u64;
type ClId = *mut c_void;

type GetPlatformIds = unsafe extern "system" fn(ClUint, *mut ClId, *mut ClUint) -> ClInt;
type GetInfo = unsafe extern "system" fn(ClId, ClUint, usize, *mut c_void, *mut usize) -> ClInt;
type GetDeviceIds =
    unsafe extern "system" fn(ClId, ClDeviceType, ClUint, *mut ClId, *mut ClUint) -> ClInt;

const CL_SUCCESS: ClInt = 0;
const CL_DEVICE_TYPE_GPU: ClDeviceType = 1 << 2;
const CL_DEVICE_NAME: ClUint = 0x102b;
const MAX_OBJECTS: ClUint = 1024;
const MAX_INFO_SIZE: usize = 64 * 1024;

struct OpenCl {
    _library: libloading::os::windows::Library,
    platform_ids: GetPlatformIds,
    device_ids: GetDeviceIds,
    device_info: GetInfo,
}

impl OpenCl {
    fn load() -> Option<Self> {
        // A bare DLL name could select an application-local loader instead of
        // the system ICD loader.
        let library = unsafe {
            libloading::os::windows::Library::load_with_flags(
                "OpenCL.dll",
                libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .ok()?;
        let platform_ids = unsafe { *library.get::<GetPlatformIds>(b"clGetPlatformIDs\0").ok()? };
        let device_ids = unsafe { *library.get::<GetDeviceIds>(b"clGetDeviceIDs\0").ok()? };
        let device_info = unsafe { *library.get::<GetInfo>(b"clGetDeviceInfo\0").ok()? };
        Some(Self {
            _library: library,
            platform_ids,
            device_ids,
            device_info,
        })
    }

    fn platforms(&self) -> Vec<ClId> {
        ids(|capacity, values, count| unsafe { (self.platform_ids)(capacity, values, count) })
    }

    fn devices(&self, platform: ClId) -> Vec<ClId> {
        ids(|capacity, values, count| unsafe {
            (self.device_ids)(platform, CL_DEVICE_TYPE_GPU, capacity, values, count)
        })
    }

    fn device_name(&self, device: ClId) -> Option<String> {
        info(self.device_info, device, CL_DEVICE_NAME)
    }
}

pub(super) fn probe() -> Vec<Device> {
    let Some(opencl) = OpenCl::load() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for platform in opencl.platforms() {
        for device in opencl.devices(platform) {
            let Some(name) = opencl.device_name(device) else {
                continue;
            };
            let Ok(target) = name.parse::<GfxTarget>() else {
                continue;
            };
            let index = result.len();
            result.push(Device {
                index,
                name: format!("ROCm{index}"),
                description: target.to_string(),
                backend: Backend::Rocm,
                device_type: if target.get_str("integrated").is_some() {
                    DeviceType::IntegratedGpu
                } else {
                    DeviceType::Gpu
                },
                memory_total: 0,
                memory_free: 0,
                compute_capability: 0,
                target: Some(target),
            });
        }
    }
    result
}

fn ids(mut query: impl FnMut(ClUint, *mut ClId, *mut ClUint) -> ClInt) -> Vec<ClId> {
    let mut count = 0;
    if query(0, std::ptr::null_mut(), &mut count) != CL_SUCCESS || count == 0 || count > MAX_OBJECTS
    {
        return Vec::new();
    }

    let mut values = vec![std::ptr::null_mut(); count as usize];
    let mut reported = 0;
    if query(count, values.as_mut_ptr(), &mut reported) != CL_SUCCESS {
        return Vec::new();
    }
    values.truncate(reported.min(count) as usize);
    values.retain(|value| !value.is_null());
    values
}

fn info(query: GetInfo, object: ClId, parameter: ClUint) -> Option<String> {
    let mut size = 0;
    if unsafe { query(object, parameter, 0, std::ptr::null_mut(), &mut size) } != CL_SUCCESS
        || size == 0
        || size > MAX_INFO_SIZE
    {
        return None;
    }

    let mut bytes = vec![0; size];
    let mut written = 0;
    if unsafe {
        query(
            object,
            parameter,
            bytes.len(),
            bytes.as_mut_ptr().cast(),
            &mut written,
        )
    } != CL_SUCCESS
    {
        return None;
    }
    let bytes = &bytes[..written.min(bytes.len())];
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let value = std::str::from_utf8(&bytes[..end]).ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
