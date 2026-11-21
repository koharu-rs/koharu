//! Upstream: src/process.rs.

use std::collections::BTreeMap;

use sentry::Scope;
use tauri::{Runtime, Webview};

/// Same tag key as the Electron SDK.
pub(crate) const PROCESS_TAG: &str = "event.process";
pub(crate) const WEBVIEW_TAG: &str = "tauri.webview";
pub(crate) const WINDOW_TAG: &str = "tauri.window";

pub(crate) const RUST_PROCESS: &str = "rust";
pub(crate) const WEBVIEW_PROCESS: &str = "webview";

/// Tags for events and transactions that came from a webview.
#[derive(Debug, Clone)]
pub(crate) struct WebviewTags {
    webview: String,
    window: String,
}

impl WebviewTags {
    pub(crate) fn new<R: Runtime>(webview: &Webview<R>) -> Self {
        Self::from_labels(webview.label(), webview.window().label())
    }

    pub(crate) fn from_labels(webview: &str, window: &str) -> Self {
        Self {
            webview: webview.into(),
            window: window.into(),
        }
    }

    pub(crate) fn webview(&self) -> &str {
        &self.webview
    }

    fn pairs(&self) -> [(&'static str, &str); 3] {
        [
            (PROCESS_TAG, WEBVIEW_PROCESS),
            (WEBVIEW_TAG, &self.webview),
            (WINDOW_TAG, &self.window),
        ]
    }

    pub(crate) fn apply(&self, tags: &mut BTreeMap<String, String>) {
        for (key, value) in self.pairs() {
            tags.insert(key.into(), value.into());
        }
    }

    /// Scope tags replace event tags, so captured events need them here.
    pub(crate) fn apply_to_scope(&self, scope: &mut Scope) {
        for (key, value) in self.pairs() {
            scope.set_tag(key, value);
        }
    }
}

/// Tags events that do not have a process yet as Rust events.
pub(crate) fn tag_rust_event(tags: &mut BTreeMap<String, String>) {
    tags.entry(PROCESS_TAG.into())
        .or_insert_with(|| RUST_PROCESS.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webview_tags() -> WebviewTags {
        WebviewTags::from_labels("settings-view", "settings")
    }

    #[test]
    fn applies_webview_tags() {
        let mut tags = BTreeMap::new();
        tags.insert(PROCESS_TAG.to_string(), "other".to_string());
        webview_tags().apply(&mut tags);

        assert_eq!(tags[PROCESS_TAG], WEBVIEW_PROCESS);
        assert_eq!(tags[WEBVIEW_TAG], "settings-view");
        assert_eq!(tags[WINDOW_TAG], "settings");
    }

    #[test]
    fn scope_tags_override_rust_scope() {
        let events = sentry::test::with_captured_events(|| {
            sentry::configure_scope(|scope| scope.set_tag(PROCESS_TAG, RUST_PROCESS));
            sentry::with_scope(
                |scope| webview_tags().apply_to_scope(scope),
                || {
                    sentry::capture_message("test", sentry::Level::Info);
                },
            );
        });

        let tags = &events[0].tags;
        assert_eq!(tags[PROCESS_TAG], WEBVIEW_PROCESS);
        assert_eq!(tags[WEBVIEW_TAG], "settings-view");
        assert_eq!(tags[WINDOW_TAG], "settings");
    }

    #[test]
    fn rust_tag_keeps_existing_process() {
        let mut tags = BTreeMap::new();
        tag_rust_event(&mut tags);
        assert_eq!(tags[PROCESS_TAG], RUST_PROCESS);

        let mut tags = BTreeMap::new();
        webview_tags().apply(&mut tags);
        tag_rust_event(&mut tags);
        assert_eq!(tags[PROCESS_TAG], WEBVIEW_PROCESS);
    }
}
