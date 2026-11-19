//! How each vendor's models keep to a JSON schema, which rig's catalog
//! doesn't say.

use futures::TryStreamExt as _;
use rig_agent::{
    agent::{AgentBuilder, AgentRunner, MultiTurnStreamItem, OutputMode},
    run::{PromptError, PromptResponse},
};
use rig_core::{
    catalog::ModelSpec,
    completion::{Effort, GenerationOptions, Reasoning},
    driver::DynModel,
    error::ProviderError,
    operation::Completion,
    providers::{chatgpt, deepseek, minimax, openrouter},
};
use serde_json::{Value, json};

use crate::hosted::{OPENCODE_GO, openai_compatible};

/// One structured answer.
pub async fn answer(
    model: &DynModel<Completion>,
    schema: schemars::Schema,
    instruction: &str,
    input: String,
    reasoning: Option<Reasoning>,
) -> Result<String, PromptError> {
    let spec = model.spec();
    let reasoning = reasoning.filter(|choice| {
        *choice != Reasoning::Off
            || spec
                .is_some_and(|spec| go_deepseek(spec) || spec.reasoning.can_disable() == Some(true))
    });
    let request = answers(model, schema, reasoning)
        .preamble(instruction)
        .build()
        .prompt(input);
    Ok(complete(model, request).await?.output())
}

/// Go streams long responses so that the connection can deliver the model's
/// reasoning and text as it generates them.
pub async fn complete(
    model: &DynModel<Completion>,
    request: AgentRunner,
) -> Result<PromptResponse, PromptError> {
    if model
        .spec()
        .is_some_and(|spec| spec.provider.vendor() == OPENCODE_GO)
    {
        let mut items = request.stream();
        let started = std::time::Instant::now();
        let mut received = false;
        while let Some(item) = items.try_next().await? {
            if !received {
                tracing::info!(
                    elapsed_seconds = started.elapsed().as_secs_f64(),
                    "first streamed model response event"
                );
                received = true;
            }
            if let MultiTurnStreamItem::FinalResponse(response) = item {
                return Ok(response);
            }
        }
        Err(ProviderError::Truncated.into())
    } else {
        request.await
    }
}

/// rig's agent builder on `model`, asking for answers `schema` describes in
/// the way its vendor keeps to one. `prompt_typed` checks an answer; a caller
/// of plain `prompt` checks it itself, as rig fails open on structured output.
#[must_use]
pub fn answers(
    model: &DynModel<Completion>,
    schema: schemars::Schema,
    reasoning: Option<Reasoning>,
) -> AgentBuilder {
    let spec = model.spec();
    let vendor = spec.map_or("", |spec| spec.provider.vendor());
    let mut builder = AgentBuilder::new(model.clone())
        .output_schema_raw(schema)
        .output_mode(output_mode(
            vendor,
            spec.is_some_and(|spec| spec.structured_output),
        ));
    // The Go gateway uses a generic OpenAI wire. Send the model's native
    // fields on this route so rig does not reject the portable option.
    if let Some(fields) = reasoning
        .filter(|_| spec.is_some_and(go_deepseek))
        .and_then(deepseek_reasoning)
    {
        return builder.additional_params(fields);
    }
    if let Some(reasoning) = reasoning {
        builder = builder.options(GenerationOptions::default().reasoning(reasoning));
    }
    match params(vendor) {
        Some(params) => builder.additional_params(params),
        None => builder,
    }
}

pub fn go_deepseek(spec: &ModelSpec) -> bool {
    spec.provider.vendor() == OPENCODE_GO && spec.id.starts_with("deepseek-v4")
}

fn deepseek_reasoning(reasoning: Reasoning) -> Option<Value> {
    let (mode, effort) = match reasoning {
        Reasoning::Off => ("disabled", "none"),
        Reasoning::Effort(Effort::Minimal | Effort::Low) => ("enabled", "low"),
        Reasoning::Effort(Effort::Max) => ("enabled", "max"),
        Reasoning::Effort(_) => ("enabled", "high"),
        _ => return None,
    };
    Some(json!({"thinking": {"type": mode}, "reasoning_effort": effort}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_go_maps_translation_effort_and_glossary_off() {
        assert_eq!(
            deepseek_reasoning(Reasoning::Effort(Effort::Medium)),
            Some(json!({"thinking": {"type": "enabled"}, "reasoning_effort": "high"}))
        );
        assert_eq!(
            deepseek_reasoning(Reasoning::Off),
            Some(json!({"thinking": {"type": "disabled"}, "reasoning_effort": "none"}))
        );
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
