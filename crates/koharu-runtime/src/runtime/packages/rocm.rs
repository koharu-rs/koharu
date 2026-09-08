use std::path::{Path, PathBuf};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::ffi::c_void;

use anyhow::{Context, Result};
use strum::IntoEnumIterator;

use crate::{
    Device, Store, download,
    hardware::GfxTarget,
    runtime::{Package, RuntimePackage, sealed},
    source::extract,
};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use crate::runtime::loader;

pub(crate) const VERSION: &str = "10.0.0";
pub(crate) const INDEX: &str = "https://stable.repo.amd.com/rocm/core/whl-next";

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, strum::Display, strum::EnumIter)]
enum Library {
    #[strum(serialize = "_rocm_sdk_core/bin/amd_comgr.dll")]
    Comgr,
    #[strum(serialize = "_rocm_sdk_core/bin/rocm_kpack.dll")]
    Kpack,
    #[strum(serialize = "_rocm_sdk_core/bin/rocm-openblas.dll")]
    OpenBlas,
    #[strum(serialize = "_rocm_sdk_core/bin/amdhip64_7.dll")]
    Hip,
    #[strum(serialize = "_rocm_sdk_core/bin/hiprtc-builtins0715.dll")]
    HipRtcBuiltins,
    #[strum(serialize = "_rocm_sdk_core/bin/hiprtc0715.dll")]
    HipRtc,
    #[strum(serialize = "_rocm_sdk_libraries/bin/rocrand.dll")]
    RocRand,
    #[strum(serialize = "_rocm_sdk_libraries/bin/hiprand.dll")]
    HipRand,
    #[strum(serialize = "_rocm_sdk_libraries/bin/origami.dll")]
    Origami,
    #[strum(serialize = "_rocm_sdk_libraries/bin/libhipblaslt.dll")]
    HipBlasLt,
    #[strum(serialize = "_rocm_sdk_libraries/bin/rocblas.dll")]
    RocBlas,
    #[strum(serialize = "_rocm_sdk_libraries/bin/hipblas.dll")]
    HipBlas,
    #[strum(serialize = "_rocm_sdk_libraries/bin/rocfft.dll")]
    RocFft,
    #[strum(serialize = "_rocm_sdk_libraries/bin/hipfft.dll")]
    HipFft,
    #[strum(serialize = "_rocm_sdk_libraries/bin/rocsolver.dll")]
    RocSolver,
    #[strum(serialize = "_rocm_sdk_libraries/bin/hipsolver.dll")]
    HipSolver,
    #[strum(serialize = "_rocm_sdk_libraries/bin/rocsparse.dll")]
    RocSparse,
    #[strum(serialize = "_rocm_sdk_libraries/bin/hipsparse.dll")]
    HipSparse,
    #[strum(serialize = "_rocm_sdk_libraries/bin/MIOpen.dll")]
    MiOpen,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, strum::Display, strum::EnumIter)]
enum Library {
    #[strum(serialize = "_rocm_sdk_core/lib/librocprofiler-register.so.0")]
    RocProfilerRegister,
    #[strum(serialize = "_rocm_sdk_core/lib/libamd_comgr.so.3")]
    Comgr,
    #[strum(serialize = "_rocm_sdk_core/lib/libhsa-runtime64.so.1")]
    HsaRuntime,
    #[strum(serialize = "_rocm_sdk_core/lib/libamdhip64.so.7")]
    Hip,
    #[strum(serialize = "_rocm_sdk_core/lib/librocprofiler-sdk.so.1")]
    RocProfilerSdk,
    #[strum(serialize = "_rocm_sdk_core/lib/librocprofiler-sdk-roctx.so.1")]
    RocProfilerSdkRoctx,
    #[strum(serialize = "_rocm_sdk_core/lib/libroctracer64.so.4")]
    RocTracer,
    #[strum(serialize = "_rocm_sdk_core/lib/libroctx64.so.4")]
    RocTx,
    #[strum(serialize = "_rocm_sdk_core/lib/libhiprtc-builtins.so.7")]
    HipRtcBuiltins,
    #[strum(serialize = "_rocm_sdk_core/lib/libhiprtc.so.7")]
    HipRtc,
    #[strum(serialize = "_rocm_sdk_core/lib/rocm_sysdeps/lib/librocm_sysdeps_liblzma.so.5")]
    Lzma,
    #[strum(serialize = "_rocm_sdk_core/lib/host-math/lib/librocm-openblas.so.0")]
    OpenBlas,
    #[strum(serialize = "_rocm_sdk_core/lib/librocm_smi64.so.1")]
    RocmSmi,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librocroller.so.1")]
    RocRoller,
    #[strum(serialize = "_rocm_sdk_libraries/lib/liborigami.so.1")]
    Origami,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipblaslt.so.1")]
    HipBlasLt,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librocblas.so.5")]
    RocBlas,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipblas.so.3")]
    HipBlas,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librocfft.so.0")]
    RocFft,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipfft.so.0")]
    HipFft,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librocrand.so.1")]
    RocRand,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhiprand.so.1")]
    HipRand,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librocsolver.so.0")]
    RocSolver,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipsolver.so.1")]
    HipSolver,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librocsparse.so.1")]
    RocSparse,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipsparse.so.4")]
    HipSparse,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipsparselt.so.0")]
    HipSparseLt,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libMIOpen.so.1")]
    MiOpen,
    #[strum(serialize = "_rocm_sdk_libraries/lib/libhipdnn_backend.so")]
    HipDnnBackend,
    #[strum(serialize = "_rocm_sdk_libraries/lib/librccl.so.1")]
    Rccl,
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[derive(Clone, Copy, strum::Display, strum::EnumIter)]
enum Library {
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Rocm(pub(crate) GfxTarget);

impl std::fmt::Display for Rocm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Rocm {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn probe(self, library: &libloading::Library) -> Result<usize> {
        type GetDeviceCount = unsafe extern "C" fn(*mut i32) -> i32;
        type GetDeviceProperties = unsafe extern "C" fn(*mut c_void, i32) -> i32;

        #[repr(C, align(64))]
        struct Properties([u8; 64 * 1024]);

        let get_device_count = unsafe { *library.get::<GetDeviceCount>(b"hipGetDeviceCount\0")? };
        let get_device_properties =
            unsafe { *library.get::<GetDeviceProperties>(b"hipGetDevicePropertiesR0600\0")? };
        let mut count = 0;
        let status = unsafe { get_device_count(&mut count) };
        if status != 0 || !(0..=128).contains(&count) {
            anyhow::bail!("bundled ROCm reported status {status} and {count} devices");
        }

        let expected = self.to_string();
        for index in 0..count {
            let mut properties = Box::new(Properties([0; 64 * 1024]));
            if unsafe { get_device_properties(properties.0.as_mut_ptr().cast(), index) } != 0 {
                continue;
            }
            let target = properties
                .0
                .windows(3)
                .enumerate()
                .find_map(|(start, bytes)| {
                    if bytes != b"gfx" {
                        return None;
                    }
                    let suffix = properties.0[start + 3..]
                        .iter()
                        .take_while(|byte| byte.is_ascii_alphanumeric())
                        .count();
                    let target =
                        std::str::from_utf8(&properties.0[start..start + 3 + suffix]).ok()?;
                    target[3..]
                        .bytes()
                        .any(|byte| byte.is_ascii_digit())
                        .then_some(target)
                });
            if target != Some(expected.as_str()) {
                continue;
            }
            return Ok(index as usize);
        }
        anyhow::bail!("bundled ROCm cannot use {self}")
    }

    fn complete(self, path: &Path) -> bool {
        #[cfg(target_os = "windows")]
        let device_library = path
            .join("_rocm_sdk_libraries/.kpack")
            .join(format!("blas_lib_{self}.kpack"))
            .is_file();
        #[cfg(target_os = "linux")]
        let device_library = {
            let root = path.join("_rocm_sdk_libraries/lib/rocblas/library");
            match self.0 {
                GfxTarget::Gfx90a => {
                    let root = root.join("gfx90a");
                    root.join("Kernels.so-000-gfx90a-xnack+.hsaco").is_file()
                        && root.join("Kernels.so-000-gfx90a-xnack-.hsaco").is_file()
                }
                GfxTarget::Gfx908
                | GfxTarget::Gfx942
                | GfxTarget::Gfx950
                | GfxTarget::Gfx1150
                | GfxTarget::Gfx1151
                | GfxTarget::Gfx1152 => root
                    .join(self.to_string())
                    .join(format!("Kernels.so-000-{self}.hsaco"))
                    .is_file(),
                _ => root.join(format!("Kernels.so-000-{self}.hsaco")).is_file(),
            }
        };
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let device_library = false;
        Library::iter().all(|library| path.join(library.to_string()).is_file()) && device_library
    }
}

impl sealed::Sealed for Rocm {}

impl Package for Rocm {
    async fn install(self) -> Result<PathBuf> {
        let (platform, target) = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            ("win_amd64", "x86_64-pc-windows-msvc")
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            ("linux_x86_64", "x86_64-unknown-linux-gnu")
        } else {
            anyhow::bail!("ROCm packages support only Windows and Linux x86_64")
        };
        let path = Store::root()
            .join("rocm")
            .join(VERSION)
            .join(target)
            .join(self.to_string());
        Store::directory(
            path,
            move |path| self.complete(path),
            move |stage| async move {
                for (url, pattern) in [
                    (
                        format!(
                            "{INDEX}/rocm-sdk-core/rocm_sdk_core-{VERSION}-py3-none-{platform}.whl"
                        ),
                        "_rocm_sdk_core/**/*",
                    ),
                    (
                        format!(
                            "{INDEX}/rocm-sdk-libraries/rocm_sdk_libraries-{VERSION}-py3-none-{platform}.whl"
                        ),
                        "_rocm_sdk_libraries/**/*",
                    ),
                    (
                        format!(
                            "{INDEX}/rocm-sdk-device-{self}/rocm_sdk_device_{self}-{VERSION}-py3-none-{platform}.whl"
                        ),
                        "_rocm_sdk_libraries/**/*",
                    ),
                ] {
                    let archive = tempfile::Builder::new().suffix(".whl").tempfile()?;
                    download::fetch(&url, archive.path()).await?;
                    extract(archive.path(), &stage, &[pattern])?;
                }
                Ok(())
            },
        )
        .await
    }
}

impl RuntimePackage for Rocm {
    const NAME: &'static str = "ROCm";

    async fn activate(self, device: &mut Device) -> Result<()> {
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            let root = self.install().await?;
            let mut hip = None;
            for library in Library::iter() {
                let loaded = loader::load(root.join(library.to_string()), true)?;
                if matches!(library, Library::Hip) {
                    hip = Some(loaded);
                }
            }
            device.index = self.probe(hip.context("ROCm manifest has no HIP library")?)?;
            device.name = format!("ROCm{}", device.index);
            Ok(())
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = device;
            anyhow::bail!("ROCm packages support only Windows and Linux")
        }
    }
}
