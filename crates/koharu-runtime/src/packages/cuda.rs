//! NVIDIA's CUDA libraries, from their wheels on PyPI.

use std::path::PathBuf;

use anyhow::Result;

use super::{Host, LIBRARIES, find, install_archive, preload};
use crate::store;

struct Wheel {
    project: &'static str,
    version: &'static str,
    /// The libraries it holds, in load order.
    windows: &'static [&'static str],
    linux: &'static [&'static str],
    /// The platform tags of its x86-64 and ARM64 Linux wheels; every
    /// Windows wheel is `win_amd64`.
    manylinux: [&'static str; 2],
}

/// In load order: each wheel comes after the ones its libraries need.
const WHEELS: &[Wheel] = &[
    Wheel {
        project: "nvidia-cuda-runtime",
        version: "13.3.29",
        windows: &["cudart64_13.dll"],
        linux: &["libcudart.so.13"],
        manylinux: [
            "manylinux2014_x86_64.manylinux_2_17_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-nvjitlink",
        version: "13.3.33",
        windows: &["nvJitLink_130_0.dll"],
        linux: &["libnvJitLink.so.13"],
        manylinux: [
            "manylinux2010_x86_64.manylinux_2_12_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-cuda-nvrtc",
        version: "13.3.33",
        windows: &["nvrtc-builtins64_133.dll", "nvrtc64_130_0.dll"],
        linux: &["libnvrtc-builtins.so.13.3", "libnvrtc.so.13"],
        manylinux: [
            "manylinux2010_x86_64.manylinux_2_12_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-cublas",
        version: "13.6.0.2",
        windows: &["cublasLt64_13.dll", "cublas64_13.dll"],
        linux: &["libcublasLt.so.13", "libcublas.so.13"],
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
    },
    Wheel {
        project: "nvidia-cufft",
        version: "12.3.0.29",
        windows: &["cufft64_12.dll"],
        linux: &["libcufft.so.12"],
        manylinux: [
            "manylinux2014_x86_64.manylinux_2_17_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-curand",
        version: "10.4.3.29",
        windows: &["curand64_10.dll"],
        linux: &["libcurand.so.10"],
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
    },
    Wheel {
        project: "nvidia-cusparse",
        version: "12.8.2.51",
        windows: &["cusparse64_12.dll"],
        linux: &["libcusparse.so.12"],
        manylinux: [
            "manylinux2014_x86_64.manylinux_2_17_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-cusolver",
        version: "12.2.6.9",
        windows: &["cusolver64_12.dll", "cusolverMg64_12.dll"],
        linux: &["libcusolver.so.12", "libcusolverMg.so.12"],
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
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
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
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
}

/// Installs every wheel and returns their folders, in load order.
pub(super) async fn install(host: Host) -> Result<Vec<PathBuf>> {
    let mut folders = Vec::new();
    for wheel in WHEELS {
        let platform = match host {
            Host::WindowsX64 => "win_amd64",
            Host::LinuxX64 => wheel.manylinux[0],
            Host::LinuxArm64 => wheel.manylinux[1],
            Host::MacosArm64 => anyhow::bail!("CUDA's wheels don't exist for macOS"),
        };
        let filename = format!(
            "{}-{}-py3-none-{platform}.whl",
            wheel.project.replace('-', "_"),
            wheel.version
        );
        // PyPI's stable path for a file, which redirects to where it's stored.
        let first = &wheel.project[..1];
        let url = format!(
            "https://files.pythonhosted.org/packages/py3/{first}/{}/{filename}",
            wheel.project
        );
        let folder = store::directory(
            store::root()
                .join("cuda")
                .join(wheel.project)
                .join(wheel.version),
            |folder| {
                wheel
                    .libraries()
                    .iter()
                    .all(|name| find(folder, name).is_ok())
            },
            move |stage| async move { install_archive(&url, &stage, LIBRARIES).await },
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
