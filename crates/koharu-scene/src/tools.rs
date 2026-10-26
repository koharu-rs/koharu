//! Tools for MCP clients and the in-app agent, generated from `Op` and
//! `Query`: a tool's name is its serde tag, its description the variant's
//! doc comment, and its input schema the variant's part of the schemars
//! schema. No tool has code of its own; what tools never send is skipped by
//! schemars attributes on `Op`.

use std::sync::LazyLock;

use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings, json_schema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{
    Answer, Error, Layer, LayerKind, LayerView, NamedStyle, Op, PageView, Query, Result, StyleView,
    doc::Key,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Whether calling it edits the project.
    pub writes: bool,
}

/// A parsed tool call. The host wraps an edit with the author and the reads
/// it tracks, and with its own translated label for a single op tool, which
/// has none.
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
    let (ops, mut defs) = variants::<Op>("op");
    let (queries, query_defs) = variants::<Query>("query");
    defs.extend(query_defs);
    let spec = |name: &str, description: &str, schema: Value, writes: bool| {
        let mut schema = schema;
        let used = reachable(&schema, &defs);
        if !used.is_empty() {
            schema["$defs"] = Value::Object(used);
        }
        ToolSpec {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: schema,
            writes,
        }
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
        return Err(Error::rejected(format!("there is no tool named {name}")));
    };
    let Value::Object(mut arguments) = arguments else {
        return Err(Error::rejected(format!("{name} takes an object")));
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
                    return Err(Error::rejected(format!("{tag:?} is not an op tool")));
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
        return Err(Error::rejected(format!("arguments can't contain `{tag}`")));
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
    serde_json::from_value(value).map_err(|error| Error::rejected(error.to_string()))
}

/// An answer as agents see it: no hashes, which the host keeps for them, and
/// each action reduced to what it was and who did it.
pub fn compact(answer: &Answer) -> Value {
    match answer {
        Answer::Project(project) => json!({
            "id": project.id,
            "version": project.version,
            "pages": project.pages.iter().map(compact_page).collect::<Vec<_>>(),
            "styles": project.styles.iter().map(compact_style).collect::<Vec<_>>(),
        }),
        Answer::Page(page) => compact_page(page),
        Answer::Layer(answer) => json!({
            "page": answer.page,
            "parent": answer.parent,
            "view": compact_layer(&answer.view),
        }),
        Answer::Style(view) => compact_style(view),
        other => serde_json::to_value(other).expect("answers serialize"),
    }
}

fn compact_page(page: &PageView) -> Value {
    json!({
        "id": page.id,
        "size": page.size,
        "layers": page.layers.iter().map(compact_layer).collect::<Vec<_>>(),
    })
}

fn compact_origin(origin: &[crate::ActionRecord]) -> Vec<Value> {
    origin
        .iter()
        .map(|record| json!({ "action": record.action.action, "by": record.action.software_agent.name }))
        .collect()
}

fn compact_style(view: &StyleView) -> Value {
    json!({
        "id": view.id,
        "style": view.style,
        "origin": compact_origin(&view.origin),
        "overrides": view.overrides,
        "stale": view.stale,
    })
}

fn compact_layer(view: &LayerView) -> Value {
    json!({
        "id": view.id,
        "layer": view.layer,
        "origin": compact_origin(&view.origin),
        "overrides": view.overrides,
        "stale": view.stale,
        "style_overrides": view.style_overrides,
        "children": view.children.iter().map(compact_layer).collect::<Vec<_>>(),
        "masks": view.masks.iter().map(compact_layer).collect::<Vec<_>>(),
    })
}

fn root_schema<T: JsonSchema>() -> Map<String, Value> {
    let schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    match schema.to_value() {
        Value::Object(map) => map,
        _ => unreachable!("a root schema is an object"),
    }
}

/// Each variant of a tagged enum as `(tag, description, schema)`, and the
/// shared definitions.
#[allow(clippy::type_complexity)]
fn variants<T: JsonSchema>(tag: &str) -> (Vec<(String, String, Value)>, Map<String, Value>) {
    let mut root = root_schema::<T>();
    let defs = match root.remove("$defs") {
        Some(Value::Object(defs)) => defs,
        _ => Map::new(),
    };
    let Some(Value::Array(variants)) = root.remove("oneOf") else {
        return (Vec::new(), defs);
    };
    let variants = variants
        .into_iter()
        .filter_map(|variant| {
            let name = variant["properties"][tag]["const"].as_str()?.to_owned();
            let description = variant["description"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            Some((name, description, variant))
        })
        .collect();
    (variants, defs)
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

/// The definitions `schema` refers to, directly or through each other.
fn reachable(schema: &Value, defs: &Map<String, Value>) -> Map<String, Value> {
    fn refs<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
        match value {
            Value::Object(map) => {
                if let Some(name) = map
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|r| r.strip_prefix("#/$defs/"))
                {
                    out.push(name);
                }
                map.values().for_each(|value| refs(value, out));
            }
            Value::Array(items) => items.iter().for_each(|item| refs(item, out)),
            _ => {}
        }
    }
    let mut used = Map::new();
    let mut pending = Vec::new();
    refs(schema, &mut pending);
    while let Some(name) = pending.pop() {
        if !used.contains_key(name)
            && let Some(def) = defs.get(name)
        {
            used.insert(name.to_owned(), def.clone());
            refs(def, &mut pending);
        }
    }
    used
}

/// The schema of `update_layer`'s patch: every layer kind with nothing
/// required, `null` allowed everywhere, and `kind` and a text's `content`
/// left out. It registers `LayerPatch` and its `Partial…` definitions.
pub(crate) fn layer_patch(generator: &mut SchemaGenerator) -> Schema {
    if !generator.definitions().contains_key("LayerPatch") {
        let mut layer = root_schema::<Layer>();
        let defs = generator.definitions_mut();
        if let Some(Value::Object(layer_defs)) = layer.remove("$defs") {
            for (name, schema) in layer_defs {
                defs.insert(format!("Partial{name}"), partial(schema));
            }
        }
        let variants: Vec<Value> = match layer.remove("oneOf") {
            Some(Value::Array(variants)) => variants
                .into_iter()
                .map(|mut variant| {
                    let text = variant["properties"][Key::Kind.as_ref()]["const"]
                        == LayerKind::Text.as_ref();
                    if let Some(Value::Object(properties)) = variant.get_mut("properties") {
                        properties.remove(Key::Kind.as_ref());
                        if text {
                            properties.remove(Key::Content.as_ref());
                        }
                    }
                    partial(variant)
                })
                .collect(),
            _ => Vec::new(),
        };
        defs.insert("LayerPatch".to_owned(), json!({ "anyOf": variants }));
    }
    json_schema!({
        "description": "A JSON Merge Patch over the layer's properties; null clears an optional property.",
        "$ref": "#/$defs/LayerPatch",
    })
}

/// The schema of `update_text_style`'s patch: a style with nothing required
/// and `null` allowed everywhere. It registers `StylePatch`.
pub(crate) fn style_patch(generator: &mut SchemaGenerator) -> Schema {
    if !generator.definitions().contains_key("StylePatch") {
        let mut style = root_schema::<NamedStyle>();
        let defs = generator.definitions_mut();
        if let Some(Value::Object(style_defs)) = style.remove("$defs") {
            for (name, schema) in style_defs {
                defs.insert(format!("Partial{name}"), partial(schema));
            }
        }
        style.remove("$schema");
        style.remove("title");
        defs.insert("StylePatch".to_owned(), partial(Value::Object(style)));
    }
    json_schema!({
        "description": "A JSON Merge Patch over the style's values; null clears an optional value.",
        "$ref": "#/$defs/StylePatch",
    })
}

/// A schema with nothing required and `null` allowed for every property,
/// visiting each subschema once.
fn partial(schema: Value) -> Value {
    let Value::Object(mut map) = schema else {
        return schema;
    };
    map.remove("required");
    map.remove("default");
    if let Some(Value::String(reference)) = map.get_mut("$ref")
        && let Some(name) = reference.strip_prefix("#/$defs/")
    {
        *reference = format!("#/$defs/Partial{name}");
    }
    if let Some(Value::Object(properties)) = map.remove("properties") {
        let properties = properties
            .into_iter()
            .map(|(key, schema)| {
                (
                    key,
                    json!({ "anyOf": [partial(schema), { "type": "null" }] }),
                )
            })
            .collect();
        map.insert("properties".to_owned(), Value::Object(properties));
    }
    if let Some(variants) = map.remove("oneOf") {
        map.insert("anyOf".to_owned(), variants);
    }
    for key in ["anyOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(items)) = map.remove(key) {
            map.insert(
                key.to_owned(),
                Value::Array(items.into_iter().map(partial).collect()),
            );
        }
    }
    for key in ["items", "additionalProperties"] {
        if let Some(child) = map.remove(key) {
            map.insert(key.to_owned(), partial(child));
        }
    }
    Value::Object(map)
}
