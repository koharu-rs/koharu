use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use strum::EnumProperty;
use walkdir::WalkDir;

use crate::{
    Store, download,
    runtime::{Package, RuntimePackage, loader, sealed},
    source::{Platform, extract, wheel},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, strum::Display, strum::EnumProperty)]
pub(crate) enum Cuda {
    #[strum(
        serialize = "runtime-13",
        props(
            project = "nvidia-cuda-runtime/13.0.48",
            windows = "cudart64_13.dll",
            linux = "libcudart.so.13"
        )
    )]
    Runtime13,
    #[strum(
        serialize = "cublas-13",
        props(
            windows_project = "nvidia-cublas/13.0.0.19",
            linux_project = "nvidia-cublas/13.1.0.3",
            windows = "cublasLt64_13.dll,cublas64_13.dll",
            linux = "libcublasLt.so.13,libcublas.so.13"
        )
    )]
    Blas13,
    #[strum(
        serialize = "cufft-12",
        props(
            project = "nvidia-cufft/12.0.0.15",
            windows = "cufft64_12.dll",
            linux = "libcufft.so.12"
        )
    )]
    Fft12,
    #[strum(
        serialize = "curand-10",
        props(
            project = "nvidia-curand/10.4.0.35",
            windows = "curand64_10.dll",
            linux = "libcurand.so.10"
        )
    )]
    Rand10,
    #[strum(
        serialize = "cudnn-9.20",
        props(
            project = "nvidia-cudnn-cu13/9.20.0.48",
            windows = "cudnn64_9.dll,cudnn_adv64_9.dll,cudnn_cnn64_9.dll,cudnn_engines_precompiled64_9.dll,cudnn_engines_runtime_compiled64_9.dll,cudnn_graph64_9.dll,cudnn_heuristic64_9.dll,cudnn_ops64_9.dll",
            linux = "libcudnn.so.9,libcudnn_adv.so.9,libcudnn_cnn.so.9,libcudnn_engines_precompiled.so.9,libcudnn_engines_runtime_compiled.so.9,libcudnn_graph.so.9,libcudnn_heuristic.so.9,libcudnn_ops.so.9"
        )
    )]
    Dnn920,
    #[strum(
        serialize = "nvrtc-13",
        props(
            project = "nvidia-cuda-nvrtc/13.0.88",
            windows = "nvrtc-builtins64_130.dll,nvrtc64_130_0.alt.dll,nvrtc64_130_0.dll",
            linux = "libnvrtc-builtins.so.13.0,libnvrtc.so.13"
        )
    )]
    Rtc13,
    #[strum(
        serialize = "nvjitlink-13",
        props(
            project = "nvidia-nvjitlink/13.0.39",
            windows = "nvJitLink_130_0.dll",
            linux = "libnvJitLink.so.13"
        )
    )]
    JitLink13,
    #[strum(
        serialize = "cusparse-12",
        props(
            project = "nvidia-cusparse/12.6.2.49",
            windows = "cusparse64_12.dll",
            linux = "libcusparse.so.12"
        )
    )]
    Sparse12,
    #[strum(
        serialize = "cusolver-12",
        props(
            project = "nvidia-cusolver/12.0.3.29",
            windows = "cusolver64_12.dll,cusolverMg64_12.dll",
            linux = "libcusolver.so.12,libcusolverMg.so.12"
        )
    )]
    Solver12,
}

impl Cuda {
    fn project(self) -> Result<&'static str> {
        let platform = if cfg!(target_os = "windows") {
            "windows_project"
        } else if cfg!(target_os = "linux") {
            "linux_project"
        } else {
            anyhow::bail!("CUDA packages do not support this operating system")
        };
        self.get_str(platform)
            .or_else(|| self.get_str("project"))
            .context("CUDA package has no distribution for this platform")
    }

    fn libraries(self) -> Result<impl Iterator<Item = &'static str>> {
        let platform = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            anyhow::bail!("CUDA packages do not support this operating system")
        };
        Ok(self
            .get_str(platform)
            .context("CUDA package has no libraries for this platform")?
            .split(','))
    }

    fn library_paths(self, root: &Path) -> Result<Vec<PathBuf>> {
        let names = self.libraries()?.collect::<Vec<_>>();
        let mut paths = vec![None; names.len()];
        for entry in WalkDir::new(root) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(index) = names
                .iter()
                .position(|name| entry.file_name() == std::ffi::OsStr::new(name))
            {
                paths[index] = Some(entry.into_path());
            }
        }
        names
            .into_iter()
            .zip(paths)
            .map(|(name, path)| path.with_context(|| format!("missing CUDA library {name}")))
            .collect()
    }
}

impl sealed::Sealed for Cuda {}

impl Package for Cuda {
    async fn install(self) -> Result<PathBuf> {
        let project = self.project()?;
        let target = Store::root().join("cuda").join(project.replace('/', "--"));
        Store::directory(
            target,
            move |path| self.library_paths(path).is_ok(),
            move |stage| async move {
                let url = wheel(project, Platform::host()?).await?;
                let archive = tempfile::Builder::new().suffix(".whl").tempfile()?;
                download::fetch(&url, archive.path()).await?;
                extract(
                    archive.path(),
                    &stage,
                    &["**/*.dll", "**/*.so", "**/*.so.*"],
                )
            },
        )
        .await
    }
}

impl RuntimePackage for Cuda {
    const NAME: &'static str = "CUDA";

    async fn activate(self) -> Result<()> {
        let directory = self.install().await?;
        for library in self.library_paths(&directory)? {
            loader::load(library, false)?;
        }
        Ok(())
    }
}
