//! The schemas `Op` gives its own fields, where schemars' derive can't say
//! what an op takes.

use schemars::{
    Schema, SchemaGenerator,
    transform::{RecursiveTransform, Transform},
};
use serde_json::{Value, json};

use crate::{Blend, LayerKind, NewLayer};

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
