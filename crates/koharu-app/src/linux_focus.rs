//! Temporary native-focus workaround for https://github.com/tauri-apps/tauri/issues/16251.
//! See the crate README for verification and removal.

use cef::{ImplBrowser, ImplBrowserHost};
use tauri::Window;
use tauri_runtime_cef::CefRuntime;

mod x11;

pub(crate) fn synchronize(window: &Window<CefRuntime>, focused: bool) {
    #[cfg(debug_assertions)]
    if std::env::var("KOHARU_DISABLE_CEF_FOCUS_WORKAROUND").is_ok_and(|value| value == "1") {
        return;
    }

    let Some(webview) = window
        .webviews()
        .into_iter()
        .find(|webview| webview.label() == window.label())
    else {
        return;
    };

    if let Err(error) = webview.with_webview(move |platform| {
        let Some(host) = platform.browser().host() else {
            return;
        };
        host.set_focus(i32::from(focused));
        // GTK can focus its proxy while CEF still reports DOM focus. CEF's
        // focus API alone does not transfer native keyboard focus on X11.
        if focused && let Err(error) = x11::focus_browser(&host) {
            tracing::warn!(%error, "failed to focus the CEF browser window");
        }
    }) {
        tracing::warn!(%error, "failed to synchronize CEF window focus");
    }
}
