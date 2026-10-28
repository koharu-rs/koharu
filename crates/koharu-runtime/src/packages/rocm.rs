//! AMD's ROCm libraries, from TheRock's wheels. AMD's index publishes no
//! hashes, so each wheel's SHA-256 was computed from the file it serves.

use std::{
    ffi::c_void,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::{Host, install_archive, preload};
use crate::{
    Device,
    store::{Store, remove_others},
};

const VERSION: &str = "10.0.0";
const INDEX: &str = "https://stable.repo.amd.com/rocm/core/whl-next";

const CORE: [(Host, &str); 2] = [
    (
        Host::WindowsX64,
        "066195cb0f7df02e6009facb17652dd992d13fae44d87652e5ab928d5b4f0851",
    ),
    (
        Host::LinuxX64,
        "3bf4a72d11aa2a4ee1e90572c73630f937d38c1b7af4dda45b483b54655138a7",
    ),
];

const LIBRARIES: [(Host, &str); 2] = [
    (
        Host::WindowsX64,
        "bfc7a1bd3b6050fb4d36d8ac2eea80d3afd70c5604a29142463d6cb890fc73bf",
    ),
    (
        Host::LinuxX64,
        "bbe83a550cb8ceeb306abf6d57df6617857a1b19384549526db2ec81a4374e57",
    ),
];

/// Each target's kernels.
const DEVICES: &[(Host, &str, &str)] = &[
    (
        Host::WindowsX64,
        "gfx1010",
        "2b6da925376de9689ab9f22ce7cc55b6d476722d9458fbd1ec52d4aa066d3e6c",
    ),
    (
        Host::WindowsX64,
        "gfx1011",
        "1a6de3efe7bc92b1e55367a767eecf0468231b03d9319494d1e4b077571b6757",
    ),
    (
        Host::WindowsX64,
        "gfx1012",
        "51f2998d2f33164e4a14594107e6c33db54da1d9867fa6947ea993997c4ae06e",
    ),
    (
        Host::WindowsX64,
        "gfx1030",
        "19ada0359b499c19c1ecb1cba21b067fce89b4f0b0de6f0f7b76cbef85dcb85b",
    ),
    (
        Host::WindowsX64,
        "gfx1031",
        "ab7cbfb41e1ada00fb7254c974f7386303cdd1139a170fb022e84e6f10d9f5f2",
    ),
    (
        Host::WindowsX64,
        "gfx1032",
        "5d0581e077cc1a482e70430539c63e60c4b4d7b9b82be0ac0bab6d62c3feb4de",
    ),
    (
        Host::WindowsX64,
        "gfx1033",
        "46ba5b395bf75756f9c5f1154b2083dd576221dfbd15620b5fa597f498aad798",
    ),
    (
        Host::WindowsX64,
        "gfx1034",
        "8d527c2f127d42ecb098e0abc45c15ca28689b6317f01b99d455e6d3562a3b52",
    ),
    (
        Host::WindowsX64,
        "gfx1035",
        "c06479bad4963f4aecde4e0fe102eb39ae652e80da62546c3bd3fb0b9079a0a4",
    ),
    (
        Host::WindowsX64,
        "gfx1036",
        "655f343cd805f21bfd45244902f99afba9b7ada323288a1b9ba3336bd41c1ccc",
    ),
    (
        Host::WindowsX64,
        "gfx1100",
        "db39cbc5aae7a67c73d76692899a6a89bbc10eafc92f856f48e9c03247a91f8e",
    ),
    (
        Host::WindowsX64,
        "gfx1101",
        "7b8edd82943a11c29c80f84c95822e36ddd794b7024878e9660170d050a29243",
    ),
    (
        Host::WindowsX64,
        "gfx1102",
        "b9e0f3fa492f0a05a5c4f479b80915477911f8402956c0e2c593b593d6214531",
    ),
    (
        Host::WindowsX64,
        "gfx1103",
        "3de6ba204d78e0cb2a32cf064371c181d7a17d1ca8b25537b1a7f11eaa66fa17",
    ),
    (
        Host::WindowsX64,
        "gfx1150",
        "22c19fcee580102cdc2f9379edf056643843c86ac6312dcf34d8ed21964c2cc4",
    ),
    (
        Host::WindowsX64,
        "gfx1151",
        "1ab27ebfe61ea3c3fd2c5c1a0e780ae22ed9b662106ce0aefda0a30e3b11577f",
    ),
    (
        Host::WindowsX64,
        "gfx1152",
        "88db4e79f31bef68bf8cb321fb83704dda6d11ad6da8dcfbdc8660f077c7195c",
    ),
    (
        Host::WindowsX64,
        "gfx1153",
        "1fde64d87edd815403014743700000a016435f040c43114e764b5cc7b919413c",
    ),
    (
        Host::WindowsX64,
        "gfx1200",
        "f33226078932ee2200f5595c521895edf5d7abf3170975d8c6f72a2a3949f516",
    ),
    (
        Host::WindowsX64,
        "gfx1201",
        "c6e4c5a0da941230bed975765e644597570081c9a41654d5a052c277167f4576",
    ),
    (
        Host::LinuxX64,
        "gfx908",
        "35fff75186b81ff46499522928abb4d46efe53cbe20b8e53c40941f28c62c7ec",
    ),
    (
        Host::LinuxX64,
        "gfx90a",
        "8bb0e955378aae4e7962ddd55661782e9e2843101b3c15dd6f9a0c7568d4b0bb",
    ),
    (
        Host::LinuxX64,
        "gfx942",
        "636f4e4c3110e4e1ccc9eacb08ef39e08f635ba9472ead2b000bedd76bfea8bd",
    ),
    (
        Host::LinuxX64,
        "gfx950",
        "01604bedb4acec867e7bbc18c4580ed15aa056fc6b151d976746a43305a8a09e",
    ),
    (
        Host::LinuxX64,
        "gfx1010",
        "d79dae603d40bd9301c896d848ca022e7ed02c860c23041af69d0b86c92cc029",
    ),
    (
        Host::LinuxX64,
        "gfx1011",
        "923d74f1912938defda23c8ddadd175b0ffc0cabb6082c3121f03bfad1d16ff3",
    ),
    (
        Host::LinuxX64,
        "gfx1012",
        "93de1ca0efd82ef3bdf9143596b0d6a0ee3dbc7dd41fac0afc88d2d405b4c924",
    ),
    (
        Host::LinuxX64,
        "gfx1030",
        "493cf38e23d970afab6e21df09a68ce678a5d28a8107d7fd2841c90a557369f6",
    ),
    (
        Host::LinuxX64,
        "gfx1031",
        "d6b1ac96507c4e79828cf447b14e25f797a14bb4e3d1b1681789323d9a0b05ea",
    ),
    (
        Host::LinuxX64,
        "gfx1032",
        "5606f831643a31426090590c6bab375f391051b55aed21195c2196051aa3eb97",
    ),
    (
        Host::LinuxX64,
        "gfx1033",
        "7fb7d4b80f1de69056deb8f6384d0e2d65257600f53df37cae90f2bbece9faed",
    ),
    (
        Host::LinuxX64,
        "gfx1034",
        "96d7368e12256f7d2ea2291f7e426a57e2ce2e27da3e6ace838a597173d1355c",
    ),
    (
        Host::LinuxX64,
        "gfx1035",
        "516598b1aace5f2b5b551f54efd134313b3cddd126203dd648a87a0bf19d7da3",
    ),
    (
        Host::LinuxX64,
        "gfx1036",
        "8de22ec33d8dd9564be9555672187cd0748954c42b9f13036050c1c111d232d2",
    ),
    (
        Host::LinuxX64,
        "gfx1100",
        "d44af24724dd1a104a973a8dbc2eb514c1a0c155ac2cf5bf38cc9ab5846f8f29",
    ),
    (
        Host::LinuxX64,
        "gfx1101",
        "3e5c648e0b0291863536732bc3e90198903b2a4907e2806d1b41e8fb4f9d4a2d",
    ),
    (
        Host::LinuxX64,
        "gfx1102",
        "dbfc6d85cd64c625555084714302467d8a46a98c05bba9f62f91b49e178af901",
    ),
    (
        Host::LinuxX64,
        "gfx1150",
        "64aaa7e396bb89372eba17293e63a66f8fc0a3ff9bf1d86dc6d1eb8b57bd03e9",
    ),
    (
        Host::LinuxX64,
        "gfx1151",
        "ec705df531e523777a5cd47b2dfadb2272935927c46c122effb11fcdcedb73d0",
    ),
    (
        Host::LinuxX64,
        "gfx1152",
        "0cd4ac7751be934d01e20bf9077e57282b805b733c37c6b51305d1b0ca3be4e0",
    ),
    (
        Host::LinuxX64,
        "gfx1200",
        "3b638ed38afa08cc700c8853a7f1390e0e37175cc926fc0ecb743c1d772dc26d",
    ),
    (
        Host::LinuxX64,
        "gfx1201",
        "81856692c18e5d6ccce486193d0f4a54b368d2e84ff5a0811b644f6f788ed4b2",
    ),
];

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

/// Installs ROCm with `device`'s kernels. Libraries come with their data
/// files, such as rocBLAS's kernels, so each wheel's tree is kept whole.
pub(super) async fn install(host: Host, device: &Device) -> Result<PathBuf> {
    let target = device.target().context("a ROCm device has a target")?;
    let platform = match host {
        Host::WindowsX64 => "win_amd64",
        Host::LinuxX64 => "linux_x86_64",
        _ => anyhow::bail!("ROCm wheels exist only for Windows and Linux on x86-64"),
    };
    let pick = |wheels: &[(Host, &'static str)]| {
        wheels
            .iter()
            .find(|(platform, _)| *platform == host)
            .map(|(_, sha256)| *sha256)
    };
    let core = pick(&CORE).context("no ROCm core for this platform")?;
    let libraries = pick(&LIBRARIES).context("no ROCm libraries for this platform")?;
    let kernels = DEVICES
        .iter()
        .find(|(platform, name, _)| *platform == host && *name == target)
        .map(|(_, _, sha256)| *sha256)
        .with_context(|| format!("ROCm {VERSION} has no kernels for {target}"))?;
    let target = target.to_owned();
    Store::directory(
        Store::root().join("rocm").join(VERSION).join(&target),
        |folder| LOAD.iter().all(|library| folder.join(library).is_file()),
        move |stage| async move {
            let wheel = |project: &str, name: &str| {
                format!("{INDEX}/{project}/{name}-{VERSION}-py3-none-{platform}.whl")
            };
            install_archive(
                &wheel("rocm-sdk-core", "rocm_sdk_core"),
                core,
                &stage,
                &["_rocm_sdk_core/**/*"],
            )
            .await?;
            install_archive(
                &wheel("rocm-sdk-libraries", "rocm_sdk_libraries"),
                libraries,
                &stage,
                &["_rocm_sdk_libraries/**/*"],
            )
            .await?;
            install_archive(
                &wheel(
                    &format!("rocm-sdk-device-{target}"),
                    &format!("rocm_sdk_device_{target}"),
                ),
                kernels,
                &stage,
                &["_rocm_sdk_libraries/**/*"],
            )
            .await
        },
    )
    .await
}

/// Loads ROCm and returns `device`'s index among the GPUs HIP lists, which
/// can differ from the order the system probe saw.
pub(super) fn load(folder: &Path, device: &Device) -> Result<usize> {
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

    let expected = device.target().context("a ROCm device has a target")?;
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

pub(super) async fn remove_older() {
    remove_others(
        Store::root().join("rocm"),
        String::new(),
        VERSION.to_owned(),
    )
    .await;
}
