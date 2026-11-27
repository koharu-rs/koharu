//! libtorch and Koharu's C shim to it, built and released apart from
//! `koharu-rs/torch`.

use std::path::PathBuf;

use anyhow::Result;
use futures::future::BoxFuture;

use super::{Host, LIBRARIES, Package, Release, preload};
use crate::{Backend, Device};

pub(super) struct Torch;

/// libtorch at the release the shim was built against. It keeps the store
/// folder of the releases that shipped both, so a start removes them.
static LIBTORCH: Release = Release {
    repository: "koharu-rs/torch",
    package: "torch",
    tag: koharu_torch_sys::LIBTORCH,
    // Its libraries, and on AMD GPUs the kernels of AOTriton, libtorch's
    // fused attention there, which it reads from `aotriton.images` beside its
    // own library.
    files: &[
        "**/*.dll",
        "**/*.dylib",
        "**/*.so",
        "**/*.so.*",
        "**/aotriton.images/**",
    ],
};

/// The shim at the release `koharu-torch-sys`'s headers come from.
static SHIM: Release = Release {
    repository: "koharu-rs/torch",
    package: "koharu-torch",
    tag: koharu_torch_sys::SHIM,
    files: LIBRARIES,
};

/// libtorch's own libraries for a GPU's `backend`, in load order, before the
/// shim. libtorch has no Vulkan build, so a Vulkan GPU's models run on its
/// CPU one. On macOS the root goes first, so dyld resolves libtorch's weak
/// C++ symbols inside one local image group instead of publishing them
/// globally.
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
            "aotriton_v2.dll",
            "torch_hip.dll",
            "torch.dll",
        ],
        Backend::Rocm { .. } => &[
            "libc10.so",
            "libc10_hip.so",
            "libcaffe2_nvrtc.so",
            "libtorch_global_deps.so",
            "libtorch_cpu.so",
            "libaotriton_v2.so",
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
        Backend::Cpu | Backend::Vulkan if cfg!(windows) => &[
            "libiomp5md.dll",
            "c10.dll",
            "torch_global_deps.dll",
            "torch_cpu.dll",
            "torch.dll",
        ],
        Backend::Cpu | Backend::Vulkan => &[
            "libgomp.so.1",
            "libc10.so",
            "libtorch_global_deps.so",
            "libtorch_cpu.so",
            "libtorch.so",
        ],
    }
}

/// The archive libtorch comes in for `backend`.
fn variant(backend: Backend) -> Backend {
    match backend {
        Backend::Vulkan => Backend::Cpu,
        backend => backend,
    }
}

fn libtorch_complete(folder: &std::path::Path, backend: Backend) -> bool {
    libraries(backend)
        .iter()
        .all(|library| folder.join(library).is_file())
        // Without its kernels, AOTriton fails every fused
        // attention as an invalid kernel image.
        && (!matches!(backend, Backend::Rocm { .. }) || folder.join("aotriton.images").is_dir())
}

fn shim_complete(folder: &std::path::Path) -> bool {
    folder.join(shim_library()).is_file()
}

impl Package for Torch {
    fn is_ready(&self, host: Host, backend: Backend) -> bool {
        LIBTORCH.is_ready(host, variant(backend), |folder| {
            libtorch_complete(folder, backend)
        }) && SHIM.is_ready(host, variant(backend), shim_complete)
    }

    /// Downloads libtorch and the shim.
    fn download(&self, host: Host, backend: Backend) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            LIBTORCH
                .download(host, variant(backend), move |folder| {
                    libtorch_complete(folder, backend)
                })
                .await?;
            SHIM.download(host, variant(backend), shim_complete).await
        })
    }

    /// Loads libtorch from its folder, then the shim from its own. The shim
    /// finds libtorch's libraries already loaded, not beside it.
    fn load(&self, host: Host, gpu: &mut Device) -> Result<()> {
        let backend = gpu.backend;
        let libtorch = LIBTORCH.folder(host, variant(backend));
        let shim = SHIM.folder(host, variant(backend));
        for library in libraries(backend) {
            preload(&libtorch.join(library), false)?;
        }
        // libtorch's own C functions for the shim's tensors live in torch_cpu.
        koharu_torch_sys::load(&[
            &shim.join(shim_library()),
            &libtorch.join(libloading::library_filename("torch_cpu")),
        ])?;
        set_cudnn(backend)
    }

    fn versions(&self) -> Vec<PathBuf> {
        vec![LIBTORCH.version(), SHIM.version()]
    }
}

/// Sets libtorch's cuDNN switch once, before any model runs: on, but off for
/// ROCm on Windows, since the switch also turns MIOpen on and off and MIOpen
/// isn't reliable on Windows.
fn set_cudnn(backend: Backend) -> Result<()> {
    let cudnn = !(cfg!(windows) && matches!(backend, Backend::Rocm { .. }));
    let error = unsafe {
        koharu_torch_sys::aten_set_setting(koharu_torch_sys::ATEN_CUDNN_ENABLED, i64::from(cudnn))
    };
    anyhow::ensure!(error == 0, "libtorch has no cuDNN setting");
    Ok(())
}

fn shim_library() -> std::ffi::OsString {
    libloading::library_filename("koharu-torch")
}
