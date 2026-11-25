//! How each vendor's models keep to a JSON schema, which rig's catalog
//! doesn't say.

use anyhow::Result;
use rig_agent::agent::{
    AgentBuilder, AgentHook, HookContext, ModelTurnAction, ModelTurnFinished, OutputMode,
};
use rig_core::{
    completion::{AssistantContent, FinishReason},
    driver::DynModel,
    operation::Completion,
    providers::{chatgpt, deepseek, minimax, openrouter},
    structured_output::json_in_text,
};
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use crate::hosted::{openai_compatible, routing};

/// rig's agent builder on `model`, asking for answers `schema` describes in
/// the way its vendor keeps to one. An answer that doesn't fit it is asked for
/// once more, and fails the run if that one doesn't either.
pub fn answers(model: &DynModel<Completion>, schema: schemars::Schema) -> Result<AgentBuilder> {
    let spec = model.spec();
    let vendor = spec.map_or("", |spec| spec.provider.vendor());
    let id = spec.map_or("", |spec| spec.id.as_str());
    let builder = AgentBuilder::new(model.clone())
        .output_schema_raw(schema)
        .output_mode(output_mode(vendor))
        .default_max_turns(2)
        .add_hook(Whole);
    Ok(match params(vendor, id)? {
        Some(params) => builder.additional_params(params),
        None => builder,
    })
}

/// Stops a run at an answer its token limit cut off: asked for again, it would
/// have less room, as the cut one goes with the request.
struct Whole;

impl AgentHook for Whole {
    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let answer = AssistantContent::concat_text(event.content);
        if event.finish_reason == Some(&FinishReason::Length)
            && json_in_text::<IgnoredAny>(&answer).is_err()
        {
            return ModelTurnAction::Stop("the answer reached the model's token limit".to_owned());
        }
        ModelTurnAction::Continue
    }
}

/// This stays per vendor because rig's catalog says DeepSeek's models keep to
/// a schema, while its DeepSeek dialect drops the schema. An OpenAI-compatible
/// server may not take a schema at all.
fn output_mode(vendor: &str) -> OutputMode {
    match vendor {
        chatgpt::PROVIDER_NAME
        | deepseek::PROVIDER_NAME
        | minimax::PROVIDER_NAME
        | openai_compatible::PROVIDER_NAME => OutputMode::Prompted,
        _ => OutputMode::Native,
    }
}

/// What else a request to model `id` with a schema needs.
fn params(vendor: &str, id: &str) -> Result<Option<Value>> {
    Ok(match vendor {
        // rig drops a DeepSeek schema, so DeepSeek is asked for JSON mode.
        deepseek::PROVIDER_NAME => Some(json!({"response_format": {"type": "json_object"}})),
        // Only hosts that keep to the schema serve the request, in the order
        // or on the host the user chose.
        openrouter::PROVIDER_NAME => Some(json!({"provider": routing::request(id)?})),
        _ => None,
    })
}
