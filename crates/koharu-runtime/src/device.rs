use std::ffi::CString;

use anyhow::{Context, Result};
use serde::Serialize;

/// The GPU API a device runs on.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, strum::Display)]
pub enum Backend {
    #[strum(serialize = "CUDA")]
    Cuda,
    #[strum(serialize = "ROCm")]
    Rocm,
    #[strum(serialize = "Vulkan")]
    Vulkan,
    #[strum(serialize = "Metal")]
    Metal,
}

/// The GPU the runtime started on.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Device {
    /// llama.cpp's name for it, such as `CUDA0`.
    pub name: String,
    /// The name its driver gives, such as `NVIDIA GeForce RTX 5090`.
    pub description: String,
    pub backend: Backend,
    /// Its index among its backend's devices, as CUDA, HIP, and libtorch
    /// count them.
    pub index: usize,
    pub integrated: bool,
    #[serde(skip)]
    pub(crate) compute_capability: u32,
    #[serde(skip)]
    pub(crate) target: Option<String>,
}

/// A device's memory, in bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Memory {
    pub free: u64,
    pub total: u64,
}

impl Device {
    /// The device's free and total memory now, as llama.cpp's backend reads
    /// them from the driver.
    pub fn memory(&self) -> Result<Memory> {
        let name = CString::new(self.name.as_str())?;
        // The runtime hands out a device only after llama.cpp loaded and
        // listed it.
        let device = unsafe { llama_cpp_sys_2::ggml_backend_dev_by_name(name.as_ptr()) };
        anyhow::ensure!(!device.is_null(), "llama.cpp doesn't list {}", self.name);
        let (mut free, mut total) = (0, 0);
        unsafe { llama_cpp_sys_2::ggml_backend_dev_memory(device, &mut free, &mut total) };
        Ok(Memory {
            free: u64::try_from(free).context("free memory doesn't fit in 64 bits")?,
            total: u64::try_from(total).context("total memory doesn't fit in 64 bits")?,
        })
    }

    /// The CUDA compute capability, such as 120 for 12.0; 0 off CUDA.
    #[must_use]
    pub fn compute_capability(&self) -> u32 {
        self.compute_capability
    }

    /// The AMD GPU's LLVM target, such as `gfx1100`; nothing off ROCm.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
}
