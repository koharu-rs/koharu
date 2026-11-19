//! The frames a second CEF renders the page at: the refresh rate of the
//! window's monitor, so a 120 or 144 Hz display shows a new frame each
//! refresh, as Chromium's own windows do.

use winit::monitor::MonitorHandle;

/// The frames a second where the monitor names no refresh rate.
pub const FALLBACK: i32 = 60;

/// `monitor`'s refresh rate in the whole frames a second CEF counts in: the
/// nearest, which shows a frame twice or drops one least often.
pub fn frame_rate(monitor: Option<&MonitorHandle>) -> i32 {
    monitor
        .and_then(refresh_rate)
        .map_or(FALLBACK, |hertz| (hertz.round() as i32).max(1))
}

/// `monitor`'s refresh rate, in hertz.
fn refresh_rate(monitor: &MonitorHandle) -> Option<f64> {
    #[cfg(windows)]
    if let Some(hertz) = display_config(monitor.native_id()) {
        return Some(hertz);
    }
    let millihertz = monitor.current_video_mode()?.refresh_rate_millihertz()?;
    Some(f64::from(millihertz.get()) / 1000.0)
}

/// The refresh rate of the monitor whose `HMONITOR` is `monitor`, in hertz,
/// from Display Config as Chromium reads it: the active path whose source is
/// the monitor's GDI device. winit's comes from the display mode, which
/// Windows rounds down to whole hertz, so a 59.94 Hz display would get 59
/// frames a second and show one twice each second.
#[cfg(windows)]
fn display_config(monitor: u64) -> Option<f64> {
    use std::mem::size_of;

    use windows::Win32::{
        Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
            DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS,
            QueryDisplayConfig,
        },
        Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFOEXW},
    };

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let monitor = HMONITOR(monitor as usize as _);
    // SAFETY: winit's native ID is the HMONITOR, and `info` is the
    // MONITORINFOEXW its size says.
    unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }
        .ok()
        .ok()?;

    let (mut path_count, mut mode_count) = (0, 0);
    // SAFETY: fills the two counts.
    unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) }
        .ok()
        .ok()?;
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: each array is as long as its count says.
    unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    }
    .ok()
    .ok()?;
    paths.truncate(path_count as usize);

    paths.iter().find_map(|path| {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        // SAFETY: the header heads the DISPLAYCONFIG_SOURCE_DEVICE_NAME its
        // type and size say.
        let named = unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } == 0;
        let rate = path.targetInfo.refreshRate;
        let known = rate.Numerator != 0 && rate.Denominator != 0;
        (named && source.viewGdiDeviceName == info.szDevice && known)
            .then(|| f64::from(rate.Numerator) / f64::from(rate.Denominator))
    })
}
