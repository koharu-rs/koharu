mod targets;

pub(crate) use targets::Target;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn probe() -> Vec<crate::Device> {
    linux::probe().unwrap_or_else(|error| {
        tracing::warn!(?error, "failed to discover AMD GPUs");
        Vec::new()
    })
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub(super) fn probe() -> Vec<crate::Device> {
    windows::probe()
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "windows")
)))]
pub(super) fn probe() -> Vec<crate::Device> {
    Vec::new()
}
