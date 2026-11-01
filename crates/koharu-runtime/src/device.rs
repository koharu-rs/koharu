use std::ffi::CString;

use serde::Serialize;

/// The API a device computes through, with what a model needs to know of it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub enum Backend {
    Cpu,
    /// The compute capability, such as 120 for 12.0.
    Cuda {
        compute_capability: u32,
    },
    /// The AMD GPU's LLVM target, such as `gfx1100`.
    Rocm {
        target: &'static str,
    },
    Metal,
    Vulkan,
}

/// A device a library runs models on.
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

/// Where each library runs models.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Devices {
    /// libtorch's: the GPU, or the CPU where libtorch has no build for the
    /// GPU's backend.
    pub torch: Device,
    /// llama.cpp's and stable-diffusion.cpp's, which share ggml: the GPU.
    pub ggml: Device,
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
}
