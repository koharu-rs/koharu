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

const RELEASE: &str = "master-841-6b3edaa.2";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, strum::Display, strum::EnumProperty)]
pub(crate) enum Diffusion {
    #[strum(
        serialize = "windows-cuda",
        props(
            asset = "x86_64-pc-windows-msvc-cuda.tar.gz",
            library = "stable-diffusion.dll"
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
    #[strum(serialize = "linux-cuda", props(library = "libstable-diffusion.so"))]
    LinuxCuda,
    #[strum(
        serialize = "windows-hip",
        props(
            asset = "x86_64-pc-windows-msvc-hip.tar.gz",
            library = "stable-diffusion.dll"
        )
    )]
    WindowsHip,
    #[strum(
        serialize = "linux-hip",
        props(
            asset = "x86_64-unknown-linux-gnu-hip.tar.gz",
            library = "libstable-diffusion.so"
        )
    )]
    LinuxHip,
    #[strum(
        serialize = "windows-vulkan",
        props(
            asset = "x86_64-pc-windows-msvc-vulkan.tar.gz",
            library = "stable-diffusion.dll"
        )
    )]
    WindowsVulkan,
    #[strum(
        serialize = "linux-vulkan",
        props(
            asset = "x86_64-unknown-linux-gnu-vulkan.tar.gz",
            library = "libstable-diffusion.so"
        )
    )]
    LinuxVulkan,
    #[strum(
        serialize = "macos-metal",
        props(
            asset = "aarch64-apple-darwin-metal.tar.gz",
            library = "libstable-diffusion.dylib"
        )
    )]
    MacosMetal,
}

impl Diffusion {
    fn asset(self) -> &'static str {
        self.get_str("asset")
            .expect("diffusion package has an asset")
    }

    fn library(self) -> &'static str {
        self.get_str("library")
            .expect("diffusion package has a library")
    }
}

impl sealed::Sealed for Diffusion {}

impl Package for Diffusion {
    async fn install(self) -> Result<PathBuf> {
        let asset = self.asset();
        let path = Store::root()
            .join("diffusion")
            .join(RELEASE)
            .join(asset.trim_end_matches(".tar.gz"));
        Store::directory(
            path,
            move |path| path.join(self.library()).is_file(),
            move |stage| async move {
                let url = format!(
                    "https://github.com/koharu-rs/diffusion/releases/download/{RELEASE}/{asset}"
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

impl DiscoverablePackage for Diffusion {
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

impl RuntimePackage for Diffusion {
    const NAME: &'static str = "diffusion";

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
        loader::load(root.join(self.library()), false)?;
        Ok(())
    }
}
