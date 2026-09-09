//! AMD targets supported by the bundled ROCm packages.
//!
//! Architecture names and KFD versions follow `rocm-systems` commit
//! `a022846cf553c2b135410a5168f97705f1b9c6ac`. Device types follow TheRock's
//! supported iGPU families.

#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    PartialEq,
    strum::Display,
    strum::EnumString,
    strum::EnumIter,
    strum::EnumProperty,
)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum Target {
    #[cfg(target_os = "linux")]
    #[strum(props(version = 90_008, integrated = false))]
    Gfx908,
    #[cfg(target_os = "linux")]
    #[strum(props(version = 90_010, integrated = false))]
    Gfx90a,
    #[cfg(target_os = "linux")]
    #[strum(props(version = 90_402, integrated = false))]
    Gfx942,
    #[cfg(target_os = "linux")]
    #[strum(props(version = 90_500, integrated = false))]
    Gfx950,
    #[strum(props(version = 100_100, integrated = false))]
    Gfx1010,
    #[strum(props(version = 100_101, integrated = false))]
    Gfx1011,
    #[strum(props(version = 100_102, integrated = false))]
    Gfx1012,
    #[strum(props(version = 100_300, integrated = false))]
    Gfx1030,
    #[strum(props(version = 100_301, integrated = false))]
    Gfx1031,
    #[strum(props(version = 100_302, integrated = false))]
    Gfx1032,
    #[strum(props(version = 100_303, integrated = true))]
    Gfx1033,
    #[strum(props(version = 100_304, integrated = false))]
    Gfx1034,
    #[strum(props(version = 100_305, integrated = true))]
    Gfx1035,
    #[strum(props(version = 100_306, integrated = true))]
    Gfx1036,
    #[strum(props(version = 110_000, integrated = false))]
    Gfx1100,
    #[strum(props(version = 110_001, integrated = false))]
    Gfx1101,
    #[strum(props(version = 110_002, integrated = false))]
    Gfx1102,
    #[cfg(target_os = "windows")]
    #[strum(props(version = 110_003, integrated = true))]
    Gfx1103,
    #[strum(props(version = 110_500, integrated = true))]
    Gfx1150,
    #[strum(props(version = 110_501, integrated = true))]
    Gfx1151,
    #[strum(props(version = 110_502, integrated = true))]
    Gfx1152,
    #[cfg(target_os = "windows")]
    #[strum(props(version = 110_503, integrated = true))]
    Gfx1153,
    #[strum(props(version = 120_000, integrated = false))]
    Gfx1200,
    #[strum(props(version = 120_001, integrated = false))]
    Gfx1201,
}

impl Target {
    pub(super) fn device_type(self) -> crate::DeviceType {
        use strum::EnumProperty;

        if self
            .get_bool("integrated")
            .expect("ROCm target has a device type")
        {
            crate::DeviceType::IntegratedGpu
        } else {
            crate::DeviceType::Gpu
        }
    }
}
