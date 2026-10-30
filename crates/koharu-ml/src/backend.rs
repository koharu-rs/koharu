//! The runtime's device as libtorch sees it, and the precision models run
//! in there.

use anyhow::{Result, bail};
use koharu_runtime::{Backend, Device};
use koharu_torch::Kind;

/// `device` as a libtorch device, and the precision its models run in:
/// BF16 where the device computes in it, F32 elsewhere.
pub(crate) fn torch(device: &Device) -> Result<(koharu_torch::Device, Kind)> {
    let bfloat16 = match device.backend {
        Backend::Cuda { compute_capability } => compute_capability >= 80,
        Backend::Rocm { target } => {
            matches!(target, "gfx908" | "gfx90a" | "gfx942" | "gfx950")
                || target.starts_with("gfx11")
                || target.starts_with("gfx12")
        }
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
        Backend::Cuda { .. } | Backend::Rocm { .. } => (koharu_torch::Device::Cuda(device.index), kind),
        Backend::Metal => (koharu_torch::Device::Mps, kind),
        Backend::Vulkan => unreachable!("refused above"),
    })
}
