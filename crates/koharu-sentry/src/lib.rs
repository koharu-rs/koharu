//! Sentry's Tauri plugin for a Tauri 3 app: the web interface's events,
//! breadcrumbs, and scope go through the Rust SDK, so both sides report as
//! one app.
//!
//! Vendored from getsentry/sentry-tauri at f83b7225301e327884c3ba62a51bba6d324c3871
//! (v0.9.0, MIT OR Apache-2.0), written for Tauri 2. The port diverges where
//! Tauri 3 or Koharu does:
//! - The plugin is built with `plugin::Builder`, since Tauri 3's `Plugin`
//!   trait has no `extend_api` to register commands with.
//! - No browser SDK is injected; the app starts its own and takes the
//!   transport, breadcrumb, and scope helpers.
//! - The wry webview context and the WebView2 and WebKitGTK crash handlers
//!   go; Tauri 3's runtime-independent termination hook reports renderer
//!   crashes ([`web_content_process_terminated`]).
//! - The Windows and Mac App Store build types go; Koharu ships through
//!   neither.

mod breadcrumbs;
mod browser;
mod commands;
mod context;
mod process;
mod scope;
mod screenshot;
mod termination;
mod transport;

use std::{borrow::Cow, sync::Arc};

use sentry::{
    ClientOptions, Hub,
    protocol::{ClientSdkInfo, ClientSdkPackage, Event},
};
use tauri::{
    Manager as _, Runtime, generate_handler,
    plugin::{Builder, TauriPlugin},
};

pub use breadcrumbs::{BreadcrumbOptions, EventFilter};
pub use screenshot::set_screenshot;
pub use termination::web_content_process_terminated;
pub use transport::Transport;

pub(crate) fn add_sdk_package(sdk: &mut Cow<'_, ClientSdkInfo>) {
    sdk.to_mut().packages.push(ClientSdkPackage {
        name: "cargo:koharu-sentry".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    });
}

/// The Sentry integration whose context the plugin fills in: add it to the
/// client options before `sentry::init`, then add [`plugin`] to the app.
#[derive(Debug, Clone, Default)]
pub struct Sentry {
    breadcrumbs: Arc<BreadcrumbOptions>,
    context: Arc<context::TauriContext>,
}

impl Sentry {
    #[must_use]
    pub fn new(breadcrumbs: BreadcrumbOptions) -> Self {
        Self {
            breadcrumbs: Arc::new(breadcrumbs),
            ..Default::default()
        }
    }

    /// Adds `context` under `key` to every event that lacks one, from any
    /// thread.
    pub fn set_context(&self, key: &str, context: sentry::protocol::Context) {
        self.context.add(key, context);
    }
}

impl sentry::Integration for Sentry {
    fn name(&self) -> &'static str {
        "tauri"
    }

    fn process_event(
        &self,
        mut event: Event<'static>,
        _options: &ClientOptions,
    ) -> Option<Event<'static>> {
        if let Some(sdk) = event.sdk.as_mut() {
            add_sdk_package(sdk);
        }
        process::tag_rust_event(&mut event.tags);
        self.context.apply(&mut event);
        Some(event)
    }
}

/// The plugin for the [`Sentry`] integration of the main hub's client, or
/// for one of its own when the client has none.
#[must_use]
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    let sentry = Hub::main()
        .with_integration(|sentry: &Sentry| Some(sentry.clone()))
        .unwrap_or_default();
    let context = Arc::clone(&sentry.context);
    let builder = Builder::<R>::new("sentry")
        .invoke_handler(generate_handler![
            commands::envelope,
            commands::breadcrumb,
            scope::scope
        ])
        .setup(move |app, _| {
            let client = Hub::main()
                .client()
                .ok_or("sentry::init must run before the Sentry plugin starts")?;
            app.manage((*client).clone());
            context.initialize(app);
            Ok(())
        });
    breadcrumbs::attach(builder, sentry.breadcrumbs, sentry.context).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_adds_sdk_metadata() {
        let options = ClientOptions::new().add_integration(Sentry::default());
        let events = sentry::test::with_captured_events_options(
            || {
                sentry::capture_message("test", sentry::Level::Info);
            },
            options,
        );

        let sdk = events[0].sdk.as_ref().expect("an event has its SDK");
        assert!(sdk.integrations.iter().any(|i| i == "tauri"));
        assert!(sdk.packages.iter().any(|p| p.name == "cargo:koharu-sentry"));
        assert_eq!(events[0].tags["event.process"], "rust");
    }
}
