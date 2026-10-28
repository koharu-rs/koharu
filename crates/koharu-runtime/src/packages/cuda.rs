//! NVIDIA's CUDA libraries, from their wheels on PyPI, pinned to the SHA-256
//! PyPI records for each.

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{Host, LIBRARIES, find, install_archive, preload};
use crate::store::{Store, remove_others};

struct Wheel {
    project: &'static str,
    version: &'static str,
    /// The libraries it holds, in load order.
    windows: &'static [&'static str],
    linux: &'static [&'static str],
    /// The wheel for each platform and its SHA-256.
    files: [(Host, &'static str, &'static str); 3],
}

/// In load order: each wheel comes after the ones its libraries need.
const WHEELS: &[Wheel] = &[
    Wheel {
        project: "nvidia-cuda-runtime",
        version: "13.3.29",
        windows: &["cudart64_13.dll"],
        linux: &["libcudart.so.13"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cuda_runtime-13.3.29-py3-none-win_amd64.whl",
                "0667ec61c3d897388efa305ed4f7609ace88849a753ba9c6311d06dca55fff4f",
            ),
            (
                Host::LinuxX64,
                "nvidia_cuda_runtime-13.3.29-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
                "e04420616e72f563167a7733272992d7e6df6dc5cb54b2f94f9f1520ea9e30c1",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cuda_runtime-13.3.29-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl",
                "73291e19c9dd919c140c91bda2f80b0eca487da5ee30a086ef7bc4918ecb90ea",
            ),
        ],
    },
    Wheel {
        project: "nvidia-nvjitlink",
        version: "13.3.33",
        windows: &["nvJitLink_130_0.dll"],
        linux: &["libnvJitLink.so.13"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_nvjitlink-13.3.33-py3-none-win_amd64.whl",
                "4297ee49639b4f2e07255a1d69b3acc7ab2d011bb892b403e91ac98368962e3b",
            ),
            (
                Host::LinuxX64,
                "nvidia_nvjitlink-13.3.33-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl",
                "26a6de7fb4c8fdaa7703d3dad720d6d427ddfea5c48a528fd97c11733ad830e5",
            ),
            (
                Host::LinuxArm64,
                "nvidia_nvjitlink-13.3.33-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl",
                "ce48b37dfeb3cb1eae4cf85adacb47d7a6539ea2272870c9a3628ce275c2037e",
            ),
        ],
    },
    Wheel {
        project: "nvidia-cuda-nvrtc",
        version: "13.3.33",
        windows: &["nvrtc-builtins64_133.dll", "nvrtc64_130_0.dll"],
        linux: &["libnvrtc-builtins.so.13.3", "libnvrtc.so.13"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cuda_nvrtc-13.3.33-py3-none-win_amd64.whl",
                "7d2af818851c0c224d5f92221e9226e51ee23c236df4b51f9194563979c888be",
            ),
            (
                Host::LinuxX64,
                "nvidia_cuda_nvrtc-13.3.33-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl",
                "82530788b8c6164a54d3fd9ae8bcca8893d397c4aeb998861982a03bbe41e204",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cuda_nvrtc-13.3.33-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl",
                "7b05ecda494c6dabc44231a608b060a71008a730d9dfda932cc508e6d29159e0",
            ),
        ],
    },
    Wheel {
        project: "nvidia-cublas",
        version: "13.6.0.2",
        windows: &["cublasLt64_13.dll", "cublas64_13.dll"],
        linux: &["libcublasLt.so.13", "libcublas.so.13"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cublas-13.6.0.2-py3-none-win_amd64.whl",
                "3b5bcd6bfb6f65010ebf195851bcb9b2aa34b9fe08479432002991c1fe84b67d",
            ),
            (
                Host::LinuxX64,
                "nvidia_cublas-13.6.0.2-py3-none-manylinux_2_27_x86_64.whl",
                "b82c80c886cea6da6e149a5c3bdba274f12b7e4ec4b00a050b916b0446fb4153",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cublas-13.6.0.2-py3-none-manylinux_2_27_aarch64.whl",
                "946f6a252b1cc72d8de912c75975fd6d8ba44f67d4e5044fe764ddb909f4a688",
            ),
        ],
    },
    Wheel {
        project: "nvidia-cufft",
        version: "12.3.0.29",
        windows: &["cufft64_12.dll"],
        linux: &["libcufft.so.12"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cufft-12.3.0.29-py3-none-win_amd64.whl",
                "510036a2bbab5c83ae93dc5c907c3a49d3518e3066ac3a2052ff0f7f9b27dfc4",
            ),
            (
                Host::LinuxX64,
                "nvidia_cufft-12.3.0.29-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
                "edb25c0626bd202ee5acc035b5dd361a3b89ed3b75a81a52df72c89150cb57c2",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cufft-12.3.0.29-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl",
                "3c268ec73fdbfd22e42913dcef6e49e0c305ab09cd9848268a841cbe9f33f748",
            ),
        ],
    },
    Wheel {
        project: "nvidia-curand",
        version: "10.4.3.29",
        windows: &["curand64_10.dll"],
        linux: &["libcurand.so.10"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_curand-10.4.3.29-py3-none-win_amd64.whl",
                "34b18d5a2a8e5db4c3846475ae4eef0cacdf3ac5e9c501f3a4efb422f137a74e",
            ),
            (
                Host::LinuxX64,
                "nvidia_curand-10.4.3.29-py3-none-manylinux_2_27_x86_64.whl",
                "1859bf37a62754d2c65001393096ca79de399f995971fa7826d0adfd88c3cf7b",
            ),
            (
                Host::LinuxArm64,
                "nvidia_curand-10.4.3.29-py3-none-manylinux_2_27_aarch64.whl",
                "63899de834d1b84b32ad60d34cb4d246bea998ddadbc85f8e1dd937dd3718d61",
            ),
        ],
    },
    Wheel {
        project: "nvidia-cusparse",
        version: "12.8.2.51",
        windows: &["cusparse64_12.dll"],
        linux: &["libcusparse.so.12"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cusparse-12.8.2.51-py3-none-win_amd64.whl",
                "2ee59291cd362038f3d40d57c7cd09b26d689f3873ae5c94b31c3270772d41b8",
            ),
            (
                Host::LinuxX64,
                "nvidia_cusparse-12.8.2.51-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
                "65cbcc4e37a34fca4ee7df2fd57da103593842cda1bbb4a144664ecfe59873a5",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cusparse-12.8.2.51-py3-none-manylinux2014_aarch64.manylinux_2_17_aarch64.whl",
                "00469fcf62c4d464a1225abd9b20864ecff35e3fbc9fb992572e83d358927755",
            ),
        ],
    },
    Wheel {
        project: "nvidia-cusolver",
        version: "12.2.6.9",
        windows: &["cusolver64_12.dll", "cusolverMg64_12.dll"],
        linux: &["libcusolver.so.12", "libcusolverMg.so.12"],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cusolver-12.2.6.9-py3-none-win_amd64.whl",
                "1dbb828e0fb27701c084a8fb80f770c562cb9a6d29b6f001320e6f542c6d024f",
            ),
            (
                Host::LinuxX64,
                "nvidia_cusolver-12.2.6.9-py3-none-manylinux_2_27_x86_64.whl",
                "e32cb40c8a4d3df475b556caaf6af5808ef064df7e291a049528a0d28017b674",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cusolver-12.2.6.9-py3-none-manylinux_2_27_aarch64.whl",
                "658ce9e6ed8a321033419fe6e55faf0f245a15aa23669eda03d7f2dcce436d73",
            ),
        ],
    },
    Wheel {
        project: "nvidia-cudnn-cu13",
        version: "9.25.1.1",
        windows: &[
            "cudnn64_9.dll",
            "cudnn_adv64_9.dll",
            "cudnn_cnn64_9.dll",
            "cudnn_engines_precompiled64_9.dll",
            "cudnn_engines_runtime_compiled64_9.dll",
            "cudnn_engines_tensor_ir64_9.dll",
            "cudnn_ext64_9.dll",
            "cudnn_graph64_9.dll",
            "cudnn_heuristic64_9.dll",
            "cudnn_ops64_9.dll",
        ],
        linux: &[
            "libcudnn.so.9",
            "libcudnn_adv.so.9",
            "libcudnn_cnn.so.9",
            "libcudnn_engines_precompiled.so.9",
            "libcudnn_engines_runtime_compiled.so.9",
            "libcudnn_engines_tensor_ir.so.9",
            "libcudnn_ext.so.9",
            "libcudnn_graph.so.9",
            "libcudnn_heuristic.so.9",
            "libcudnn_ops.so.9",
        ],
        files: [
            (
                Host::WindowsX64,
                "nvidia_cudnn_cu13-9.25.1.1-py3-none-win_amd64.whl",
                "e1de75bf1ad9040414f9b13cc87135d660d13dd3c859180dd6b63353e571f860",
            ),
            (
                Host::LinuxX64,
                "nvidia_cudnn_cu13-9.25.1.1-py3-none-manylinux_2_27_x86_64.whl",
                "195199e8c7f8bca0c366ea675dd2441a406646cf95a98bcda66c999d57dd1f26",
            ),
            (
                Host::LinuxArm64,
                "nvidia_cudnn_cu13-9.25.1.1-py3-none-manylinux_2_27_aarch64.whl",
                "6ed974d46b1b5fd123c0d9ff031c857142aa94257c6df41c40f6b5fd11a01d50",
            ),
        ],
    },
];

impl Wheel {
    fn libraries(&self) -> &'static [&'static str] {
        if cfg!(windows) {
            self.windows
        } else {
            self.linux
        }
    }

    fn directory(&self) -> String {
        format!("{}--{}", self.project, self.version)
    }
}

/// Installs every wheel and returns their folders, in load order.
pub(super) async fn install(host: Host) -> Result<Vec<PathBuf>> {
    let mut folders = Vec::new();
    for wheel in WHEELS {
        let (_, filename, sha256) = wheel
            .files
            .iter()
            .find(|(platform, ..)| *platform == host)
            .with_context(|| format!("{} has no wheel for this platform", wheel.project))?;
        // PyPI's stable path for a file, which redirects to where it's stored.
        let first = &wheel.project[..1];
        let url = format!(
            "https://files.pythonhosted.org/packages/py3/{first}/{}/{filename}",
            wheel.project
        );
        let folder = Store::directory(
            Store::root().join("cuda").join(wheel.directory()),
            |folder| {
                wheel
                    .libraries()
                    .iter()
                    .all(|name| find(folder, name).is_ok())
            },
            move |stage| async move { install_archive(&url, sha256, &stage, LIBRARIES).await },
        )
        .await?;
        folders.push(folder);
    }
    Ok(folders)
}

pub(super) fn load(folders: &[PathBuf]) -> Result<()> {
    for (wheel, folder) in WHEELS.iter().zip(folders) {
        for name in wheel.libraries() {
            preload(&find(folder, name)?, false)?;
        }
    }
    Ok(())
}

pub(super) async fn remove_older() {
    for wheel in WHEELS {
        remove_others(
            Store::root().join("cuda"),
            format!("{}--", wheel.project),
            wheel.directory(),
        )
        .await;
    }
}
