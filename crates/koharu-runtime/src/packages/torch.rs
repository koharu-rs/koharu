//! libtorch with Koharu's C shim, built and released from `koharu-rs/torch`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Host, native::Release, preload};
use crate::Backend;

/// The shim at the release `koharu-torch-sys`'s headers come from.
static RELEASE: Release = Release {
    repository: "koharu-rs/torch",
    tag: koharu_torch_sys::RELEASE,
};

/// libtorch's own libraries for `backend`, in load order, before the shim,
/// or `None` where libtorch has no build for it: Vulkan. On macOS the root
/// goes first, so dyld resolves libtorch's weak C++ symbols inside one local
/// image group instead of publishing them globally.
fn libraries(backend: Backend) -> Option<&'static [&'static str]> {
    Some(match backend {
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
        // Its OpenMP runtime first.
        Backend::Cpu if cfg!(windows) => &[
            "libiomp5md.dll",
            "c10.dll",
            "torch_global_deps.dll",
            "torch_cpu.dll",
            "torch.dll",
        ],
        Backend::Cpu => &[
            "libgomp.so.1",
            "libc10.so",
            "libtorch_global_deps.so",
            "libtorch_cpu.so",
            "libtorch.so",
        ],
        Backend::Vulkan => return None,
    })
}

/// The backend libtorch runs a GPU's models on: the GPU's own where libtorch
/// has a build for it, and the CPU otherwise.
pub(super) fn backend(gpu: Backend) -> Backend {
    if libraries(gpu).is_some() {
        gpu
    } else {
        Backend::Cpu
    }
}

pub(super) async fn install(host: Host, backend: Backend) -> Result<PathBuf> {
    let libraries = libraries(backend).context("libtorch has no build for this backend")?;
    let complete = move |folder: &Path| {
        libraries
            .iter()
            .all(|library| folder.join(library).is_file())
            && folder.join(shim()).is_file()
    };
    RELEASE.install(host, backend, complete).await
}

pub(super) fn load(backend: Backend, folder: &Path) -> Result<()> {
    for library in libraries(backend).context("libtorch has no build for this backend")? {
        preload(&folder.join(library), false)?;
    }
    // libtorch's own C functions for the shim's tensors live in torch_cpu.
    koharu_torch_sys::load(&[
        &folder.join(shim()),
        &folder.join(libloading::library_filename("torch_cpu")),
    ])?;
    // libtorch's cuDNN switch also turns MIOpen on and off; MIOpen isn't
    // reliable on Windows. Set once, before any model runs.
    let cudnn = !(cfg!(windows) && matches!(backend, Backend::Rocm { .. }));
    let error = unsafe {
        koharu_torch_sys::koharu_torch_set_setting(
            koharu_torch_sys::KOHARU_TORCH_CUDNN_ENABLED,
            i64::from(cudnn),
        )
    };
    anyhow::ensure!(error == 0, "libtorch has no cuDNN setting");
    Ok(())
}

fn shim() -> std::ffi::OsString {
    libloading::library_filename("koharu-torch")
}
