/// A device libtorch runs on. A HIP build of libtorch calls its GPUs CUDA
/// devices too.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Device {
    Cpu,
    Cuda(usize),
    Mps,
}

impl Device {
    // The shim's encoding: -1 is the CPU, -2 Metal, and an index a GPU.
    pub(crate) fn c_int(self) -> libc::c_int {
        match self {
            Device::Cpu => -1,
            Device::Cuda(index) => index as libc::c_int,
            Device::Mps => -2,
        }
    }

    pub(crate) fn from_c_int(v: libc::c_int) -> Self {
        match v {
            -2 => Device::Mps,
            index if index >= 0 => Device::Cuda(index as usize),
            _ => Device::Cpu,
        }
    }

    pub fn is_cuda(self) -> bool {
        matches!(self, Device::Cuda(_))
    }
}
