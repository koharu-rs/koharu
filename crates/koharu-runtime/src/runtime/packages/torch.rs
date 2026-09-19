use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use strum::EnumProperty;

use crate::{
    Hardware, Store, download,
    runtime::{
        DiscoverablePackage, Package, RuntimePackage, graph::Component, loader, packages::Cuda,
        sealed,
    },
    source::extract,
};

const RELEASE: &str = "v2.13.0.7";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, strum::Display, strum::EnumProperty)]
pub enum Torch {
    // Keep macOS root-first so dyld resolves LibTorch's weak C++ symbols inside
    // one private RTLD_LOCAL image group instead of publishing them globally.
    #[cfg_attr(target_os = "macos", strum(serialize = "metal"))]
    #[cfg_attr(not(target_os = "macos"), strum(serialize = "cpu"))]
    #[strum(props(
        windows = "libiomp5md.dll,c10.dll,torch_global_deps.dll,torch_cpu.dll,torch.dll,koharu-torch.dll",
        linux = "libgomp.so.1,libc10.so,libtorch_global_deps.so,libtorch_cpu.so,libtorch.so,libkoharu-torch.so",
        macos = "libtorch.dylib,libtorch_global_deps.dylib,libtorch_cpu.dylib,libc10.dylib,libkoharu-torch.dylib"
    ))]
    Cpu,
    #[strum(
        serialize = "cuda",
        props(
            windows = "c10.dll,c10_cuda.dll,caffe2_nvrtc.dll,torch_global_deps.dll,torch_cpu.dll,torch_cuda.dll,torch.dll,koharu-torch.dll",
            linux = "libc10.so,libc10_cuda.so,libcaffe2_nvrtc.so,libtorch_global_deps.so,libtorch_cpu.so,libtorch_cuda.so,libtorch.so,libkoharu-torch.so"
        )
    )]
    Cuda,
    #[strum(
        serialize = "hip",
        props(
            windows = "c10.dll,c10_hip.dll,caffe2_nvrtc.dll,torch_global_deps.dll,torch_cpu.dll,torch_hip.dll,torch.dll,koharu-torch.dll",
            linux = "libc10.so,libc10_hip.so,libcaffe2_nvrtc.so,libtorch_global_deps.so,libtorch_cpu.so,libtorch_hip.so,libtorch.so,libkoharu-torch.so"
        )
    )]
    Rocm,
}

impl Torch {
    pub fn library_names(self) -> Result<impl Iterator<Item = &'static str>> {
        let property = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "macos"
        } else {
            anyhow::bail!("Torch does not support this target")
        };
        Ok(self
            .get_str(property)
            .with_context(|| format!("Torch {self} does not support this target"))?
            .split(','))
    }

    fn complete(self, root: &Path) -> bool {
        self.library_names()
            .is_ok_and(|names| names.into_iter().all(|name| root.join(name).is_file()))
    }

    fn asset(self) -> Result<String> {
        let target = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            "x86_64-pc-windows-msvc"
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            "x86_64-unknown-linux-gnu"
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            "aarch64-unknown-linux-gnu"
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "aarch64-apple-darwin"
        } else {
            anyhow::bail!("Torch {self} does not support this target")
        };
        Ok(format!("{target}-{self}.tar.gz"))
    }
}

impl sealed::Sealed for Torch {}

impl Package for Torch {
    async fn install(self) -> Result<PathBuf> {
        let path = Store::root()
            .join("torch")
            .join(RELEASE)
            .join(self.to_string());
        let asset = self.asset()?;

        Store::directory(
            path,
            move |path| self.complete(path),
            move |stage| async move {
                let url = format!(
                    "https://github.com/koharu-rs/torch/releases/download/{RELEASE}/{asset}"
                );
                let archive = tempfile::Builder::new().suffix(".tar.gz").tempfile()?;
                download::fetch(&url, archive.path()).await?;
                extract(
                    archive.path(),
                    &stage,
                    &["**/*.dll", "**/*.dylib", "**/*.so", "**/*.so.*"],
                )
            },
        )
        .await
    }
}

impl DiscoverablePackage for Torch {
    fn discover(hardware: &Hardware) -> Option<Self> {
        if hardware.supports_metal() {
            return Some(Self::Cpu);
        }
        if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            return hardware.supports_cuda().then_some(Self::Cuda);
        }
        if !cfg!(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64")
        )) {
            return None;
        }
        if hardware.supports_cuda() {
            return Some(Self::Cuda);
        }
        if hardware.supports_rocm() {
            return Some(Self::Rocm);
        }
        tracing::warn!("no supported Torch accelerator was discovered; using CPU");
        Some(Self::Cpu)
    }
}

impl RuntimePackage for Torch {
    const NAME: &'static str = "Torch";

    fn dependencies(self, hardware: &Hardware) -> Result<Vec<Component>> {
        match self {
            Self::Cpu => Ok(Vec::new()),
            Self::Rocm => Ok(vec![Component::Rocm(hardware.rocm_target()?)]),
            Self::Cuda => {
                let packages = [
                    Cuda::Runtime13,
                    Cuda::JitLink13,
                    Cuda::Rtc13,
                    Cuda::Blas13,
                    Cuda::Fft12,
                    Cuda::Rand10,
                    Cuda::Sparse12,
                    Cuda::Solver12,
                    Cuda::Dnn925,
                ];
                Ok(packages.into_iter().map(Component::Cuda).collect())
            }
        }
    }

    async fn activate(self) -> Result<()> {
        let directory = self.install().await?;
        for library in self.library_names()? {
            loader::load(directory.join(library), false)?;
        }
        Ok(())
    }
}
