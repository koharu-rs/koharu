//! Upstream: src/scope.rs.

use std::collections::BTreeMap;

use sentry::{
    Hub, Scope, User,
    protocol::{Context, Value},
};
use serde::{Deserialize, Deserializer};
use serde_json::Map;

/// Scope changes from the browser SDK.
///
/// A `null` value removes the key. A `null` user clears the user.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScopeUpdate {
    #[serde(deserialize_with = "present")]
    user: Option<Option<User>>,
    tags: BTreeMap<String, Option<Value>>,
    extra: BTreeMap<String, Option<Value>>,
    contexts: BTreeMap<String, Option<Map<String, Value>>>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

fn to_context(key: &str, mut map: Map<String, Value>) -> Context {
    // Infer the type from the key like the event protocol does
    if !map.contains_key("type") {
        map.insert("type".into(), key.into());
    }
    serde_json::from_value(Value::Object(map.clone())).unwrap_or_else(|_| {
        map.remove("type");
        Context::Other(map.into_iter().collect())
    })
}

impl ScopeUpdate {
    fn apply(self, scope: &mut Scope) {
        if let Some(user) = self.user {
            scope.set_user(user);
        }

        for (key, value) in self.tags {
            match value {
                Some(Value::String(value)) => scope.set_tag(&key, value),
                Some(value) => scope.set_tag(&key, value),
                None => scope.remove_tag(&key),
            }
        }

        for (key, value) in self.extra {
            match value {
                Some(value) => scope.set_extra(&key, value),
                None => scope.remove_extra(&key),
            }
        }

        for (key, value) in self.contexts {
            match value {
                Some(value) => scope.set_context(&key, to_context(&key, value)),
                None => scope.remove_context(&key),
            }
        }
    }
}

// Other threads copy the main hub scope when they first use Sentry.
// The minidump scope sync also falls back to the main hub.
#[tauri::command]
pub fn scope(scope: ScopeUpdate) {
    Hub::main().configure_scope(|s| scope.apply(s));
}

#[cfg(test)]
mod tests {
    use sentry::protocol::Event;
    use serde_json::json;

    use super::*;

    fn apply(scope: &mut Scope, update: Value) {
        serde_json::from_value::<ScopeUpdate>(update)
            .expect("present")
            .apply(scope);
    }

    fn event(scope: &Scope) -> Event<'static> {
        scope.apply_to_event(Event::default()).expect("present")
    }

    #[test]
    fn applies_and_removes_values() {
        let mut scope = Scope::default();
        apply(
            &mut scope,
            json!({
                "user": { "id": "42", "email": "a@b.c" },
                "tags": { "string": "value", "number": 1, "bool": true },
                "extra": { "nested": { "a": [1, 2] } },
                "contexts": {
                    "custom": { "key": "value" },
                    "app": { "app_name": "browser" }
                }
            }),
        );

        let event = event(&scope);
        let user = event.user.expect("present");
        assert_eq!(user.id.as_deref(), Some("42"));
        assert_eq!(user.email.as_deref(), Some("a@b.c"));
        assert_eq!(event.tags["string"], "value");
        assert_eq!(event.tags["number"], "1");
        assert_eq!(event.tags["bool"], "true");
        assert_eq!(event.extra["nested"], json!({ "a": [1, 2] }));
        let Context::Other(custom) = &event.contexts["custom"] else {
            panic!("expected other context");
        };
        assert_eq!(custom["key"], "value");
        assert!(!custom.contains_key("type"));
        let Context::App(app) = &event.contexts["app"] else {
            panic!("expected app context");
        };
        assert_eq!(app.app_name.as_deref(), Some("browser"));

        apply(
            &mut scope,
            json!({
                "tags": { "string": null },
                "extra": { "nested": null },
                "contexts": { "custom": null }
            }),
        );

        let event = self::event(&scope);
        assert!(event.user.is_some());
        assert!(!event.tags.contains_key("string"));
        assert_eq!(event.tags["number"], "1");
        assert!(event.extra.is_empty());
        assert!(!event.contexts.contains_key("custom"));
        assert!(event.contexts.contains_key("app"));
    }

    #[test]
    fn null_user_clears_user() {
        let mut scope = Scope::default();
        scope.set_user(Some(User {
            id: Some("42".into()),
            ..Default::default()
        }));

        apply(&mut scope, json!({ "tags": { "a": "b" } }));
        assert!(event(&scope).user.is_some());

        apply(&mut scope, json!({ "user": null }));
        assert!(event(&scope).user.is_none());
    }

    #[test]
    fn command_updates_main_hub() {
        let update =
            serde_json::from_value(json!({ "tags": { "from": "webview" } })).expect("present");
        std::thread::spawn(move || scope(update))
            .join()
            .expect("present");

        let tags = Hub::main().configure_scope(|s| event(s).tags);
        assert_eq!(tags["from"], "webview");
    }
}
