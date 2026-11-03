use anyhow::{Context, Result};
use strum::{EnumProperty, IntoEnumIterator};

/// The platforms Koharu ships native packages for, each named by the target
/// triple Koharu's own releases name their archives with.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumIter, strum::EnumProperty, strum::IntoStaticStr,
)]
pub(super) enum Host {
    #[strum(
        serialize = "x86_64-pc-windows-msvc",
        props(os = "windows", arch = "x86_64")
    )]
    WindowsX64,
    #[strum(
        serialize = "x86_64-unknown-linux-gnu",
        props(os = "linux", arch = "x86_64")
    )]
    LinuxX64,
    #[strum(
        serialize = "aarch64-unknown-linux-gnu",
        props(os = "linux", arch = "aarch64")
    )]
    LinuxArm64,
    #[strum(
        serialize = "aarch64-apple-darwin",
        props(os = "macos", arch = "aarch64")
    )]
    MacosArm64,
}

impl Host {
    pub(super) fn current() -> Result<Self> {
        Self::iter()
            .find(|host| {
                host.get_str("os") == Some(std::env::consts::OS)
                    && host.get_str("arch") == Some(std::env::consts::ARCH)
            })
            .context("Koharu has no native packages for this platform")
    }
}
