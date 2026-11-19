//! Intel's oneAPI runtime libraries that libtorch's XPU build needs, from
//! their wheels on PyPI at the versions PyTorch's own XPU wheel requires.

use std::path::PathBuf;

use anyhow::Result;
use futures::future::BoxFuture;

use super::{Host, Package, extract, preload};
use crate::{Backend, Device, store};

pub(super) struct OneApi;

/// The SYCL runtime's version, which names the set.
const VERSION: &str = "2026.1.2";

/// Each wheel's project and version. The others PyTorch requires hold only
/// licenses, or libraries Koharu's libtorch doesn't link: OpenCL, PTI, and
/// oneMKL's RNG and sparse domains.
const WHEELS: &[(&str, &str)] = &[
    ("tcmlib", "1.5.0"),
    ("umf", "1.1.0"),
    ("intel-cmplr-lib-rt", "2026.1.2"),
    ("intel-cmplr-lib-ur", "2026.1.2"),
    ("intel-sycl-rt", "2026.1.2"),
    ("intel-openmp", "2026.1.2"),
    ("tbb", "2023.1.0"),
    ("mkl", "2026.1.0"),
    ("onemkl-sycl-blas", "2026.1.0"),
    ("onemkl-sycl-dft", "2026.1.0"),
    ("onemkl-sycl-lapack", "2026.1.0"),
];

/// The libraries libtorch needs, in load order.
const LOAD: &[&str] = &[
    "Library/bin/libhwloc-15.dll",
    "Library/bin/umf.dll",
    "Library/bin/libmmd.dll",
    "Library/bin/ur_loader.dll",
    "Library/bin/ur_win_proxy_loader.dll",
    "Library/bin/sycl9.dll",
    "Library/bin/libiomp5md.dll",
    "Library/bin/tbb12.dll",
    "Library/bin/mkl_core.3.dll",
    "Library/bin/mkl_intel_thread.3.dll",
    "Library/bin/mkl_tbb_thread.3.dll",
    "Library/bin/mkl_sycl_blas.6.dll",
    "Library/bin/mkl_sycl_dft.6.dll",
    "Library/bin/mkl_sycl_lapack.6.dll",
];

/// What the store keeps: the libraries, and those others find in their own
/// folder. The Unified Runtime loader finds its Level Zero adapters there,
/// and MKL's core its CPU kernels.
const FILES: &[&str] = &[
    "Library/bin/libhwloc-15.dll",
    "Library/bin/umf.dll",
    "Library/bin/libmmd.dll",
    "Library/bin/ur_loader.dll",
    "Library/bin/ur_win_proxy_loader.dll",
    "Library/bin/ur_adapter_level_zero.dll",
    "Library/bin/ur_adapter_level_zero_v2.dll",
    "Library/bin/sycl9.dll",
    "Library/bin/libiomp5md.dll",
    "Library/bin/tbb12.dll",
    "Library/bin/mkl_core.3.dll",
    "Library/bin/mkl_intel_thread.3.dll",
    "Library/bin/mkl_tbb_thread.3.dll",
    "Library/bin/mkl_def.3.dll",
    "Library/bin/mkl_mc3.3.dll",
    "Library/bin/mkl_avx2.3.dll",
    "Library/bin/mkl_avx512.3.dll",
    "Library/bin/mkl_avx10.3.dll",
    "Library/bin/mkl_vml_*.3.dll",
    "Library/bin/mkl_sycl_blas.6.dll",
    "Library/bin/mkl_sycl_dft.6.dll",
    "Library/bin/mkl_sycl_lapack.6.dll",
];

fn folder() -> PathBuf {
    store::root().join("oneapi").join(VERSION)
}

impl Package for OneApi {
    fn download(&self, host: Host, _: Backend) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            anyhow::ensure!(
                host == Host::WindowsX64,
                "Koharu's XPU build exists only for Windows on x86-64"
            );
            store::put(
                &folder(),
                folder(),
                // Complete with every file it keeps, not just those Koharu
                // loads: SYCL can't find the GPU without its Level Zero
                // adapters. A pattern names no one file to check.
                |folder| {
                    FILES
                        .iter()
                        .filter(|file| !file.contains('*'))
                        .all(|file| folder.join(file).is_file())
                },
                |stage| async move {
                    for (project, version) in WHEELS {
                        // PyPI's stable path, which redirects to where it's
                        // stored. Only TBB's wheel is tagged for Python 3
                        // alone.
                        let python = if *project == "tbb" { "py3" } else { "py2.py3" };
                        let url = format!(
                            "https://files.pythonhosted.org/packages/{python}/{}/{project}/{}-{version}-{python}-none-win_amd64.whl",
                            &project[..1],
                            project.replace('-', "_"),
                        );
                        extract(&url, &stage, FILES).await?;
                    }
                    Ok(stage)
                },
            )
            .await?;
            Ok(())
        })
    }

    fn load(&self, _: Host, _: &mut Device) -> Result<()> {
        for library in LOAD {
            preload(&folder().join(library), false)?;
        }
        Ok(())
    }

    fn versions(&self) -> Vec<PathBuf> {
        vec![folder()]
    }
}
