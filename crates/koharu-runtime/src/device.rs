use std::ffi::CString;

use serde::Serialize;

/// The API a device computes through, with what a model needs to know of it.
/// As a `&'static str` it's the name Koharu's native builds give its variant:
/// `cpu`, `cuda`, `hip`, `metal`, or `vulkan`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum Backend {
    Cpu,
    /// The compute capability, such as 120 for 12.0.
    Cuda {
        compute_capability: u32,
    },
    /// The AMD GPU's LLVM target, such as `gfx1100`.
    #[strum(serialize = "hip")]
    Rocm {
        target: &'static str,
    },
    Metal,
    Vulkan,
}

/// The device models run on, as llama.cpp lists it. libtorch, which has no
/// Vulkan build, runs on the CPU in a Vulkan device's place.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Device {
    /// llama.cpp's name for it, such as `CUDA0` or `CPU`.
    pub name: String,
    /// Its index as CUDA and HIP count devices, which is also a GPU's place
    /// in llama.cpp's device list; 0 for the CPU.
    pub index: usize,
    /// llama.cpp's description of it, such as `NVIDIA GeForce RTX 5090`.
    pub description: String,
    pub backend: Backend,
    /// Whether it shares the system's memory.
    pub integrated: bool,
}

/// A device's memory, in bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Memory {
    pub free: u64,
    pub total: u64,
}

impl Device {
    /// The device's free and total memory now, as llama.cpp's backend reads
    /// them from the driver, or the system's for the CPU.
    #[must_use]
    pub fn memory(&self) -> Memory {
        let (mut free, mut total) = (0, 0);
        let name = CString::new(self.name.as_str()).expect("llama.cpp's names have no NUL");
        // The runtime hands out a device only after llama.cpp loaded and
        // listed it under its name.
        unsafe {
            let device = koharu_llama_sys::ggml_backend_dev_by_name(name.as_ptr());
            koharu_llama_sys::ggml_backend_dev_memory(device, &mut free, &mut total);
        }
        Memory {
            free: free as u64,
            total: total as u64,
        }
    }

    /// Whether this device reports reliable device-wide VRAM free and total
    /// accounting across processes (discrete CUDA and ROCm/HIP GPUs only).
    #[must_use]
    pub fn has_vram_accounting(&self) -> bool {
        self.has_vram_accounting_with_total(
            self.supports_vram_accounting()
                .then(|| self.memory().total)
                .unwrap_or(0),
        )
    }

    fn supports_vram_accounting(&self) -> bool {
        matches!(self.backend, Backend::Cuda { .. } | Backend::Rocm { .. }) && !self.integrated
    }

    fn has_vram_accounting_with_total(&self, total: u64) -> bool {
        self.supports_vram_accounting() && total > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(backend: Backend, integrated: bool) -> Device {
        Device {
            name: "TEST0".to_string(),
            index: 0,
            description: "Test Device".to_string(),
            backend,
            integrated,
        }
    }

    #[test]
    fn non_discrete_and_non_cuda_rocm_skip_vram_accounting() {
        assert!(!device(Backend::Cpu, true).has_vram_accounting());
        assert!(!device(Backend::Cpu, false).has_vram_accounting());
        assert!(!device(Backend::Metal, true).has_vram_accounting());
        assert!(!device(Backend::Metal, false).has_vram_accounting());
        assert!(!device(Backend::Vulkan, false).has_vram_accounting());
        assert!(!device(Backend::Vulkan, true).has_vram_accounting());
        assert!(
            !device(
                Backend::Cuda {
                    compute_capability: 89
                },
                true
            )
            .has_vram_accounting()
        );
        assert!(!device(Backend::Rocm { target: "gfx1100" }, true).has_vram_accounting());
    }

    #[test]
    fn discrete_cuda_and_rocm_require_nonzero_total_memory() {
        let cuda = device(
            Backend::Cuda {
                compute_capability: 89,
            },
            false,
        );
        assert!(!cuda.has_vram_accounting_with_total(0));
        assert!(cuda.has_vram_accounting_with_total(8 * 1024 * 1024 * 1024));

        let rocm = device(Backend::Rocm { target: "gfx1100" }, false);
        assert!(!rocm.has_vram_accounting_with_total(0));
        assert!(rocm.has_vram_accounting_with_total(16 * 1024 * 1024 * 1024));
    }
}
