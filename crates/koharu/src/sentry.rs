use std::sync::Arc;

use sentry::{
    ClientOptions, IntoDsn, User,
    integrations::{
        minidump::MinidumpIntegration,
        tracing::{BreadcrumbLayer, ErrorLayer, LogLayer, SpanLayer},
    },
};
use tracing::Level;
use tracing_subscriber::{
    Layer as _,
    filter::{LevelFilter, filter_fn},
    registry::LookupSpan,
};

/// Where the panic hook logs from. Sentry's panic integration already
/// reports each panic with its stack, so the hook's error isn't a second
/// issue.
const PANIC_TARGET: &str = "koharu::panic";

/// Starts Sentry with everything on: every error and trace, logs, native
/// crash dumps, release health, and the interface's events through
/// `koharu-sentry`. It must run before the async runtime starts, so its
/// threads share the main hub, and before anything that should run only once:
/// the crash reporter re-runs this binary and never returns from here.
pub fn initialize() -> sentry::ClientInitGuard {
    // The crash reporter has its own scope: a native crash carries only the
    // breadcrumbs and user sent to it, through the integration the client
    // shares with this function.
    let minidump = Arc::new(MinidumpIntegration::new());
    let forward = Arc::clone(&minidump);
    let mut options = ClientOptions::new()
        .maybe_release(sentry::release_name!())
        .send_default_pii(true)
        .sample_rate(1.0)
        .traces_sample_rate(1.0)
        .auto_session_tracking(true)
        .before_breadcrumb(move |breadcrumb| {
            forward.add_breadcrumb(breadcrumb.clone());
            Some(breadcrumb)
        })
        .transport(koharu_sentry::Transport)
        .add_integration(koharu_sentry::Sentry::default());
    options.integrations.push(minidump.clone());
    options.dsn = option_env!("SENTRY_DSN")
        .into_dsn()
        .expect("invalid SENTRY_DSN environment variable");
    let guard = sentry::init(options);

    let user = User {
        id: machine_uid::get().ok(),
        ..Default::default()
    };
    sentry::configure_scope(|scope| scope.set_user(Some(user.clone())));
    minidump.set_user(Some(user));
    guard
}

pub fn tracing_layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    // INFO and up as spans and logs; warnings and info as breadcrumbs too,
    // since a crash report carries breadcrumbs and not logs; errors as
    // issues.
    SpanLayer::new()
        .with_filter(LevelFilter::INFO)
        .and_then(LogLayer::new().with_filter(LevelFilter::INFO))
        .and_then(BreadcrumbLayer::new().with_filter(filter_fn(|metadata| {
            matches!(*metadata.level(), Level::WARN | Level::INFO)
        })))
        .and_then(ErrorLayer::new().with_filter(filter_fn(|metadata| {
            *metadata.level() == Level::ERROR && metadata.target() != PANIC_TARGET
        })))
}
