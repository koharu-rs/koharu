//! Linux CEF WebGPU switches and GDK backend default.
//!
//! `tauri-runtime-cef` 3.0.0-alpha.1 treats valued tuples as switches, but a
//! flag-only name without a leading `-` becomes `append_argument` rather than
//! `append_switch`. Pass `"--enable-unsafe-webgpu"` with `None`.

use std::ffi::OsStr;

pub(crate) fn command_line_args() -> [(&'static str, Option<&'static str>); 3] {
    [
        ("--enable-unsafe-webgpu", None),
        ("use-angle", Some("vulkan")),
        ("--ozone-platform", Some("x11")),
    ]
}

pub(crate) fn gdk_backend_default(existing: Option<&OsStr>) -> Option<&'static str> {
    match existing {
        None => Some("x11"),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn linux_cef_command_line_args_enable_webgpu_angle_vulkan_and_ozone_x11() {
        let args = command_line_args();
        assert!(
            args.iter().any(|(name, value)| {
                name.trim_start_matches('-') == "enable-unsafe-webgpu" && value.is_none()
            }),
            "missing enable-unsafe-webgpu"
        );
        assert!(
            args.iter().any(|(name, value)| {
                name.trim_start_matches('-') == "use-angle" && *value == Some("vulkan")
            }),
            "missing use-angle=vulkan"
        );
        assert!(
            args.iter().any(|(name, value)| {
                name.trim_start_matches('-') == "ozone-platform" && *value == Some("x11")
            }),
            "missing ozone-platform=x11"
        );
    }

    #[test]
    fn linux_cef_enable_unsafe_webgpu_is_not_a_flag_only_name_without_dash() {
        let args = command_line_args();
        let webgpu = args
            .iter()
            .find(|(name, _)| name.trim_start_matches('-') == "enable-unsafe-webgpu")
            .expect("enable-unsafe-webgpu must be present");
        assert_ne!(
            webgpu.0, "enable-unsafe-webgpu",
            "flag-only names without a leading '-' become append_argument, not append_switch"
        );
        assert!(
            webgpu.0.starts_with('-'),
            "enable-unsafe-webgpu must be passed as a switch name with a leading dash"
        );
        assert!(webgpu.1.is_none());
    }

    #[test]
    fn linux_cef_gdk_helper_does_not_overwrite_existing_gdk_backend() {
        assert_eq!(
            gdk_backend_default(Some(OsStr::new("wayland"))),
            None,
            "must not override an explicit GDK_BACKEND"
        );
        assert_eq!(
            gdk_backend_default(Some(OsStr::new("x11"))),
            None,
            "must not override an explicit GDK_BACKEND"
        );
        assert_eq!(gdk_backend_default(None), Some("x11"));
    }
}
