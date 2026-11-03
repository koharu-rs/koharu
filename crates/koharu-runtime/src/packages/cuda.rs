//! NVIDIA's CUDA libraries, from their wheels on PyPI.

use std::path::PathBuf;

use anyhow::Result;

use super::{Host, LIBRARIES, install_archive, preload};
use crate::store;

struct Wheel {
    project: &'static str,
    version: &'static str,
    /// The libraries it holds, in load order, at their paths in the wheel.
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
        windows: &["nvidia/cu13/bin/x86_64/cudart64_13.dll"],
        linux: &["nvidia/cu13/lib/libcudart.so.13"],
        manylinux: [
            "manylinux2014_x86_64.manylinux_2_17_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-nvjitlink",
        version: "13.3.33",
        windows: &["nvidia/cu13/bin/x86_64/nvJitLink_130_0.dll"],
        linux: &["nvidia/cu13/lib/libnvJitLink.so.13"],
        manylinux: [
            "manylinux2010_x86_64.manylinux_2_12_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-cuda-nvrtc",
        version: "13.3.33",
        windows: &[
            "nvidia/cu13/bin/x86_64/nvrtc-builtins64_133.dll",
            "nvidia/cu13/bin/x86_64/nvrtc64_130_0.dll",
        ],
        linux: &[
            "nvidia/cu13/lib/libnvrtc-builtins.so.13.3",
            "nvidia/cu13/lib/libnvrtc.so.13",
        ],
        manylinux: [
            "manylinux2010_x86_64.manylinux_2_12_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-cublas",
        version: "13.6.0.2",
        windows: &[
            "nvidia/cu13/bin/x86_64/cublasLt64_13.dll",
            "nvidia/cu13/bin/x86_64/cublas64_13.dll",
        ],
        linux: &[
            "nvidia/cu13/lib/libcublasLt.so.13",
            "nvidia/cu13/lib/libcublas.so.13",
        ],
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
    },
    Wheel {
        project: "nvidia-cufft",
        version: "12.3.0.29",
        windows: &["nvidia/cu13/bin/x86_64/cufft64_12.dll"],
        linux: &["nvidia/cu13/lib/libcufft.so.12"],
        manylinux: [
            "manylinux2014_x86_64.manylinux_2_17_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-curand",
        version: "10.4.3.29",
        windows: &["nvidia/cu13/bin/x86_64/curand64_10.dll"],
        linux: &["nvidia/cu13/lib/libcurand.so.10"],
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
    },
    Wheel {
        project: "nvidia-cusparse",
        version: "12.8.2.51",
        windows: &["nvidia/cu13/bin/x86_64/cusparse64_12.dll"],
        linux: &["nvidia/cu13/lib/libcusparse.so.12"],
        manylinux: [
            "manylinux2014_x86_64.manylinux_2_17_x86_64",
            "manylinux2014_aarch64.manylinux_2_17_aarch64",
        ],
    },
    Wheel {
        project: "nvidia-cusolver",
        version: "12.2.6.9",
        windows: &[
            "nvidia/cu13/bin/x86_64/cusolver64_12.dll",
            "nvidia/cu13/bin/x86_64/cusolverMg64_12.dll",
        ],
        linux: &[
            "nvidia/cu13/lib/libcusolver.so.12",
            "nvidia/cu13/lib/libcusolverMg.so.12",
        ],
        manylinux: ["manylinux_2_27_x86_64", "manylinux_2_27_aarch64"],
    },
    Wheel {
        project: "nvidia-cudnn-cu13",
        version: "9.25.1.1",
        windows: &[
            "nvidia/cudnn/bin/cudnn64_9.dll",
            "nvidia/cudnn/bin/cudnn_adv64_9.dll",
            "nvidia/cudnn/bin/cudnn_cnn64_9.dll",
            "nvidia/cudnn/bin/cudnn_engines_precompiled64_9.dll",
            "nvidia/cudnn/bin/cudnn_engines_runtime_compiled64_9.dll",
            "nvidia/cudnn/bin/cudnn_engines_tensor_ir64_9.dll",
            "nvidia/cudnn/bin/cudnn_ext64_9.dll",
            "nvidia/cudnn/bin/cudnn_graph64_9.dll",
            "nvidia/cudnn/bin/cudnn_heuristic64_9.dll",
            "nvidia/cudnn/bin/cudnn_ops64_9.dll",
        ],
        linux: &[
            "nvidia/cudnn/lib/libcudnn.so.9",
            "nvidia/cudnn/lib/libcudnn_adv.so.9",
            "nvidia/cudnn/lib/libcudnn_cnn.so.9",
            "nvidia/cudnn/lib/libcudnn_engines_precompiled.so.9",
            "nvidia/cudnn/lib/libcudnn_engines_runtime_compiled.so.9",
            "nvidia/cudnn/lib/libcudnn_engines_tensor_ir.so.9",
            "nvidia/cudnn/lib/libcudnn_ext.so.9",
            "nvidia/cudnn/lib/libcudnn_graph.so.9",
            "nvidia/cudnn/lib/libcudnn_heuristic.so.9",
            "nvidia/cudnn/lib/libcudnn_ops.so.9",
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

    /// PyPI's stable path for the wheel for `host`, which redirects to where
    /// it's stored.
    fn url(&self, host: Host) -> Result<String> {
        let platform = match host {
            Host::WindowsX64 => "win_amd64",
            Host::LinuxX64 => self.manylinux[0],
            Host::LinuxArm64 => self.manylinux[1],
            Host::MacosArm64 => anyhow::bail!("CUDA's wheels don't exist for macOS"),
        };
        let filename = format!(
            "{}-{}-py3-none-{platform}.whl",
            self.project.replace('-', "_"),
            self.version
        );
        let first = &self.project[..1];
        Ok(format!(
            "https://files.pythonhosted.org/packages/py3/{first}/{}/{filename}",
            self.project
        ))
    }
}

/// Installs every wheel and returns their folders, in load order.
pub(super) async fn install(host: Host) -> Result<Vec<PathBuf>> {
    let mut folders = Vec::new();
    for wheel in WHEELS {
        let url = wheel.url(host)?;
        let folder = store::install(
            store::root()
                .join("cuda")
                .join(wheel.project)
                .join(wheel.version),
            |folder| {
                wheel
                    .libraries()
                    .iter()
                    .all(|library| folder.join(library).is_file())
            },
            move |stage| async move {
                install_archive(&url, &stage, LIBRARIES).await?;
                Ok(stage)
            },
        )
        .await?;
        folders.push(folder);
    }
    Ok(folders)
}

pub(super) fn load(folders: &[PathBuf]) -> Result<()> {
    for (wheel, folder) in WHEELS.iter().zip(folders) {
        for library in wheel.libraries() {
            preload(&folder.join(library), false)?;
        }
    }
    Ok(())
}
