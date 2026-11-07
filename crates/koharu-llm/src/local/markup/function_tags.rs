//! The call syntax Qwen's and LFM3's chat templates write:
//! `<function=name><parameter=key>value</parameter></function>`, each value
//! on its own lines, a string as it is and anything else as JSON.

use std::sync::Arc;

use rig_core::completion::ToolDefinition;
use serde_json::{Map, Value};

use super::Call;

const PARAMETER: &str = "<parameter=";
const PARAMETER_END: &str = "</parameter>";

pub(super) fn parse(body: &str, tools: &[Arc<ToolDefinition>]) -> Result<Call, String> {
    let body = body.trim();
    let function = body
        .strip_prefix("<function=")
        .and_then(|function| function.split_once('>'))
        .filter(|(name, _)| !name.trim().is_empty());
    let Some((name, rest)) = function else {
        return Err(format!("a tool call names no function: {body}"));
    };
    let name = name.trim();
    let properties = tools
        .iter()
        .find(|tool| tool.name.as_str() == name)
        .and_then(|tool| tool.parameters.get("properties"));
    let arguments = match parameters(rest, properties) {
        Some(arguments) => Value::Object(arguments).to_string(),
        None => rest.trim().to_owned(),
    };
    Ok(Call {
        name: name.to_owned(),
        arguments,
    })
}

fn parameters(mut rest: &str, properties: Option<&Value>) -> Option<Map<String, Value>> {
    let mut arguments = Map::new();
    while let Some(at) = rest.find(PARAMETER) {
        let (key, after) = rest[at + PARAMETER.len()..].split_once('>')?;
        let (value, after) = after.split_once(PARAMETER_END)?;
        // The template puts the value on its own lines.
        let value = value.strip_prefix('\n').unwrap_or(value);
        let value = value.strip_suffix('\n').unwrap_or(value);
        let schema = properties.and_then(|properties| properties.get(key));
        arguments.insert(key.trim().to_owned(), read(value, schema));
        rest = after;
    }
    // A function that never closes was cut short.
    rest.contains("</function>").then_some(arguments)
}

/// A string as written, and anything else as the JSON the template wrote. What
/// doesn't read stays text for the schema check to refuse.
fn read(value: &str, schema: Option<&Value>) -> Value {
    if schema.is_some_and(takes_string) {
        return Value::String(value.to_owned());
    }
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()))
}

fn takes_string(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => kind == "string",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "string"),
        _ => false,
    }
}
