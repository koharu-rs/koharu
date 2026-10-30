use serde::Serialize;

/// The GPU API a device runs on, with what a model needs to know of it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub enum Backend {
    /// The compute capability, such as 120 for 12.0.
    Cuda {
        compute_capability: u32,
    },
    /// The AMD GPU's LLVM target, such as `gfx1100`.
    Rocm {
        target: &'static str,
    },
    Vulkan,
    Metal,
}

/// The GPU the runtime started on.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Device {
    /// llama.cpp's name for it, such as `CUDA0`.
    pub name: String,
    /// Its index as CUDA and HIP count devices, which is also its place in
    /// llama.cpp's device list.
    pub index: usize,
    /// The name its driver gives, such as `NVIDIA GeForce RTX 5090`.
    pub description: String,
    pub backend: Backend,
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
    /// them from the driver.
    #[must_use]
    pub fn memory(&self) -> Memory {
        let (mut free, mut total) = (0, 0);
        // The runtime hands out a device only after llama.cpp loaded and
        // listed it at its index.
        unsafe {
            let device = koharu_llama_sys::ggml_backend_dev_get(self.index);
            koharu_llama_sys::ggml_backend_dev_memory(device, &mut free, &mut total);
        }
        Memory {
            free: free as u64,
            total: total as u64,
        }
    }
}
