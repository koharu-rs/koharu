//! Language models for Koharu, local and hosted. The README is the design.

mod agent;
mod chatgpt;
mod hosted;
mod local;

use std::path::Path;

use anyhow::{Context, Result, bail};
use rig_core::{
    completion::{CompletionRequest, Message},
    driver::DynModel,
    operation::Completion,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use agent::{Agent, AgentConfig, Event, Host, Invocation, Tool};
pub use chatgpt::{sign_in, sign_out};
pub use hosted::{Endpoint, Providers};
pub use local::{Sampling, Size};
/// rig, whose requests and streams every model here takes and gives.
pub use rig_core as rig;

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

impl Provider {
    /// How the provider's models keep to a JSON schema.
    fn json(self) -> Json {
        match self {
            Self::ChatGpt | Self::DeepSeek | Self::MiniMax => Json::Instructions,
            _ => Json::Schema,
        }
    }
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

impl Default for Selection {
    fn default() -> Self {
        Self {
            provider: Provider::Local,
            model: local::catalog::DEFAULT_MODEL.to_owned(),
            quantization: Some(local::catalog::DEFAULT_QUANTIZATION.to_owned()),
        }
    }
}

/// How a model keeps to a JSON schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Json {
    /// The schema goes with the request, and the provider holds the answer
    /// to it; the local engine holds it with a grammar.
    Schema,
    /// The schema goes in the instructions, with the provider's JSON mode
    /// where it has one.
    Instructions,
}

/// A model to send rig's requests to.
#[derive(Clone, Debug)]
pub struct Model {
    pub completion: DynModel<Completion>,
    provider: Provider,
}

impl Model {
    /// `request`, asking for an answer `schema` describes in the way this
    /// model keeps to one. The caller still checks the answer, as rig fails
    /// open on structured output.
    #[must_use]
    pub fn structured(
        &self,
        mut request: CompletionRequest,
        schema: schemars::Schema,
    ) -> CompletionRequest {
        match self.provider.json() {
            Json::Schema => request.output_schema = Some(schema),
            Json::Instructions => {
                // After the leading instructions, so a system prompt stays
                // first.
                let at = request
                    .chat_history
                    .iter()
                    .take_while(|message| matches!(***message, Message::System { .. }))
                    .count();
                let content = format!(
                    "Answer with JSON alone, as this JSON schema describes:\n{}",
                    schema.as_value()
                );
                request
                    .chat_history
                    .insert(at, std::sync::Arc::new(Message::System { content }));
            }
        }
        let params = match self.provider {
            // rig drops a DeepSeek schema, so DeepSeek is asked for JSON mode.
            Provider::DeepSeek => Some(json!({"response_format": {"type": "json_object"}})),
            // Only providers that keep to the schema serve the request.
            Provider::OpenRouter => Some(json!({"provider": {"require_parameters": true}})),
            _ => None,
        };
        if let Some(params) = params {
            request.additional_params = Some(match request.additional_params.take() {
                Some(held) => rig_core::json_utils::merge(held, params),
                None => params,
            });
        }
        request
    }
}

/// The model `selection` names, as a rig model; a local one loads with
/// `size`, which a hosted one doesn't use.
pub async fn model(selection: &Selection, size: Size) -> Result<Model> {
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
                entry.id.to_owned(),
                model,
                projector,
                size,
                entry.sampling.clone(),
            )
            .await?
        }
        provider => hosted::model(provider, &selection.model).await?,
    };
    Ok(Model {
        completion,
        provider: selection.provider,
    })
}

/// A GGUF outside the catalog, such as PaddleOCR-VL's, with its vision
/// projector, as a local model.
pub async fn load(model: &Path, projector: Option<&Path>, size: Size) -> Result<Model> {
    let id = model
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("{} has no UTF-8 name", model.display()))?
        .to_owned();
    let completion = local::load(
        id,
        model.to_owned(),
        projector.map(Path::to_owned),
        size,
        Sampling::default(),
    )
    .await?;
    Ok(Model {
        completion,
        provider: Provider::Local,
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
    pub json: Json,
    /// The languages a local model translates into, as BCP 47 tags; `None`
    /// for any.
    pub languages: Option<&'static [&'static str]>,
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
            tools: false,
            reasoning: entry.reasoning,
            json: Json::Schema,
            languages: entry.languages,
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

fn hosted_only(provider: Provider) -> Result<()> {
    if provider == Provider::Local {
        bail!("local models don't take tools, so the agent runs on a hosted model");
    }
    Ok(())
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
