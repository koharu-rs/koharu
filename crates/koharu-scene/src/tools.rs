//! Tools for MCP clients and the in-app agent, generated from `Op` and
//! `Query`: a tool's name is its serde tag, its description the variant's
//! doc comment, and its input schema the variant's part of the schemars
//! schema. No tool has code of its own; what tools never send is skipped by
//! schemars attributes on `Op`.

use std::sync::LazyLock;

use schemars::{
    JsonSchema, Schema, SchemaGenerator,
    generate::SchemaSettings,
    transform::{RecursiveTransform, Transform},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{Blend, Error, LayerKind, NewLayer, Op, Query, Result};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Whether calling it edits the project.
    pub writes: bool,
}

/// A parsed tool call. The host wraps an edit with its own translated label
/// for a single op tool, which has none.
#[derive(Clone, Debug)]
pub enum ToolCall {
    Edit { label: Option<String>, ops: Vec<Op> },
    Query(Query),
}

static TOOLS: LazyLock<Vec<ToolSpec>> = LazyLock::new(build);

pub fn tools() -> &'static [ToolSpec] {
    &TOOLS
}

fn build() -> Vec<ToolSpec> {
    let ops = variants::<Op>("op");
    let queries = variants::<Query>("query");
    let spec = |name: &str, description: &str, input_schema: Value, writes: bool| ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema,
        writes,
    };
    let mut tools: Vec<ToolSpec> = ops
        .iter()
        .map(|(name, description, schema)| spec(name, description, without_tag(schema, "op"), true))
        .collect();
    tools.push(spec(
        "apply_edit",
        "Apply several ops at once, as one undo step. Later ops can refer to pages and layers \
         created earlier in the batch by the `$label` they gave as `id`.",
        json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "description": "What the batch does, for the undo menu." },
                "ops": { "type": "array", "items": { "anyOf": ops.iter().map(|(_, _, schema)| schema.clone()).collect::<Vec<_>>() } },
            },
            "required": ["label", "ops"],
            "additionalProperties": false,
        }),
        true,
    ));
    tools.extend(queries.iter().map(|(name, description, schema)| {
        spec(name, description, without_tag(schema, "query"), false)
    }));
    tools
}

/// Parses a call to one of `tools()`.
pub fn tool_call(name: &str, arguments: Value) -> Result<ToolCall> {
    let Some(tool) = tools().iter().find(|tool| tool.name == name) else {
        return Err(Error::Rejected(format!("there is no tool named {name}")));
    };
    let Value::Object(mut arguments) = arguments else {
        return Err(Error::Rejected(format!("{name} takes an object")));
    };
    if name == "apply_edit" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Batch {
            label: String,
            ops: Vec<Value>,
        }
        let batch: Batch = parse(Value::Object(arguments))?;
        let ops = batch
            .ops
            .into_iter()
            .map(|op| {
                let tag = op.get("op").and_then(Value::as_str).unwrap_or_default();
                if !tools()
                    .iter()
                    .any(|tool| tool.writes && tool.name == tag && tag != "apply_edit")
                {
                    return Err(Error::Rejected(format!("{tag:?} is not an op tool")));
                }
                parse(op)
            })
            .collect::<Result<_>>()?;
        return Ok(ToolCall::Edit {
            label: Some(batch.label),
            ops,
        });
    }
    let tag = if tool.writes { "op" } else { "query" };
    if arguments.contains_key(tag) {
        return Err(Error::Rejected(format!("arguments can't contain `{tag}`")));
    }
    arguments.insert(tag.to_owned(), Value::String(name.to_owned()));
    let value = Value::Object(arguments);
    Ok(match tool.writes {
        true => ToolCall::Edit {
            label: None,
            ops: vec![parse(value)?],
        },
        false => ToolCall::Query(parse(value)?),
    })
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| Error::Rejected(error.to_string()))
}

/// Each variant of a tagged enum as `(tag, description, schema)`, with every
/// subschema inlined.
fn variants<T: JsonSchema>(tag: &str) -> Vec<(String, String, Value)> {
    let schema = SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>();
    let Some(Value::Array(variants)) = schema.get("oneOf") else {
        return Vec::new();
    };
    variants
        .iter()
        .filter_map(|variant| {
            let name = variant["properties"][tag]["const"].as_str()?.to_owned();
            let description = variant["description"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            Some((name, description, variant.clone()))
        })
        .collect()
}

fn without_tag(schema: &Value, tag: &str) -> Value {
    let mut schema = schema.clone();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.remove(tag);
    }
    if let Some(Value::Array(required)) = schema.get_mut("required") {
        required.retain(|key| key != tag);
    }
    schema
}

/// The schema of `update_layer`'s patch: a new layer's values with nothing
/// required and `null` allowed where a value is optional, without what a
/// patch can't name: `kind`, `attributes`, and a text's `content`, a
/// translation's included.
pub(crate) fn layer_patch(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = generator.subschema_for::<NewLayer>();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.remove("attributes");
        // A new layer's defaults leave the values every layer has out of
        // `required`, and its kind picks a blend it leaves out, but a stored
        // layer has every value, so none clears.
        properties.insert(
            "blend".to_owned(),
            generator.subschema_for::<Blend>().into(),
        );
        let own: Vec<Value> = properties.keys().cloned().map(Value::from).collect();
        schema.insert("required".to_owned(), own.into());
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(variants)) = schema.get_mut(key) {
            for variant in variants {
                let kind = variant["properties"]["kind"]["const"].clone();
                let Some(Value::Object(properties)) = variant.get_mut("properties") else {
                    continue;
                };
                properties.remove("kind");
                if kind == LayerKind::Text.as_ref() {
                    properties.remove("content");
                }
                if kind == LayerKind::Block.as_ref()
                    && let Some(translation) = properties.get_mut("translation")
                {
                    RecursiveTransform(|schema: &mut Schema| {
                        if let Some(Value::Object(properties)) = schema.get_mut("properties") {
                            properties.remove("content");
                        }
                    })
                    .transform(
                        <&mut Schema>::try_from(translation).expect("a property is a schema"),
                    );
                }
            }
        }
    }
    RecursiveTransform(partial).transform(&mut schema);
    schema.insert(
        "description".to_owned(),
        "A JSON Merge Patch over the layer's values; null clears an optional value.".into(),
    );
    schema
}

/// One schema with nothing required, and `null` allowed for what wasn't
/// required, which a merge patch clears to its default or to none.
fn partial(schema: &mut Schema) {
    let required = schema.remove("required").unwrap_or_default();
    schema.remove("default");
    if let Some(variants) = schema.remove("oneOf") {
        schema.insert("anyOf".to_owned(), variants);
    }
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for (key, property) in properties.iter_mut() {
            let optional = required
                .as_array()
                .is_none_or(|required| !required.contains(&Value::from(key.as_str())));
            if optional {
                *property = json!({ "anyOf": [property.take(), { "type": "null" }] });
            }
        }
    }
}
