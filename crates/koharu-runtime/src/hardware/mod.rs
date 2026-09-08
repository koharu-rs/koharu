mod cuda;
mod hip;
mod vulkan;

use std::{cmp::Reverse, sync::OnceLock};

use strum::EnumProperty;

use crate::{Backend, Device, DeviceType};

pub(crate) use hip::GfxTarget;

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
            if let Some((driver, cuda)) = cuda::probe() {
                if driver >= 13000 {
                    devices.extend(cuda);
                } else {
                    tracing::warn!(driver, required = 13000, "CUDA driver is too old");
                }
            }
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
                    (Backend::Cuda, integrated)
                        if cfg!(any(
                            all(target_os = "windows", target_arch = "x86_64"),
                            all(target_os = "linux", target_arch = "x86_64"),
                            all(target_os = "linux", target_arch = "aarch64")
                        )) && device.compute_capability >= 75 =>
                    {
                        if integrated {
                            2
                        } else {
                            0
                        }
                    }
                    (Backend::Rocm, integrated)
                        if cfg!(all(
                            target_arch = "x86_64",
                            any(target_os = "windows", target_os = "linux")
                        )) && device.target.is_some_and(|target| {
                            target.get_str(std::env::consts::OS).is_some()
                        }) =>
                    {
                        if integrated {
                            3
                        } else {
                            1
                        }
                    }
                    (Backend::Vulkan, _)
                        if cfg!(all(
                            target_arch = "x86_64",
                            any(target_os = "windows", target_os = "linux")
                        )) =>
                    {
                        4
                    }
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
    pub fn devices(&self) -> &[Device] {
        &self.devices
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
        cfg!(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64")
        )) && matches!(self.device(), Some(device) if device.backend == Backend::Cuda && device.compute_capability >= 75)
    }

    #[must_use]
    pub fn cuda_compute_capability(&self) -> u32 {
        self.device()
            .filter(|device| device.backend == Backend::Cuda)
            .map_or(0, Device::compute_capability)
    }

    #[must_use]
    pub fn supports_rocm(&self) -> bool {
        cfg!(all(
            target_arch = "x86_64",
            any(target_os = "windows", target_os = "linux")
        )) && self
            .rocm_target()
            .is_some_and(|target| target.get_str(std::env::consts::OS).is_some())
    }

    pub(crate) fn rocm_target(&self) -> Option<GfxTarget> {
        self.device()
            .filter(|device| device.backend == Backend::Rocm)
            .and_then(|device| device.target)
    }

    #[must_use]
    pub fn supports_vulkan(&self) -> bool {
        cfg!(all(
            target_arch = "x86_64",
            any(target_os = "windows", target_os = "linux")
        )) && matches!(self.device(), Some(device) if device.backend == Backend::Vulkan)
    }

    #[must_use]
    pub fn supports_metal(&self) -> bool {
        cfg!(all(target_os = "macos", target_arch = "aarch64"))
            && matches!(self.device(), Some(device) if device.backend == Backend::Metal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_targets() {
        let supported = |target: &str| {
            let Ok(target) = target.parse::<GfxTarget>() else {
                return false;
            };
            Hardware {
                devices: vec![Device {
                    target: Some(target),
                    ..Device::rocm(0)
                }],
                candidates: vec![0],
                selected: Some(0),
            }
            .supports_rocm()
        };
        let x64_rocm = cfg!(all(
            target_arch = "x86_64",
            any(target_os = "windows", target_os = "linux")
        ));
        assert_eq!(supported("gfx1010"), x64_rocm);
        assert_eq!(supported("gfx1036"), x64_rocm);
        assert_eq!(supported("gfx1201"), x64_rocm);
        assert_eq!(
            supported("gfx908"),
            cfg!(all(target_os = "linux", target_arch = "x86_64"))
        );
        assert_eq!(
            supported("gfx1103"),
            cfg!(all(target_os = "windows", target_arch = "x86_64"))
        );
        assert_eq!(
            supported("gfx1153"),
            cfg!(all(target_os = "windows", target_arch = "x86_64"))
        );
        assert!(!supported("gfx1251"));
    }
}
