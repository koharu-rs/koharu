//! The runtime's device as libtorch sees it, and the precision models run
//! in there.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use koharu_runtime::{Backend, Device};
use koharu_torch::{Kind, Tensor};

/// Whether libtorch's allocator has started. Until a tensor starts it, it holds
/// nothing and its memory calls fail, as `assertValidDevice` finds no device.
static STARTED: AtomicBool = AtomicBool::new(false);

/// `device` as a libtorch device, and the precision its models run in, with
/// libtorch's allocator started there.
pub(crate) fn torch(device: &Device) -> (koharu_torch::Device, Kind) {
    let placed = placement(device);
    if Tensor::empty_memory_format(&[0], (Kind::Uint8, placed.0), None).is_ok() {
        STARTED.store(true, Ordering::Relaxed);
    }
    placed
}

/// `device` as a libtorch device, and the precision its models run in: BF16
/// where the device computes in it, F32 elsewhere.
fn placement(device: &Device) -> (koharu_torch::Device, Kind) {
    let precision = |bfloat16| {
        if bfloat16 {
            Kind::BFloat16
        } else {
            Kind::Float
        }
    };
    match device.backend {
        // libtorch has no Vulkan build, so the runtime installs its CPU one.
        Backend::Cpu | Backend::Vulkan => (koharu_torch::Device::Cpu, Kind::Float),
        // libtorch addresses HIP devices as CUDA ones.
        Backend::Cuda { compute_capability } => (
            koharu_torch::Device::Cuda(device.index),
            precision(compute_capability >= 80),
        ),
        Backend::Rocm { target } => (
            koharu_torch::Device::Cuda(device.index),
            precision(
                matches!(target, "gfx908" | "gfx90a" | "gfx942" | "gfx950")
                    || target.starts_with("gfx11")
                    || target.starts_with("gfx12"),
            ),
        ),
        Backend::Metal => (koharu_torch::Device::Mps, Kind::Float),
    }
}

/// Device memory libtorch's caching allocator holds on `device` that no tensor
/// uses, such as what an unloaded model's weights took.
pub fn torch_cache(device: &Device) -> Result<u64> {
    let Some(index) = accelerator(device) else {
        return Ok(0);
    };
    let memory = koharu_torch::Cuda::memory(index)?;
    Ok(memory.reserved.saturating_sub(memory.active))
}

/// Hands libtorch's unused memory on `device` back to the device once its queued
/// work ends, the order ComfyUI frees a model in.
pub fn release_torch_cache(device: &Device) -> Result<()> {
    let Some(index) = accelerator(device) else {
        return Ok(());
    };
    koharu_torch::Cuda::synchronize(index)?;
    koharu_torch::Cuda::empty_cache()?;
    Ok(())
}

/// How many of libtorch's allocations on `device` have failed for lack of memory
/// so far. A failed call raised this if it ran out of memory.
pub fn torch_ooms(device: &Device) -> Result<u64> {
    let Some(index) = accelerator(device) else {
        return Ok(0);
    };
    Ok(koharu_torch::Cuda::memory(index)?.ooms)
}

/// The index libtorch's accelerator calls `device` by, or `None` where libtorch
/// runs on the CPU or hasn't started.
fn accelerator(device: &Device) -> Option<i64> {
    if !STARTED.load(Ordering::Relaxed) {
        return None;
    }
    match placement(device).0 {
        koharu_torch::Device::Cpu => None,
        koharu_torch::Device::Cuda(index) => Some(index as i64),
        koharu_torch::Device::Mps => Some(0),
    }
}
