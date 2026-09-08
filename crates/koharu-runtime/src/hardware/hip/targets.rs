//! Canonical AMD targets recognized by `rocm-bootstrap`.
//!
//! Architecture names and KFD versions follow `rocm-systems` commit
//! `a022846cf553c2b135410a5168f97705f1b9c6ac`. Device types follow TheRock's
//! iGPU families, including legacy APUs retained by `rocm-bootstrap`.
//! Platform properties identify targets packaged by Koharu's ROCm runtime.

use strum::{AsRefStr, EnumIter, EnumProperty, EnumString};

#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    PartialEq,
    AsRefStr,
    EnumIter,
    EnumProperty,
    EnumString,
    strum::Display,
)]
#[repr(i64)]
pub(crate) enum GfxTarget {
    #[strum(serialize = "gfx900")]
    Gfx900 = 90_000,
    #[strum(serialize = "gfx902", props(integrated = "true"))]
    Gfx902 = 90_002,
    #[strum(serialize = "gfx904")]
    Gfx904 = 90_004,
    #[strum(serialize = "gfx906")]
    Gfx906 = 90_006,
    #[strum(serialize = "gfx908", props(linux = "true"))]
    Gfx908 = 90_008,
    #[strum(serialize = "gfx909", props(integrated = "true"))]
    Gfx909 = 90_009,
    #[strum(serialize = "gfx90a", props(linux = "true"))]
    Gfx90a = 90_010,
    #[strum(serialize = "gfx90c", props(integrated = "true"))]
    Gfx90c = 90_012,
    #[strum(serialize = "gfx942", props(linux = "true"))]
    Gfx942 = 90_402,
    #[strum(serialize = "gfx950", props(linux = "true"))]
    Gfx950 = 90_500,
    #[strum(serialize = "gfx1010", props(windows = "true", linux = "true"))]
    Gfx1010 = 100_100,
    #[strum(serialize = "gfx1011", props(windows = "true", linux = "true"))]
    Gfx1011 = 100_101,
    #[strum(serialize = "gfx1012", props(windows = "true", linux = "true"))]
    Gfx1012 = 100_102,
    #[strum(serialize = "gfx1013", props(integrated = "true"))]
    Gfx1013 = 100_103,
    #[strum(serialize = "gfx1030", props(windows = "true", linux = "true"))]
    Gfx1030 = 100_300,
    #[strum(serialize = "gfx1031", props(windows = "true", linux = "true"))]
    Gfx1031 = 100_301,
    #[strum(serialize = "gfx1032", props(windows = "true", linux = "true"))]
    Gfx1032 = 100_302,
    #[strum(
        serialize = "gfx1033",
        props(integrated = "true", windows = "true", linux = "true")
    )]
    Gfx1033 = 100_303,
    #[strum(serialize = "gfx1034", props(windows = "true", linux = "true"))]
    Gfx1034 = 100_304,
    #[strum(
        serialize = "gfx1035",
        props(integrated = "true", windows = "true", linux = "true")
    )]
    Gfx1035 = 100_305,
    #[strum(
        serialize = "gfx1036",
        props(integrated = "true", windows = "true", linux = "true")
    )]
    Gfx1036 = 100_306,
    #[strum(serialize = "gfx1100", props(windows = "true", linux = "true"))]
    Gfx1100 = 110_000,
    #[strum(serialize = "gfx1101", props(windows = "true", linux = "true"))]
    Gfx1101 = 110_001,
    #[strum(serialize = "gfx1102", props(windows = "true", linux = "true"))]
    Gfx1102 = 110_002,
    #[strum(serialize = "gfx1103", props(integrated = "true", windows = "true"))]
    Gfx1103 = 110_003,
    #[strum(
        serialize = "gfx1150",
        props(integrated = "true", windows = "true", linux = "true")
    )]
    Gfx1150 = 110_500,
    #[strum(
        serialize = "gfx1151",
        props(integrated = "true", windows = "true", linux = "true")
    )]
    Gfx1151 = 110_501,
    #[strum(
        serialize = "gfx1152",
        props(integrated = "true", windows = "true", linux = "true")
    )]
    Gfx1152 = 110_502,
    #[strum(serialize = "gfx1153", props(integrated = "true", windows = "true"))]
    Gfx1153 = 110_503,
    #[strum(serialize = "gfx1200", props(windows = "true", linux = "true"))]
    Gfx1200 = 120_000,
    #[strum(serialize = "gfx1201", props(windows = "true", linux = "true"))]
    Gfx1201 = 120_001,
    #[strum(serialize = "gfx1250")]
    Gfx1250 = 120_500,
    #[strum(serialize = "gfx1251")]
    Gfx1251 = 120_501,
}
