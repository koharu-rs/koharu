//! Paths, overrides, and origin, on a layer's properties as JSON.
//!
//! A path is recorded where the old and new values first differ in shape:
//! one side is missing, or the type, enum variant, or key set differs.
//! Arrays, scalars, and enum values are always whole; attribute maps are
//! compared key by key. So resetting a recorded path always restores a valid
//! value.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::{
    ActionRecord, Pointer, PointerBuf,
    doc::{Generated, Node},
};

pub(crate) fn diff(old: &Map<String, Value>, new: &Map<String, Value>) -> Vec<PointerBuf> {
    let mut paths = Vec::new();
    compare(Pointer::root(), old, new, &mut paths);
    paths
}

fn compare(
    path: &Pointer,
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    out: &mut Vec<PointerBuf>,
) {
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let child = path.with_trailing_token(key);
        match (old.get(key), new.get(key)) {
            (Some(Value::Object(a)), Some(Value::Object(b))) if same_keys(a, b) || open(&child) => {
                compare(&child, a, b, out);
            }
            (a, b) if a != b => out.push(child),
            _ => {}
        }
    }
}

fn same_keys(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    a.len() == b.len() && a.keys().all(|key| b.contains_key(key))
}

/// Attribute maps have open key sets, so they compare key by key.
fn open(path: &Pointer) -> bool {
    path.first()
        .is_some_and(|first| first.decoded() == "attributes")
        && path.count() <= 2
}

pub(crate) fn get<'a>(props: &'a Map<String, Value>, path: &Pointer) -> Option<&'a Value> {
    let mut tokens = path.tokens();
    let first = tokens.next()?;
    tokens.try_fold(props.get(first.decoded().as_ref())?, |value, token| {
        value.as_object()?.get(token.decoded().as_ref())
    })
}

/// Sets the value at a non-root `path`, or removes it for `None`.
pub(crate) fn set(props: &mut Map<String, Value>, path: &Pointer, value: Option<Value>) {
    let Some((parents, last)) = path.split_back() else {
        return;
    };
    let mut map = props;
    for token in parents.tokens() {
        let entry = map
            .entry(token.decoded().into_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        map = entry.as_object_mut().expect("just made an object");
    }
    let last = last.decoded().into_owned();
    match value {
        Some(value) => map.insert(last, value),
        None => map.remove(&last),
    };
}

/// The properties as the models left them: the shown values with each
/// overridden path put back to the model's value.
pub(crate) fn model_view(node: &Node) -> Map<String, Value> {
    let mut props = node.props.clone();
    for (path, generated) in &node.generated {
        if let Generated::Value(value) = generated {
            set(&mut props, path, value.clone());
        }
    }
    props
}

/// True when some model action wrote `path`, part of it, or around it.
pub(crate) fn owned(origin: &[ActionRecord], path: &Pointer) -> bool {
    origin.iter().any(|action| {
        action
            .writes
            .iter()
            .any(|written| written.starts_with(path) || path.starts_with(written))
    })
}

/// The overridden path that `path` lies within, if any.
pub(crate) fn covering<'a>(
    generated: &'a BTreeMap<PointerBuf, Generated>,
    path: &Pointer,
) -> Option<&'a PointerBuf> {
    generated.keys().find(|key| path.starts_with(key))
}

/// Overridden paths inside `path`.
pub(crate) fn inside(
    generated: &BTreeMap<PointerBuf, Generated>,
    path: &Pointer,
) -> Vec<PointerBuf> {
    generated
        .keys()
        .filter(|key| key.starts_with(path))
        .cloned()
        .collect()
}

/// Appends a model's action. A rerun of the same model takes over every path
/// that model wrote on the layer, so its ingredients replace the old ones;
/// another model's action takes over only the paths it writes. An action left
/// with no paths is dropped.
pub(crate) fn take_over(origin: &mut Vec<ActionRecord>, mut action: ActionRecord) {
    let agent = action.action.software_agent.name.clone();
    let own = action.writes.clone();
    for older in origin.iter_mut() {
        if older.action.software_agent.name == agent {
            action.writes.append(&mut older.writes);
        } else {
            older
                .writes
                .retain(|written| !own.iter().any(|new| written.starts_with(new)));
        }
    }
    origin.retain(|older| !older.writes.is_empty());
    // Sorted, a path comes before the paths inside it, which it covers.
    action.writes.sort();
    let mut writes: Vec<PointerBuf> = Vec::new();
    for path in action.writes.drain(..) {
        if !writes.iter().any(|outer| path.starts_with(outer)) {
            writes.push(path);
        }
    }
    action.writes = writes;
    origin.push(action);
}
