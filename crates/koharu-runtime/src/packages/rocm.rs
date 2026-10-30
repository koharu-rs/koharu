//! AMD's ROCm libraries, from TheRock's wheels.

use std::{
    ffi::c_void,
    path::{Path, PathBuf},
};

use anyhow::Result;

use super::{Host, install_archive, preload};
use crate::store;

const VERSION: &str = "10.0.0";
const INDEX: &str = "https://stable.repo.amd.com/rocm/core/whl-next";

/// The libraries the others need, in load order.
#[cfg(windows)]
const LOAD: &[&str] = &[
    "_rocm_sdk_core/bin/amd_comgr.dll",
    "_rocm_sdk_core/bin/rocm_kpack.dll",
    "_rocm_sdk_core/bin/rocm-openblas.dll",
    "_rocm_sdk_core/bin/amdhip64_7.dll",
    "_rocm_sdk_core/bin/hiprtc-builtins0715.dll",
    "_rocm_sdk_core/bin/hiprtc0715.dll",
    "_rocm_sdk_libraries/bin/rocrand.dll",
    "_rocm_sdk_libraries/bin/hiprand.dll",
    "_rocm_sdk_libraries/bin/origami.dll",
    "_rocm_sdk_libraries/bin/libhipblaslt.dll",
    "_rocm_sdk_libraries/bin/rocblas.dll",
    "_rocm_sdk_libraries/bin/hipblas.dll",
    "_rocm_sdk_libraries/bin/rocfft.dll",
    "_rocm_sdk_libraries/bin/hipfft.dll",
    "_rocm_sdk_libraries/bin/rocsolver.dll",
    "_rocm_sdk_libraries/bin/hipsolver.dll",
    "_rocm_sdk_libraries/bin/rocsparse.dll",
    "_rocm_sdk_libraries/bin/hipsparse.dll",
    "_rocm_sdk_libraries/bin/MIOpen.dll",
];

#[cfg(not(windows))]
const LOAD: &[&str] = &[
    "_rocm_sdk_core/lib/librocprofiler-register.so.0",
    "_rocm_sdk_core/lib/libamd_comgr.so.3",
    "_rocm_sdk_core/lib/libhsa-runtime64.so.1",
    "_rocm_sdk_core/lib/libamdhip64.so.7",
    "_rocm_sdk_core/lib/librocprofiler-sdk.so.1",
    "_rocm_sdk_core/lib/librocprofiler-sdk-roctx.so.1",
    "_rocm_sdk_core/lib/libroctracer64.so.4",
    "_rocm_sdk_core/lib/libroctx64.so.4",
    "_rocm_sdk_core/lib/libhiprtc-builtins.so.7",
    "_rocm_sdk_core/lib/libhiprtc.so.7",
    "_rocm_sdk_core/lib/rocm_sysdeps/lib/librocm_sysdeps_liblzma.so.5",
    "_rocm_sdk_core/lib/host-math/lib/librocm-openblas.so.0",
    "_rocm_sdk_core/lib/librocm_smi64.so.1",
    "_rocm_sdk_libraries/lib/librocroller.so.1",
    "_rocm_sdk_libraries/lib/liborigami.so.1",
    "_rocm_sdk_libraries/lib/libhipblaslt.so.1",
    "_rocm_sdk_libraries/lib/librocblas.so.5",
    "_rocm_sdk_libraries/lib/libhipblas.so.3",
    "_rocm_sdk_libraries/lib/librocfft.so.0",
    "_rocm_sdk_libraries/lib/libhipfft.so.0",
    "_rocm_sdk_libraries/lib/librocrand.so.1",
    "_rocm_sdk_libraries/lib/libhiprand.so.1",
    "_rocm_sdk_libraries/lib/librocsolver.so.0",
    "_rocm_sdk_libraries/lib/libhipsolver.so.1",
    "_rocm_sdk_libraries/lib/librocsparse.so.1",
    "_rocm_sdk_libraries/lib/libhipsparse.so.4",
    "_rocm_sdk_libraries/lib/libhipsparselt.so.0",
    "_rocm_sdk_libraries/lib/libMIOpen.so.1",
    "_rocm_sdk_libraries/lib/libhipdnn_backend.so",
    "_rocm_sdk_libraries/lib/librccl.so.1",
];

#[cfg(windows)]
const HIP: &str = "_rocm_sdk_core/bin/amdhip64_7.dll";
#[cfg(not(windows))]
const HIP: &str = "_rocm_sdk_core/lib/libamdhip64.so.7";

/// Installs ROCm with `target`'s kernels. Libraries come with their data
/// files, such as rocBLAS's kernels, so each wheel's tree is kept whole.
pub(super) async fn install(host: Host, target: &'static str) -> Result<PathBuf> {
    let platform = match host {
        Host::WindowsX64 => "win_amd64",
        Host::LinuxX64 => "linux_x86_64",
        _ => anyhow::bail!("ROCm wheels exist only for Windows and Linux on x86-64"),
    };
    store::directory(
        store::root().join("rocm").join(VERSION).join(target),
        |folder| LOAD.iter().all(|library| folder.join(library).is_file()),
        move |stage| async move {
            let wheel = |project: &str, name: &str| {
                format!("{INDEX}/{project}/{name}-{VERSION}-py3-none-{platform}.whl")
            };
            install_archive(
                &wheel("rocm-sdk-core", "rocm_sdk_core"),
                &stage,
                &["_rocm_sdk_core/**/*"],
            )
            .await?;
            install_archive(
                &wheel("rocm-sdk-libraries", "rocm_sdk_libraries"),
                &stage,
                &["_rocm_sdk_libraries/**/*"],
            )
            .await?;
            install_archive(
                &wheel(
                    &format!("rocm-sdk-device-{target}"),
                    &format!("rocm_sdk_device_{target}"),
                ),
                &stage,
                &["_rocm_sdk_libraries/**/*"],
            )
            .await
        },
    )
    .await
}

/// Loads ROCm and returns the index of the GPU with `expected` as its target
/// among the GPUs HIP lists, which can differ from the order the system probe
/// saw.
pub(super) fn load(folder: &Path, expected: &str) -> Result<usize> {
    for library in LOAD {
        preload(&folder.join(library), true)?;
    }

    type GetDeviceCount = unsafe extern "C" fn(*mut i32) -> i32;
    type GetDeviceProperties = unsafe extern "C" fn(*mut c_void, i32) -> i32;

    #[repr(C, align(64))]
    struct Properties([u8; 64 * 1024]);

    // HIP is already loaded, so this opens the same image.
    let library = unsafe { libloading::Library::new(folder.join(HIP)) }?;
    let get_device_count = unsafe { *library.get::<GetDeviceCount>(b"hipGetDeviceCount\0")? };
    let get_device_properties =
        unsafe { *library.get::<GetDeviceProperties>(b"hipGetDevicePropertiesR0600\0")? };
    let mut count = 0;
    let status = unsafe { get_device_count(&mut count) };
    anyhow::ensure!(
        status == 0 && (0..=128).contains(&count),
        "HIP reported status {status} and {count} devices"
    );

    for index in 0..count {
        let mut properties = Box::new(Properties([0; 64 * 1024]));
        if unsafe { get_device_properties(properties.0.as_mut_ptr().cast(), index) } != 0 {
            continue;
        }
        // The properties' architecture name, such as `gfx1100:sramecc-:xnack-`.
        let target = properties
            .0
            .windows(3)
            .enumerate()
            .find_map(|(start, bytes)| {
                if bytes != b"gfx" {
                    return None;
                }
                let length = properties.0[start + 3..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_alphanumeric())
                    .count();
                let target = std::str::from_utf8(&properties.0[start..start + 3 + length]).ok()?;
                target[3..]
                    .bytes()
                    .any(|byte| byte.is_ascii_digit())
                    .then_some(target)
            });
        if target == Some(expected) {
            return Ok(index as usize);
        }
    }
    anyhow::bail!("HIP lists no {expected}")
}
