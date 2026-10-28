//! The runtime's device as libtorch sees it, and the precision models run
//! in there.

use anyhow::{Result, bail};
use koharu_runtime::{Backend, Device};
use tch::Kind;

/// `device` as a libtorch device, and the precision its models run in:
/// BF16 where the device computes in it, F32 elsewhere.
pub(crate) fn torch(device: &Device) -> Result<(tch::Device, Kind)> {
    let bfloat16 = match device.backend {
        Backend::Cuda => device.compute_capability() >= 80,
        Backend::Rocm => device.target().is_some_and(|target| {
            matches!(target, "gfx908" | "gfx90a" | "gfx942" | "gfx950")
                || target.starts_with("gfx11")
                || target.starts_with("gfx12")
        }),
        Backend::Metal => false,
        Backend::Vulkan => bail!(
            "libtorch doesn't run on Vulkan, so {} can't run this model",
            device.description
        ),
    };
    let kind = if bfloat16 {
        Kind::BFloat16
    } else {
        Kind::Float
    };
    Ok(match device.backend {
        // libtorch addresses HIP devices as CUDA ones.
        Backend::Cuda | Backend::Rocm => (tch::Device::Cuda(device.index), kind),
        Backend::Metal => (tch::Device::Mps, kind),
        Backend::Vulkan => unreachable!("refused above"),
    })
}
