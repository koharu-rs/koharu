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
    ActionRecord, Pointer,
    doc::{Generated, Node},
};

pub(crate) fn diff(old: &Map<String, Value>, new: &Map<String, Value>) -> Vec<Pointer> {
    let mut paths = Vec::new();
    compare(&Pointer::root(), old, new, &mut paths);
    paths
}

fn compare(
    path: &Pointer,
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    out: &mut Vec<Pointer>,
) {
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let child = path.child(key);
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
    let tokens = path.tokens();
    tokens.first().is_some_and(|first| first == "attributes") && tokens.len() <= 2
}

pub(crate) fn get<'a>(props: &'a Map<String, Value>, path: &Pointer) -> Option<&'a Value> {
    let tokens = path.tokens();
    let (first, rest) = tokens.split_first()?;
    rest.iter().try_fold(props.get(first)?, |value, token| {
        value.as_object()?.get(token)
    })
}

/// Sets the value at a non-root `path`, or removes it for `None`.
pub(crate) fn set(props: &mut Map<String, Value>, path: &Pointer, value: Option<Value>) {
    let tokens = path.tokens();
    let Some((last, parents)) = tokens.split_last() else {
        return;
    };
    let mut map = props;
    for token in parents {
        let entry = map
            .entry(token.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        map = entry.as_object_mut().expect("just made an object");
    }
    match value {
        Some(value) => map.insert(last.clone(), value),
        None => map.remove(last),
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
    origin
        .iter()
        .any(|action| action.writes.iter().any(|written| written.overlaps(path)))
}

/// The overridden path that `path` lies within, if any.
pub(crate) fn covering<'a>(
    generated: &'a BTreeMap<Pointer, Generated>,
    path: &Pointer,
) -> Option<&'a Pointer> {
    generated.keys().find(|key| path.within(key))
}

/// Overridden paths inside `path`.
pub(crate) fn inside(generated: &BTreeMap<Pointer, Generated>, path: &Pointer) -> Vec<Pointer> {
    generated
        .keys()
        .filter(|key| key.within(path))
        .cloned()
        .collect()
}

/// Appends a model's action, which takes over the paths it wrote from older
/// actions; an action left with no paths is dropped.
pub(crate) fn take_over(origin: &mut Vec<ActionRecord>, action: ActionRecord) {
    for older in origin.iter_mut() {
        older
            .writes
            .retain(|written| !action.writes.iter().any(|new| written.within(new)));
    }
    origin.retain(|older| !older.writes.is_empty());
    origin.push(action);
}
