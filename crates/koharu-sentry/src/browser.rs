//! Not upstream: sentry-rust 0.49 can't parse every event the browser SDK
//! sends, and upstream forwards one it can't raw, past the Rust scope: no
//! user, context, breadcrumbs, or tags. Two things fail it: the `sourcemap`
//! debug images that carry source maps' debug IDs
//! (getsentry/sentry-rust#1267), and tags that aren't strings, such as the
//! Next.js SDK's `turbopack: true`. This sets an event's `debug_meta` aside
//! and writes its tags as the strings Sentry keeps them as, so the event
//! parses and goes through the scope, and puts `debug_meta` back into the
//! event the transport sends.

use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
};

use sentry::Envelope;
use serde_json::Value;

/// How many events' `debug_meta` wait for their event at most; one the
/// client drops leaves its entry until it falls off.
const WAITING: usize = 32;

static KEPT: Mutex<VecDeque<(String, Value)>> = Mutex::new(VecDeque::new());

/// An envelope item: its header and its payload.
type Item = (Value, Vec<u8>);

/// `bytes` with its event adapted to what sentry-rust parses: `debug_meta`
/// taken out and kept for [`restore`], and tags as strings. `None` when
/// there was nothing to adapt.
pub(crate) fn adapt(bytes: &[u8]) -> Option<Vec<u8>> {
    let (header, mut items) = split(bytes)?;
    let mut adapted = false;
    for (item, payload) in &mut items {
        if item.get("type").and_then(Value::as_str) != Some("event") {
            continue;
        }
        let mut event: Value = serde_json::from_slice(payload).ok()?;
        let fields = event.as_object_mut()?;
        if let Some(tags) = fields.get_mut("tags").and_then(Value::as_object_mut) {
            for value in tags.values_mut() {
                if !value.is_string() {
                    *value = Value::String(match value.take() {
                        Value::Null => String::new(),
                        other => other.to_string(),
                    });
                    adapted = true;
                }
            }
        }
        if let Some(id) = fields
            .get("event_id")
            .and_then(Value::as_str)
            .map(normalize)
            && let Some(meta) = fields.remove("debug_meta")
        {
            let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
            if kept.len() == WAITING {
                kept.pop_front();
            }
            kept.push_back((id, meta));
            adapted = true;
        }
        *payload = serde_json::to_vec(&event).ok()?;
    }
    adapted.then(|| join(&header, &mut items))
}

/// `envelope` with the `debug_meta` [`adapt`] kept for its event put back,
/// as a raw envelope; as it was when nothing was kept for it.
pub(crate) fn restore(envelope: Envelope) -> Envelope {
    let Some(id) = envelope
        .event()
        .map(|event| normalize(&event.event_id.to_string()))
    else {
        return envelope;
    };
    let meta = {
        let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(index) = kept.iter().position(|(kept, _)| *kept == id) else {
            return envelope;
        };
        kept.remove(index).map(|(_, meta)| meta)
    };
    let Some(meta) = meta else {
        return envelope;
    };
    let mut bytes = Vec::new();
    if envelope.to_writer(&mut bytes).is_err() {
        return envelope;
    }
    let Some((header, mut items)) = split(&bytes) else {
        return envelope;
    };
    for (item, payload) in &mut items {
        if item.get("type").and_then(Value::as_str) != Some("event") {
            continue;
        }
        let Ok(mut event) = serde_json::from_slice::<Value>(payload) else {
            return envelope;
        };
        if let Some(fields) = event.as_object_mut() {
            fields.insert("debug_meta".to_owned(), meta.clone());
        }
        let Ok(written) = serde_json::to_vec(&event) else {
            return envelope;
        };
        *payload = written;
    }
    Envelope::from_bytes_raw(join(&header, &mut items)).unwrap_or(envelope)
}

/// An event ID as the protocol writes it: 32 hex digits, no dashes.
fn normalize(id: &str) -> String {
    id.replace('-', "").to_ascii_lowercase()
}

/// An envelope's header line and its items, each a header and a payload.
fn split(bytes: &[u8]) -> Option<(Vec<u8>, Vec<Item>)> {
    let line = |start: usize| {
        let end = bytes[start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(bytes.len(), |at| start + at);
        (&bytes[start..end], (end + 1).min(bytes.len()))
    };
    let (header, mut at) = line(0);
    let mut items = Vec::new();
    while at < bytes.len() {
        let (item, next) = line(at);
        at = next;
        if item.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let item: Value = serde_json::from_slice(item).ok()?;
        let payload = match item.get("length").and_then(Value::as_u64) {
            Some(length) => {
                let end = at.checked_add(usize::try_from(length).ok()?)?;
                let payload = bytes.get(at..end)?.to_vec();
                at = (end + 1).min(bytes.len());
                payload
            }
            None => {
                let (payload, next) = line(at);
                at = next;
                payload.to_vec()
            }
        };
        items.push((item, payload));
    }
    Some((header.to_vec(), items))
}

/// The envelope `split` read, each item's length set to its payload's.
fn join(header: &[u8], items: &mut [Item]) -> Vec<u8> {
    let mut bytes = header.to_vec();
    for (item, payload) in items {
        if let Some(fields) = item.as_object_mut() {
            fields.insert("length".to_owned(), payload.len().into());
        }
        bytes.push(b'\n');
        bytes.extend(serde_json::to_vec(item).unwrap_or_default());
        bytes.push(b'\n');
        bytes.extend_from_slice(payload);
    }
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An envelope as the Next.js SDK sends one, with a tag that isn't a
    /// string and a source map's debug image, both of which the typed parser
    /// refuses.
    const BROWSER: &str = concat!(
        r#"{"event_id":"9ec79c33ec9942ab8353589fcb2e04dc"}"#,
        "\n",
        r#"{"type":"event"}"#,
        "\n",
        r#"{"event_id":"9ec79c33ec9942ab8353589fcb2e04dc","level":"error","message":"boom","#,
        r#""tags":{"turbopack":true},"#,
        r#""debug_meta":{"images":[{"type":"sourcemap","code_file":"app:///a.js","#,
        r#""debug_id":"3a1f2b4c-5d6e-4f70-8192-a3b4c5d6e7f8"}]}}"#,
        "\n",
    );

    #[test]
    fn a_browser_event_parses_once_adapted_and_sends_its_source_maps() {
        assert!(Envelope::from_slice(BROWSER.as_bytes()).is_err());
        let adapted = adapt(BROWSER.as_bytes()).expect("the event needs adapting");
        let envelope = Envelope::from_slice(&adapted).expect("the adapted event parses");
        let event = envelope.event().expect("an event").clone();
        assert_eq!(event.tags["turbopack"], "true");

        let mut sent = Envelope::new();
        sent.add_item(event);
        let mut bytes = Vec::new();
        restore(sent)
            .to_writer(&mut bytes)
            .expect("the envelope writes");
        let (_, items) = split(&bytes).expect("the restored envelope splits");
        let event: Value = serde_json::from_slice(&items[0].1).expect("the event is JSON");
        assert_eq!(event["debug_meta"]["images"][0]["type"], "sourcemap");
        assert_eq!(event["message"], "boom");
    }
}
