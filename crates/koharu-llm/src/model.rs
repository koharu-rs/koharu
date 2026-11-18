//! How each vendor's models keep to a JSON schema, which rig's catalog
//! doesn't say.

use rig_agent::agent::{AgentBuilder, OutputMode};
use rig_core::{
    driver::DynModel,
    operation::Completion,
    providers::{chatgpt, deepseek, minimax, openrouter},
};
use serde_json::{Value, json};

use crate::hosted::{OPENCODE_GO, openai_compatible};

/// rig's agent builder on `model`, asking for answers `schema` describes in
/// the way its vendor keeps to one. `prompt_typed` checks an answer; a caller
/// of plain `prompt` checks it itself, as rig fails open on structured output.
#[must_use]
pub fn answers(model: &DynModel<Completion>, schema: schemars::Schema) -> AgentBuilder {
    let spec = model.spec();
    let vendor = spec.map_or("", |spec| spec.provider.vendor());
    let builder = AgentBuilder::new(model.clone())
        .output_schema_raw(schema)
        .output_mode(output_mode(
            vendor,
            spec.is_some_and(|spec| spec.structured_output),
        ));
    match params(vendor) {
        Some(params) => builder.additional_params(params),
        None => builder,
    }
}

/// This stays per vendor because rig's catalog says DeepSeek's models keep to
/// a schema, while its DeepSeek dialect drops the schema. An OpenAI-compatible
/// server may not take a schema at all.
fn output_mode(vendor: &str, structured_output: bool) -> OutputMode {
    match vendor {
        chatgpt::PROVIDER_NAME
        | deepseek::PROVIDER_NAME
        | minimax::PROVIDER_NAME
        | openai_compatible::PROVIDER_NAME => OutputMode::Prompted,
        OPENCODE_GO if !structured_output => OutputMode::Prompted,
        _ => OutputMode::Native,
    }
}

/// What else a request with a schema needs.
fn params(vendor: &str) -> Option<Value> {
    match vendor {
        // rig drops a DeepSeek schema, so DeepSeek is asked for JSON mode.
        deepseek::PROVIDER_NAME => Some(json!({"response_format": {"type": "json_object"}})),
        // Only hosts that keep to the schema serve the request.
        openrouter::PROVIDER_NAME => Some(json!({"provider": {"require_parameters": true}})),
        _ => None,
    }
}
