//! Language models for Koharu, local and hosted.

mod agent;
mod chatgpt;
mod context;
mod hosted;
mod local;
mod model;

use anyhow::Result;
use rig_core::catalog::Pricing;
use serde::{Deserialize, Serialize};

pub use agent::{Agent, AgentConfig, Event, Host, Invocation, Tool};
pub use chatgpt::{sign_in, sign_out};
pub use context::Context;
pub use hosted::{Endpoint, Providers};
pub use local::{Sampling, Size};
pub use model::Model;
pub use rig_agent::agent::AgentBuilder;
/// rig, whose requests and streams every model here takes and gives.
pub use rig_core as rig;
pub use rig_memory::{CompactReason, SummaryPrompts};

/// Who answers a request.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    Serialize,
    strum::Display,
    strum::VariantArray,
)]
pub enum Provider {
    #[serde(rename = "local")]
    #[strum(serialize = "local")]
    Local,
    #[serde(rename = "openai")]
    #[strum(serialize = "openai")]
    OpenAi,
    #[serde(rename = "chatgpt")]
    #[strum(serialize = "chatgpt")]
    ChatGpt,
    #[serde(rename = "claude")]
    #[strum(serialize = "claude")]
    Claude,
    #[serde(rename = "gemini")]
    #[strum(serialize = "gemini")]
    Gemini,
    #[serde(rename = "xai")]
    #[strum(serialize = "xai")]
    Xai,
    #[serde(rename = "deepseek")]
    #[strum(serialize = "deepseek")]
    DeepSeek,
    #[serde(rename = "minimax")]
    #[strum(serialize = "minimax")]
    MiniMax,
    #[serde(rename = "openrouter")]
    #[strum(serialize = "openrouter")]
    OpenRouter,
    #[serde(rename = "lm-studio")]
    #[strum(serialize = "lm-studio")]
    LmStudio,
    #[serde(rename = "openai-compatible")]
    #[strum(serialize = "openai-compatible")]
    OpenAiCompatible,
}

/// A model the user picked.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Selection {
    pub provider: Provider,
    pub model: String,
    /// A local model's quantization; its first when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
}

/// The model `selection` names, as a rig model, its files fetched first if the
/// store doesn't have them; a local one loads with `size`, which a hosted one
/// doesn't use.
pub async fn connect(selection: &Selection, size: Size) -> Result<Model> {
    let completion = match selection.provider {
        Provider::Local => {
            let entry = local::catalog::entry(&selection.model)?;
            let quantization = entry.quantization(selection.quantization.as_deref())?;
            let (model, projector) = match entry.projector {
                Some(projector) => {
                    let [model, projector] = entry
                        .repository
                        .resolve([quantization.file, projector])
                        .await?;
                    (model, Some(projector))
                }
                None => {
                    let [model] = entry.repository.resolve([quantization.file]).await?;
                    (model, None)
                }
            };
            local::load(
                local::Loading {
                    id: entry.id.to_owned(),
                    model,
                    projector,
                    defaults: entry.sampling.clone(),
                    markup: entry.markup,
                },
                size,
            )
            .await?
        }
        provider => hosted::model(provider, &selection.model)?,
    };
    Ok(Model {
        completion,
        provider: selection.provider,
    })
}

/// A model the user can pick, and what it can do.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelInfo {
    pub provider: Provider,
    pub id: String,
    pub name: String,
    /// A local model's quantizations, the default first; none for a hosted
    /// one.
    pub quantizations: Vec<Quantization>,
    pub images: bool,
    pub tools: bool,
    pub reasoning: bool,
    /// The languages a local model translates into, as BCP 47 tags; `None`
    /// for any.
    pub languages: Option<&'static [&'static str]>,
    /// What rig's catalog says of a hosted model; a local model's window
    /// depends on how it loads.
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub pricing: Option<Pricing>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Quantization {
    pub id: &'static str,
    pub name: &'static str,
    pub downloaded: bool,
}

/// Every model the user can pick: the catalog's, and each hosted
/// provider's that has a key or a sign-in. A provider whose list fails is
/// left out, with the failure logged.
pub async fn models() -> Vec<ModelInfo> {
    let mut models = local::catalog::MODELS
        .iter()
        .map(|entry| ModelInfo {
            provider: Provider::Local,
            id: entry.id.to_owned(),
            name: entry.name.to_owned(),
            quantizations: entry
                .quantizations
                .iter()
                .map(|quantization| Quantization {
                    id: quantization.id,
                    name: quantization.name,
                    downloaded: std::iter::once(quantization.file)
                        .chain(entry.projector)
                        .all(|file| entry.repository.exists(file)),
                })
                .collect(),
            images: entry.projector.is_some(),
            // A translation model's template may write a general model's
            // markup, but the model was trained only to translate.
            tools: !entry.translator && entry.markup.is_some_and(|markup| markup.call.is_some()),
            reasoning: !entry.translator
                && entry
                    .markup
                    .is_some_and(|markup| markup.reasoning.is_some()),
            languages: entry.languages,
            context_window: None,
            max_output_tokens: None,
            pricing: None,
        })
        .collect::<Vec<_>>();
    let listed = futures::future::join_all(
        <Provider as strum::VariantArray>::VARIANTS
            .iter()
            .filter(|provider| **provider != Provider::Local)
            .map(|&provider| async move { (provider, hosted::models(provider).await) }),
    )
    .await;
    for (provider, listed) in listed {
        match listed {
            Ok(listed) => models.extend(listed),
            Err(error) => tracing::warn!("couldn't list {provider}'s models: {error:#}"),
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use rig_core::{
        completion::CompletionRequest,
        providers::{anthropic::wire::AnthropicConfig, openai::OpenAIConfig},
        wire::{Body, Encoded, Mode, Wire},
    };
    use serde_json::{Value, json};

    fn body(encoded: Encoded) -> Value {
        let Body::Bytes(bytes) = encoded.request.body() else {
            panic!("a JSON body");
        };
        serde_json::from_slice(bytes).unwrap()
    }

    /// rig rewrites a schema toward strict mode for OpenAI and Claude.
    /// Koharu's schemas are written that way, with every property required,
    /// no `oneOf`, nothing beside a `$ref`, and no numeric bounds, so the
    /// rewrite must leave them as they are; a rig release that rewrites more
    /// fails here.
    #[test]
    fn rig_sends_a_strict_schema_as_it_is() {
        let text = json!({
            "type": "object",
            "properties": {
                "first": {"type": "integer"},
                "last": {"type": "integer"},
                "text": {"type": "string"},
            },
            "required": ["first", "last", "text"],
            "additionalProperties": false,
        });
        let schema = json!({
            "type": "object",
            "properties": {"texts": {"type": "array", "items": text}},
            "required": ["texts"],
            "additionalProperties": false,
        });
        let request = || {
            CompletionRequest::new("hi")
                .output_schema(schemars::Schema::try_from(schema.clone()).unwrap())
        };
        let http = rig_reqwest::ReqwestClient::from(reqwest::Client::new());
        let openai = OpenAIConfig::new("key")
            .connect(http)
            .responses("gpt-5.5")
            .wire;
        let openai = body(openai.encode(request(), Mode::Unary).unwrap());
        assert_eq!(openai.pointer("/text/format/schema"), Some(&schema));
        let claude = AnthropicConfig::new("key").completion("claude-opus-5-5");
        let claude = body(claude.encode(request(), Mode::Unary).unwrap());
        assert_eq!(
            claude.pointer("/output_config/format/schema"),
            Some(&schema)
        );
    }
}
