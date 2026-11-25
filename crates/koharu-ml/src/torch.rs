//! libtorch on the runtime's device: the device as libtorch sees it, the
//! precision models run in there, and its turns on it.

use koharu_runtime::{Backend, Device, Library, Turn};
use koharu_torch::Kind;

/// `device` as a libtorch device, and the precision its models run in: BF16
/// where the device computes in it, F32 elsewhere.
pub(crate) fn device(device: &Device) -> (koharu_torch::Device, Kind) {
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

/// What `f` computes without gradients, in libtorch's turn on the device.
pub(crate) fn inference<T>(f: impl FnOnce() -> T) -> T {
    let _turn = Turn::take(Library::Torch);
    koharu_torch::inference_mode(f)
}
