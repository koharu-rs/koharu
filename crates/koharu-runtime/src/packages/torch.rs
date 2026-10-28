//! libtorch with Koharu's C shim, built and released from `koharu-rs/torch`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{Host, native::Release, preload};
use crate::Backend;

pub(super) static RELEASE: Release = Release {
    repository: "koharu-rs/torch",
    tag: "v2.13.0.7",
    archives: &[
        (
            "aarch64-apple-darwin-metal",
            "cc13ec11bd014eb334595e6255ec450f8a3bbc6d8c90ff87fb04769066163754",
        ),
        (
            "aarch64-unknown-linux-gnu-cuda",
            "10b59e19b60f4f65296a72bf03e5a98125d692184e4dca0a57298ba5b3d0a0d9",
        ),
        (
            "x86_64-pc-windows-msvc-cuda",
            "6ac600e10eac53e45b9a8ca4c3d04771a0423c0b2e70adf422df46ff07766f8d",
        ),
        (
            "x86_64-pc-windows-msvc-hip",
            "4cf2b9861507a199bf57fc3e0bf4182a61438161b8ad13906ac946c859be498c",
        ),
        (
            "x86_64-unknown-linux-gnu-cuda",
            "12498b1fe8cc5e7132911211f4c68b94be7a34ebfab4da8b5dc39f0b07b65134",
        ),
        (
            "x86_64-unknown-linux-gnu-hip",
            "79e8641ddcb0bd33f7f5b2ad651c8b60a48b05f90abd68aa8c4306aa955e268e",
        ),
    ],
};

/// libtorch's own libraries, in load order, before the shim. On macOS the
/// root goes first, so dyld resolves libtorch's weak C++ symbols inside one
/// local image group instead of publishing them globally.
fn libraries(backend: Backend) -> &'static [&'static str] {
    match backend {
        Backend::Cuda if cfg!(windows) => &[
            "c10.dll",
            "c10_cuda.dll",
            "caffe2_nvrtc.dll",
            "torch_global_deps.dll",
            "torch_cpu.dll",
            "torch_cuda.dll",
            "torch.dll",
        ],
        Backend::Cuda => &[
            "libc10.so",
            "libc10_cuda.so",
            "libcaffe2_nvrtc.so",
            "libtorch_global_deps.so",
            "libtorch_cpu.so",
            "libtorch_cuda.so",
            "libtorch.so",
        ],
        Backend::Rocm if cfg!(windows) => &[
            "c10.dll",
            "c10_hip.dll",
            "caffe2_nvrtc.dll",
            "torch_global_deps.dll",
            "torch_cpu.dll",
            "torch_hip.dll",
            "torch.dll",
        ],
        Backend::Rocm => &[
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
    let variant = match backend {
        Backend::Cuda => "cuda",
        Backend::Rocm => "hip",
        Backend::Metal => "metal",
        Backend::Vulkan => return Ok(None),
    };
    let complete = move |folder: &Path| {
        libraries(backend)
            .iter()
            .all(|library| folder.join(library).is_file())
            && folder.join(shim()).is_file()
    };
    Ok(Some(
        RELEASE.install_variant(host, variant, complete).await?,
    ))
}

pub(super) fn load(backend: Backend, folder: &Path) -> Result<()> {
    for library in libraries(backend) {
        preload(&folder.join(library), false)?;
    }
    koharu_torch_sys::load(&[&folder.join(shim())])?;
    // libtorch's cuDNN switch also turns MIOpen on and off; MIOpen isn't
    // reliable on Windows. Set once, before any model runs.
    let cudnn = !(cfg!(windows) && backend == Backend::Rocm);
    unsafe { koharu_torch_sys::atc_set_user_enabled_cudnn(i32::from(cudnn)) };
    Ok(())
}

fn shim() -> std::ffi::OsString {
    libloading::library_filename("koharu-torch")
}
