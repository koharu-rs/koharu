use std::mem::MaybeUninit;

use anyhow::{Context as _, Result, ensure};
use cef::{BrowserHost, ImplBrowserHost};
use x11_dl::xlib;

pub(super) fn focus_browser(host: &BrowserHost) -> Result<()> {
    ensure!(
        cef::currently_on(cef::ThreadId::UI) != 0,
        "native browser focus must run on CEF's UI thread"
    );
    let window = host.window_handle();
    let display = cef::get_xdisplay().cast::<xlib::Display>();
    if window == 0 || display.is_null() {
        return Ok(());
    }
    let x = xlib::Xlib::open().context("failed to load Xlib")?;
    let mut attributes = MaybeUninit::<xlib::XWindowAttributes>::uninit();

    // SAFETY: with_webview holds the browser alive on CEF's UI thread. CEF owns
    // the display, which is borrowed here and must not be closed. Xlib initializes
    // attributes only on success; unmapped windows cannot receive input focus.
    unsafe {
        if (x.XGetWindowAttributes)(display, window, attributes.as_mut_ptr()) != 0
            && attributes.assume_init().map_state == xlib::IsViewable
        {
            (x.XSetInputFocus)(display, window, xlib::RevertToParent, xlib::CurrentTime);
            (x.XFlush)(display);
        }
    }
    Ok(())
}
