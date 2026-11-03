mod targets;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::probe;
use targets::Target;
#[cfg(target_os = "windows")]
pub(super) use windows::probe;
