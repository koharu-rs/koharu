//! Requests to the sites the built-in extensions read, which carry the
//! browser's cookies, so the user's sign-in to a site, in its sign-in
//! window, applies to them.

use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use cef::{
    CefString, CookieVisitor, ImplBrowser as _, ImplBrowserHost as _, ImplCookieManager as _,
    ImplCookieVisitor, ImplRequestContext as _, WrapCookieVisitor, rc::Rc as _,
};
use parking_lot::Mutex;
use reqwest::{cookie::CookieStore, header::HeaderValue};
use tauri::{AppHandle, Manager as _};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::oneshot;
use url::Url;

use crate::screen;

/// The client the extensions' requests go through.
pub fn client(handle: AppHandle<CefRuntime>) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .cookie_provider(Arc::new(Browser(handle)))
        .build()?)
}

/// Removes the site's session from the shared browser profile, including
/// HttpOnly and parent-domain cookies, before account queries run again.
pub async fn sign_out(handle: &AppHandle<CefRuntime>, url: Url) -> Result<()> {
    let webview = handle
        .get_webview(screen::MAIN)
        .context("no main webview")?;
    let (started, starting) = oneshot::channel();
    webview.with_webview(move |webview| {
        let result = (|| {
            let manager = webview
                .browser()
                .host()
                .and_then(|host| host.request_context())
                .and_then(|context| context.cookie_manager(None))
                .context("the browser has no cookie manager")?;
            let (finished, finishing) = oneshot::channel();
            let mut visitor = ForgetCookies::new(Arc::new(Mutex::new(Some(finished))));
            ensure!(
                manager.visit_url_cookies(
                    Some(&CefString::from(url.as_str())),
                    1,
                    Some(&mut visitor)
                ) != 0,
                "the browser couldn't clear the site's cookies"
            );
            Ok::<_, anyhow::Error>(finishing)
        })();
        let _ = started.send(result);
    })?;
    let finished = starting.await??;
    finished.await?;
    Ok(())
}

cef::wrap_cookie_visitor! {
    struct ForgetCookies {
        finished: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    }

    impl CookieVisitor {
        fn visit(
            &self,
            _cookie: Option<&cef::Cookie>,
            _count: i32,
            _total: i32,
            delete_cookie: Option<&mut i32>,
        ) -> i32 {
            if let Some(delete_cookie) = delete_cookie {
                *delete_cookie = 1;
            }
            1
        }
    }
}

// CEF releases the visitor after traversal, including when there are no cookies.
impl Drop for ForgetCookies {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.lock().take() {
            let _ = finished.send(());
        }
    }
}

/// The browser's cookies, which every window shares.
struct Browser(AppHandle<CefRuntime>);

impl CookieStore for Browser {
    /// A reply's cookies aren't kept: a site's session is the browser's.
    fn set_cookies(&self, _: &mut dyn Iterator<Item = &HeaderValue>, _: &Url) {}

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let cookies = self
            .0
            .get_webview(screen::MAIN)?
            .cookies_for_url(url.clone())
            .inspect_err(|error| tracing::warn!("failed to read the browser's cookies: {error}"))
            .ok()?;
        let header = cookies
            .iter()
            .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
            .collect::<Vec<_>>()
            .join("; ");
        HeaderValue::from_str(&header)
            .ok()
            .filter(|header| !header.is_empty())
    }
}
