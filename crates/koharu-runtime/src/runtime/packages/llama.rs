use std::path::PathBuf;

use anyhow::{Context, Result};
use strum::EnumProperty;

use crate::{
    Device, Hardware, Store, download,
    runtime::{
        DiscoverablePackage, Package, RuntimePackage,
        graph::Component,
        loader,
        packages::{Cuda, Rocm},
        sealed,
    },
    source::extract,
};

const RELEASE: &str = "b10752";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, strum::Display, strum::EnumProperty)]
pub(crate) enum Llama {
    #[strum(
        serialize = "windows-cuda",
        props(
            asset = "x86_64-pc-windows-msvc-cuda.tar.gz",
            libraries = "llama.dll,mtmd.dll"
        )
    )]
    WindowsCuda,
    #[cfg_attr(
        all(target_os = "linux", target_arch = "aarch64"),
        strum(props(asset = "aarch64-unknown-linux-gnu-cuda.tar.gz"))
    )]
    #[cfg_attr(
        not(all(target_os = "linux", target_arch = "aarch64")),
        strum(props(asset = "x86_64-unknown-linux-gnu-cuda.tar.gz"))
    )]
    #[strum(serialize = "linux-cuda", props(libraries = "libllama.so,libmtmd.so"))]
    LinuxCuda,
    #[strum(
        serialize = "windows-hip",
        props(
            asset = "x86_64-pc-windows-msvc-hip.tar.gz",
            libraries = "llama.dll,mtmd.dll"
        )
    )]
    WindowsHip,
    #[strum(
        serialize = "linux-hip",
        props(
            asset = "x86_64-unknown-linux-gnu-hip.tar.gz",
            libraries = "libllama.so,libmtmd.so"
        )
    )]
    LinuxHip,
    #[strum(
        serialize = "windows-vulkan",
        props(
            asset = "x86_64-pc-windows-msvc-vulkan.tar.gz",
            libraries = "llama.dll,mtmd.dll"
        )
    )]
    WindowsVulkan,
    #[strum(
        serialize = "linux-vulkan",
        props(
            asset = "x86_64-unknown-linux-gnu-vulkan.tar.gz",
            libraries = "libllama.so,libmtmd.so"
        )
    )]
    LinuxVulkan,
    #[strum(
        serialize = "macos-metal",
        props(
            asset = "aarch64-apple-darwin-metal.tar.gz",
            libraries = "libllama.dylib,libmtmd.dylib"
        )
    )]
    MacosMetal,
}

impl Llama {
    fn asset(self) -> &'static str {
        self.get_str("asset").expect("llama package has an asset")
    }

    fn libraries(self) -> impl Iterator<Item = &'static str> {
        self.get_str("libraries")
            .expect("llama package has libraries")
            .split(',')
    }
}

impl sealed::Sealed for Llama {}

impl Package for Llama {
    async fn install(self) -> Result<PathBuf> {
        let asset = self.asset();
        let path = Store::root()
            .join("llama")
            .join(RELEASE)
            .join(asset.trim_end_matches(".tar.gz"));
        Store::directory(
            path,
            move |path| self.libraries().all(|name| path.join(name).is_file()),
            move |stage| async move {
                let url = format!(
                    "https://github.com/koharu-rs/llama/releases/download/{RELEASE}/{asset}"
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

impl DiscoverablePackage for Llama {
    fn discover(hardware: &Hardware) -> Option<Self> {
        if hardware.supports_cuda() {
            return Some(if cfg!(target_os = "windows") {
                Self::WindowsCuda
            } else {
                Self::LinuxCuda
            });
        }
        if hardware.supports_rocm() {
            return Some(if cfg!(target_os = "windows") {
                Self::WindowsHip
            } else {
                Self::LinuxHip
            });
        }
        if hardware.supports_vulkan() {
            return Some(if cfg!(target_os = "windows") {
                Self::WindowsVulkan
            } else {
                Self::LinuxVulkan
            });
        }
        hardware.supports_metal().then_some(Self::MacosMetal)
    }
}

impl RuntimePackage for Llama {
    const NAME: &'static str = "llama";

    fn dependencies(self, hardware: &Hardware) -> Result<Vec<Component>> {
        match self {
            Self::WindowsCuda | Self::LinuxCuda => Ok(vec![
                Component::Cuda(Cuda::Runtime13),
                Component::Cuda(Cuda::Blas13),
            ]),
            Self::WindowsHip | Self::LinuxHip => Ok(vec![Component::Rocm(Rocm(
                hardware
                    .rocm_target()
                    .context("no ROCm device was discovered")?,
            ))]),
            Self::WindowsVulkan | Self::LinuxVulkan | Self::MacosMetal => Ok(Vec::new()),
        }
    }

    async fn activate(self, _device: &mut Device) -> Result<()> {
        let root = self.install().await?;
        for library in self.libraries() {
            loader::load(root.join(library), false)
                .with_context(|| format!("failed to activate llama library {library}"))?;
        }
        Ok(())
    }
}
