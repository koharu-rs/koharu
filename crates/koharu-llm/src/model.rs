//! How each vendor's models keep to a JSON schema, which rig's catalog
//! doesn't say.

use rig_agent::agent::{
    AgentBuilder, AgentHook, HookContext, ModelTurnAction, ModelTurnFinished, OutcomeAction,
    OutcomeEvent, OutputMode,
};
use rig_core::{
    completion::{AssistantContent, FinishReason},
    driver::DynModel,
    effect::Answered,
    error::{ErrorKind, ErrorReport},
    operation::Completion,
    providers::{chatgpt, deepseek, minimax, openrouter},
    structured_output::json_in_text,
};
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use crate::hosted::openai_compatible;

/// rig's agent builder on `model`, asking for answers `schema` describes in
/// the way its vendor keeps to one. An answer that doesn't fit it is asked for
/// once more, and fails the run if that one doesn't either.
#[must_use]
pub fn answers(model: &DynModel<Completion>, schema: schemars::Schema) -> AgentBuilder {
    let vendor = model.spec().map_or("", |spec| spec.provider.vendor());
    let builder = AgentBuilder::new(model.clone())
        .output_schema_raw(schema)
        .output_mode(output_mode(vendor))
        .default_max_turns(2)
        .add_hook(Whole);
    match params(vendor) {
        Some(params) => builder.additional_params(params),
        None => builder,
    }
}

/// Stops a run at an answer its provider filtered, or its token limit cut off:
/// asked for again, the cut one would have less room, as it goes with the
/// request.
struct Whole;

impl AgentHook for Whole {
    /// A filtered answer is refused, as a blocked request is.
    async fn on_outcome(&self, _ctx: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        match event.outcome {
            Ok(Answered::Completion(answer))
                if answer.finish_reason() == Some(FinishReason::ContentFilter) =>
            {
                OutcomeAction::replace(Err(ErrorReport {
                    refusal: true,
                    ..ErrorReport::new(ErrorKind::Provider, "the provider blocked the answer")
                }))
            }
            _ => OutcomeAction::proceed(),
        }
    }

    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let answer = AssistantContent::concat_text(event.content);
        if event.finish_reason == Some(&FinishReason::Length)
            && json_in_text::<IgnoredAny>(&answer).is_err()
        {
            return ModelTurnAction::stop("the model's answer ran on too long and was cut off");
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
