//! Devices on which tensor computations are run.

use koharu_torch_sys::{
    ATEN_CUDNN_BENCHMARK, ATEN_CUDNN_ENABLED, ATEN_HAS_CUDNN, AtenMemoryStats, AtenSetting,
    aten_accelerator, aten_empty_cache, aten_has, aten_memory_stats, aten_set_setting,
    aten_setting, aten_synchronize,
};

use super::utils::check;
use crate::{Error, Result};

/// c10::DeviceType's values for the devices Koharu runs on. A HIP build of
/// libtorch reports its GPUs as CUDA ones.
const CPU: i32 = 0;
const CUDA: i32 = 1;
const MPS: i32 = 13;

/// A torch device.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Device {
    /// The main CPU device.
    Cpu,
    /// The main GPU device.
    Cuda(usize),
    /// The main MPS device.
    Mps,
}

/// Cuda related helper functions.
pub enum Cuda {}

impl Cuda {
    /// Returns the number of CUDA devices available.
    pub fn device_count() -> i64 {
        let (mut device_type, mut count) = (0, 0);
        check(unsafe { aten_accelerator(&mut device_type, &mut count) })
            .expect("libtorch reports its accelerator");
        if device_type == CUDA {
            i64::from(count)
        } else {
            0
        }
    }

    /// Returns true if at least one CUDA device is available.
    pub fn is_available() -> bool {
        Cuda::device_count() > 0
    }

    /// Returns true if CUDA is available, and CuDNN is available.
    pub fn cudnn_is_available() -> bool {
        let mut has = false;
        check(unsafe { aten_has(ATEN_HAS_CUDNN, &mut has) })
            .expect("libtorch reports its features");
        Cuda::is_available() && has
    }

    /// Waits for all kernels in all streams on a CUDA device to complete.
    pub fn synchronize(device_index: i64) -> Result<()> {
        check(unsafe { aten_synchronize(index(device_index)?) })
    }

    /// What the caching allocator holds on a device now.
    pub fn memory(device_index: i64) -> Result<AllocatorMemory> {
        let mut stats = unsafe { std::mem::zeroed::<AtenMemoryStats>() };
        check(unsafe { aten_memory_stats(index(device_index)?, &mut stats) })?;
        let count = |count: i64| u64::try_from(count).unwrap_or(0);
        Ok(AllocatorMemory {
            reserved: count(stats.reserved_bytes.current),
            active: count(stats.active_bytes.current),
            ooms: count(stats.num_ooms),
        })
    }

    /// Returns the caching allocator's unused device memory to the device.
    pub fn empty_cache() -> Result<()> {
        check(unsafe { aten_empty_cache() })
    }

    /// Returns true if cudnn is enabled by the user.
    ///
    /// This does not indicate whether cudnn is actually usable.
    pub fn user_enabled_cudnn() -> bool {
        setting(ATEN_CUDNN_ENABLED) != 0
    }

    /// Enable or disable cudnn.
    pub fn set_user_enabled_cudnn(b: bool) {
        set_setting(ATEN_CUDNN_ENABLED, i64::from(b))
    }

    /// Sets cudnn benchmark mode.
    ///
    /// When set cudnn will try to optimize the generators durning
    /// the first network runs and then use the optimized architecture
    /// in the following runs. This can result in significant performance
    /// improvements.
    pub fn cudnn_set_benchmark(b: bool) {
        set_setting(ATEN_CUDNN_BENCHMARK, i64::from(b))
    }
}

/// The caching allocator's device memory, in bytes: what it holds from the
/// device, and how much of that tensors use. The rest it keeps for its own next
/// allocations until `Cuda::empty_cache`. `ooms` counts the allocations it has
/// failed for lack of memory, which tells an out-of-memory error from any other
/// without reading its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocatorMemory {
    pub reserved: u64,
    pub active: u64,
    pub ooms: u64,
}

fn index(device_index: i64) -> Result<i32> {
    i32::try_from(device_index).map_err(|_| Error::Convert(format!("no device {device_index}")))
}

fn setting(setting: AtenSetting) -> i64 {
    let mut value = 0;
    check(unsafe { aten_setting(setting, &mut value) }).expect("libtorch has the setting");
    value
}

fn set_setting(setting: AtenSetting, value: i64) {
    check(unsafe { aten_set_setting(setting, value) }).expect("libtorch has the setting")
}

impl Device {
    /// The c10::DeviceType and index libtorch's C functions take.
    pub(crate) fn c_parts(self) -> (i32, i32) {
        match self {
            Device::Cpu => (CPU, -1),
            Device::Cuda(index) => (CUDA, index as i32),
            Device::Mps => (MPS, -1),
        }
    }

    pub(crate) fn from_c_parts(device_type: i32, index: i32) -> Result<Device> {
        match device_type {
            CPU => Ok(Device::Cpu),
            CUDA => Ok(Device::Cuda(usize::try_from(index).unwrap_or(0))),
            MPS => Ok(Device::Mps),
            _ => Err(Error::Convert(format!(
                "unsupported device type {device_type}"
            ))),
        }
    }

    /// Returns a GPU device if available, else default to CPU.
    pub fn cuda_if_available() -> Device {
        if Cuda::is_available() {
            Device::Cuda(0)
        } else {
            Device::Cpu
        }
    }

    pub fn is_cuda(self) -> bool {
        match self {
            Device::Cuda(_) => true,
            Device::Cpu | Device::Mps => false,
        }
    }
}
