# koharu-sentry

Sentry for both halves of the app.

The web interface's events, breadcrumbs, and scope go through the Rust SDK, so
the interface and the native side report as one app. It's Sentry's own Tauri
plugin [sentry-tauri], vendored at v0.9.0 and ported to Tauri 3 and CEF. The
links are under [Sources](#sources).

## Goal

A report shows everything that led to it, from whichever side it came. A crash
in a model, an error in a command, and an exception in the interface carry the
same breadcrumbs, the same user, and the same context.

## 1. One app

| Part | Model | Cite |
|---|---|---|
| client | one, the Rust SDK's; the interface's SDK sends through it instead of to Sentry | sentry-tauri's transport passes the browser's events and breadcrumbs to the Rust SDK, and the user, tags, and contexts set in the browser with them [sentry-tauri] |
| scope | one, the main one: the interface's breadcrumbs and scope changes land there, and native crash reports carry it | |
| session | one per run, so release health counts runs, not windows | |
| context | the same app, device, operating system, and screen on every event, and the GPU the window draws with | Sentry's device, OS, app, and GPU contexts [Sentry contexts] |
| screenshot | the window's image, attached to an error | |

A context the app adds reaches every event from every thread, through the
integration rather than the scope, because a thread copies the scope when it
starts and wouldn't see a change made after.

## 2. Where it differs from sentry-tauri

Each difference is Tauri 3's or Koharu's:

- **No injected browser SDK.** The interface starts its own and takes the
  plugin's transport, breadcrumb, and scope helpers.
- **CEF, not the system web view.** There's no web view context to read, and
  instead of WebView2's and WebKitGTK's crash handlers, Tauri 3's termination
  hook reports a crashed renderer process, whichever runtime the app runs on.
- **No store build types.** Koharu ships through neither the Windows nor the
  Mac App Store.
- **Events the Rust SDK can't parse still go through the scope.** The browser
  SDK sends some an older Rust SDK refuses, such as debug images that carry
  source maps' IDs [sentry-rust#1267], or tags that aren't strings. Upstream
  forwards those raw, past the scope, without the user, context, or
  breadcrumbs. Here they're adapted to parse, and the debug images go back on
  before sending.
- **The interface's commands run off the main thread**, which draws the window,
  so an error can wait for the window's image.

## Sources

Read in November 2026.

- Sentry: [sentry-tauri], [Sentry contexts], [sentry-rust#1267]

[sentry-tauri]: https://github.com/getsentry/sentry-tauri
[Sentry contexts]: https://develop.sentry.dev/sdk/data-model/event-payloads/contexts/
[sentry-rust#1267]: https://github.com/getsentry/sentry-rust/issues/1267
