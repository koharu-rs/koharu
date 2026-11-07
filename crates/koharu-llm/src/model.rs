//! A connected model, and how each provider keeps to a schema.

use rig_agent::agent::{AgentBuilder, OutputMode};
use rig_core::{driver::DynModel, operation::Completion};
use serde_json::{Value, json};

use crate::Provider;

/// A model to send rig's requests to. Its `completion` carries its
/// [`ModelSpec`](rig_core::catalog::ModelSpec).
#[derive(Clone, Debug)]
pub struct Model {
    pub completion: DynModel<Completion>,
    pub(crate) provider: Provider,
}

impl Model {
    /// rig's agent builder on this model.
    #[must_use]
    pub fn agent(&self) -> AgentBuilder {
        AgentBuilder::new(self.completion.clone())
    }

    /// rig's agent builder on this model, asking for answers `schema`
    /// describes in the way this model keeps to one. `prompt_typed` checks an
    /// answer; a caller of plain `prompt` checks it itself, as rig fails open
    /// on structured output.
    #[must_use]
    pub fn answers(&self, schema: schemars::Schema) -> AgentBuilder {
        let builder = self
            .agent()
            .output_schema_raw(schema)
            .output_mode(self.provider.output_mode());
        match self.provider.schema_params() {
            Some(params) => builder.additional_params(params),
            None => builder,
        }
    }
}

impl Provider {
    /// How the provider's models keep to a JSON schema. This stays per
    /// provider because rig's catalog says DeepSeek's do, while its DeepSeek
    /// dialect drops the schema.
    fn output_mode(self) -> OutputMode {
        match self {
            Self::ChatGpt | Self::DeepSeek | Self::MiniMax => OutputMode::Prompted,
            _ => OutputMode::Native,
        }
    }

    /// What else a request with a schema needs.
    fn schema_params(self) -> Option<Value> {
        match self {
            // rig drops a DeepSeek schema, so DeepSeek is asked for JSON mode.
            Self::DeepSeek => Some(json!({"response_format": {"type": "json_object"}})),
            // Only providers that keep to the schema serve the request.
            Self::OpenRouter => Some(json!({"provider": {"require_parameters": true}})),
            _ => None,
        }
    }
}
