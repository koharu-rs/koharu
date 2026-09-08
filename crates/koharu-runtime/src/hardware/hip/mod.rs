mod targets;

pub(crate) use targets::GfxTarget;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) fn probe() -> Vec<crate::Device> {
    linux::probe().unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to discover AMD GPUs");
        Vec::new()
    })
}

#[cfg(target_os = "windows")]
pub(super) fn probe() -> Vec<crate::Device> {
    windows::probe()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) fn probe() -> Vec<crate::Device> {
    Vec::new()
}
