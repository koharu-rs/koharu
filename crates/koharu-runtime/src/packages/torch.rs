//! libtorch with Koharu's C shim, built and released from `koharu-rs/torch`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{Host, native::Release, preload};
use crate::Backend;

/// The shim at the release `koharu-torch-sys`'s headers come from.
static RELEASE: Release = Release {
    repository: "koharu-rs/torch",
    tag: koharu_torch_sys::RELEASE,
};

/// libtorch's own libraries, in load order, before the shim. On macOS the
/// root goes first, so dyld resolves libtorch's weak C++ symbols inside one
/// local image group instead of publishing them globally.
fn libraries(backend: Backend) -> &'static [&'static str] {
    match backend {
        Backend::Cuda { .. } if cfg!(windows) => &[
            "c10.dll",
            "c10_cuda.dll",
            "caffe2_nvrtc.dll",
            "torch_global_deps.dll",
            "torch_cpu.dll",
            "torch_cuda.dll",
            "torch.dll",
        ],
        Backend::Cuda { .. } => &[
            "libc10.so",
            "libc10_cuda.so",
            "libcaffe2_nvrtc.so",
            "libtorch_global_deps.so",
            "libtorch_cpu.so",
            "libtorch_cuda.so",
            "libtorch.so",
        ],
        Backend::Rocm { .. } if cfg!(windows) => &[
            "c10.dll",
            "c10_hip.dll",
            "caffe2_nvrtc.dll",
            "torch_global_deps.dll",
            "torch_cpu.dll",
            "torch_hip.dll",
            "torch.dll",
        ],
        Backend::Rocm { .. } => &[
            "libc10.so",
            "libc10_hip.so",
            "libcaffe2_nvrtc.so",
            "libtorch_global_deps.so",
            "libtorch_cpu.so",
            "libtorch_hip.so",
            "libtorch.so",
        ],
        Backend::Metal => &[
            "libtorch.dylib",
            "libtorch_global_deps.dylib",
            "libtorch_cpu.dylib",
            "libc10.dylib",
        ],
        Backend::Vulkan => &[],
    }
}

/// Installs libtorch for `backend`, or nothing on Vulkan, which libtorch
/// doesn't run on.
pub(super) async fn install(host: Host, backend: Backend) -> Result<Option<PathBuf>> {
    if backend == Backend::Vulkan {
        return Ok(None);
    }
    let complete = move |folder: &Path| {
        libraries(backend)
            .iter()
            .all(|library| folder.join(library).is_file())
            && folder.join(shim()).is_file()
    };
    Ok(Some(RELEASE.install(host, backend, complete).await?))
}

pub(super) fn load(backend: Backend, folder: &Path) -> Result<()> {
    for library in libraries(backend) {
        preload(&folder.join(library), false)?;
    }
    koharu_torch_sys::load(&[&folder.join(shim())])?;
    koharu_torch_sys::io::register_callbacks();
    // libtorch's cuDNN switch also turns MIOpen on and off; MIOpen isn't
    // reliable on Windows. Set once, before any model runs.
    let cudnn = !(cfg!(windows) && matches!(backend, Backend::Rocm { .. }));
    unsafe { koharu_torch_sys::cuda::atc_set_user_enabled_cudnn(i32::from(cudnn)) };
    Ok(())
}

fn shim() -> std::ffi::OsString {
    libloading::library_filename("koharu-torch")
}
