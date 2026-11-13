//! Upstream: src/webview_crash.rs. Divergence: Tauri 3 reports a renderer's
//! end through one runtime-independent hook, which CEF calls with a reason,
//! so this replaces upstream's WebView2 and WebKitGTK handlers; the reasons
//! and levels stay those upstream took from the Electron SDK.

use sentry::{
    Breadcrumb, Hub, Level,
    protocol::{Event, Map, SessionStatus, Value},
};
use tauri::{
    Runtime, Webview,
    webview::{WebContentProcessTermination, WebContentProcessTerminationReason as Reason},
};

use crate::process::WebviewTags;

/// Captures a webview's web content process ending. Pass it to
/// `tauri::Builder::on_web_content_process_terminate`.
pub fn web_content_process_terminated<R: Runtime>(
    webview: &Webview<R>,
    termination: &WebContentProcessTermination,
) {
    let mut data = Map::new();
    if let Some(code) = termination.error_code {
        data.insert("error_code".into(), code.into());
    }
    if let Some(error) = &termination.error_string {
        data.insert("error".into(), error.clone().into());
    }
    capture(
        &WebviewTags::new(webview),
        reason(&termination.reason),
        data,
    );
}

/// The Electron SDK's name for a reason; an unknown one is abnormal, since
/// a runtime that knows a clean exit reports it.
fn reason(reason: &Reason) -> &'static str {
    match reason {
        Reason::Normal => "clean-exit",
        Reason::Killed => "killed",
        Reason::Crashed => "crashed",
        Reason::OutOfMemory => "oom",
        Reason::LaunchFailed => "launch-failed",
        Reason::IntegrityFailure => "integrity-failure",
        _ => "abnormal-exit",
    }
}

/// Uses the same levels as the Electron SDK.
fn level_for_reason(reason: &str) -> Level {
    match reason {
        "abnormal-exit" | "killed" => Level::Warning,
        "crashed" | "oom" | "launch-failed" | "integrity-failure" => Level::Fatal,
        _ => Level::Debug,
    }
}

fn capture(tags: &WebviewTags, reason: &str, mut data: Map<String, Value>) {
    let hub = Hub::current();
    let level = level_for_reason(reason);
    let message = format!("'webview' process exited with '{reason}'");

    if level >= Level::Warning {
        hub.with_scope(
            |scope| tags.apply_to_scope(scope),
            || {
                hub.capture_event(Event {
                    message: Some(message.clone()),
                    level,
                    ..Default::default()
                })
            },
        );
    }

    data.insert("reason".into(), reason.into());
    data.insert("label".into(), tags.webview().into());
    hub.add_breadcrumb(Breadcrumb {
        ty: "process".into(),
        category: Some("child-process".into()),
        message: Some(message),
        level,
        data,
        ..Default::default()
    });

    if level == Level::Fatal {
        hub.end_session_with_status(SessionStatus::Crashed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_is_fatal_and_a_kill_a_warning() {
        assert_eq!(level_for_reason(reason(&Reason::Crashed)), Level::Fatal);
        assert_eq!(level_for_reason(reason(&Reason::OutOfMemory)), Level::Fatal);
        assert_eq!(level_for_reason(reason(&Reason::Killed)), Level::Warning);
        assert_eq!(level_for_reason(reason(&Reason::Unknown)), Level::Warning);
        assert_eq!(level_for_reason(reason(&Reason::Normal)), Level::Debug);
    }

    #[test]
    fn captures_a_crash_with_its_webview() {
        let events = sentry::test::with_captured_events(|| {
            capture(
                &WebviewTags::from_labels("main", "main"),
                "crashed",
                Map::new(),
            );
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, Level::Fatal);
        assert_eq!(events[0].tags["event.process"], "webview");
        assert_eq!(events[0].tags["tauri.webview"], "main");
    }
}
