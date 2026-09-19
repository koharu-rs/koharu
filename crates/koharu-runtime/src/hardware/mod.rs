mod cuda;
mod hip;
mod vulkan;

pub(crate) use hip::Target;

use std::{cmp::Reverse, sync::OnceLock};

use crate::{Backend, Device, DeviceType};

/// Accelerator capabilities and their process-wide selection priority.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Hardware {
    pub(crate) devices: Vec<Device>,
    pub(crate) candidates: Vec<usize>,
    pub(crate) selected: Option<usize>,
}

impl Hardware {
    #[must_use]
    pub fn discover() -> Self {
        static HARDWARE: OnceLock<Hardware> = OnceLock::new();
        HARDWARE.get_or_init(Self::probe).clone()
    }

    fn probe() -> Self {
        let mut devices = Vec::new();
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            devices.push(Device {
                description: "Metal".to_owned(),
                device_type: DeviceType::IntegratedGpu,
                ..Device::metal(0)
            });
        } else if cfg!(any(target_os = "linux", target_os = "windows")) {
            devices.extend(cuda::probe());
            devices.extend(hip::probe());
            if vulkan::probe() {
                devices.push(Device {
                    description: "Vulkan".to_owned(),
                    ..Device::vulkan(0)
                });
            }
        }

        let mut candidates = devices
            .iter()
            .enumerate()
            .filter_map(|(position, device)| {
                // Lower categories are preferred: Metal or discrete CUDA, discrete
                // ROCm, integrated CUDA, integrated ROCm, then Vulkan.
                let category = match (&device.backend, device.is_integrated()) {
                    (Backend::Metal, _) => 0,
                    (Backend::Cuda, false) => 0,
                    (Backend::Rocm, false) => 1,
                    (Backend::Cuda, true) => 2,
                    (Backend::Rocm, true) => 3,
                    (Backend::Vulkan, _) => 4,
                    _ => return None,
                };
                Some((position, category))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(position, category)| {
            let device = &devices[*position];
            (
                *category,
                Reverse(device.memory_total),
                Reverse(device.compute_capability),
                device.index,
                *position,
            )
        });
        let candidates = candidates
            .into_iter()
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        let selected = candidates.first().copied();
        Self {
            devices,
            candidates,
            selected,
        }
    }

    #[must_use]
    pub fn device(&self) -> Option<&Device> {
        self.selected.and_then(|index| self.devices.get(index))
    }

    pub(crate) fn candidates(&self) -> impl Iterator<Item = Self> + '_ {
        self.candidates
            .iter()
            .copied()
            .map(Some)
            .chain(std::iter::once(None))
            .map(|selected| Self {
                selected,
                ..self.clone()
            })
    }

    #[must_use]
    pub fn supports_cuda(&self) -> bool {
        matches!(self.device(), Some(device) if device.backend == Backend::Cuda)
    }

    #[must_use]
    pub fn supports_rocm(&self) -> bool {
        matches!(self.device(), Some(device) if device.backend == Backend::Rocm)
    }

    pub(crate) fn rocm_target(&self) -> anyhow::Result<Target> {
        let target = self
            .device()
            .filter(|device| device.backend == Backend::Rocm)
            .and_then(Device::target)
            .ok_or_else(|| anyhow::anyhow!("no ROCm target selected"))?;
        Ok(target.parse()?)
    }

    #[must_use]
    pub fn supports_vulkan(&self) -> bool {
        matches!(self.device(), Some(device) if device.backend == Backend::Vulkan)
    }

    #[must_use]
    pub fn supports_metal(&self) -> bool {
        matches!(self.device(), Some(device) if device.backend == Backend::Metal)
    }
}
