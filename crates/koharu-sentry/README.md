# koharu-sentry

Sentry's Tauri plugin, vendored from getsentry/sentry-tauri v0.9.0 and ported to Tauri 3 and CEF, so the web interface's events, breadcrumbs, and scope go through the Rust SDK and both sides report as one app: one client, one scope that native crash reports also carry, one session per run, and the same app, device, and screen context on every event, with renderer crashes reported through Tauri's runtime-independent termination hook.
