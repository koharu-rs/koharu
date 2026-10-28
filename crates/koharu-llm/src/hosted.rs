//! Hosted models through rig's providers, on Koharu's HTTP client, and the
//! models each provider lists, with what each can do.

use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use rig_core::{
    driver::DynModel,
    error::ProviderError,
    http_client::{self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse},
    operation::Completion,
    providers::{
        anthropic::wire::AnthropicConfig,
        gemini::GeminiConfig,
        openai::{
            OpenAIConfig,
            wire::{DEEPSEEK, MINIMAX, OPENROUTER},
        },
        xai,
    },
};
use rig_reqwest::ReqwestClient;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ModelInfo, Provider};

/// `[providers]`: where the providers that run on the user's own server
/// are. A provider without an entry is at its default.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Providers {
    #[serde(rename = "lm-studio")]
    pub lm_studio: Endpoint,
    #[serde(rename = "openai-compatible")]
    pub openai_compatible: Endpoint,
}

impl koharu_config::Section for Providers {
    const NAME: &'static str = "providers";
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Endpoint {
    pub base_url: Option<url::Url>,
}

impl Endpoint {
    fn base_url(&self, default: &str) -> String {
        self.base_url
            .as_ref()
            .map_or(default, url::Url::as_str)
            .trim_end_matches('/')
            .to_owned()
    }
}

/// LM Studio's own server.
const LM_STUDIO: &str = "http://localhost:1234";
/// Ollama's OpenAI-compatible endpoint.
const OPENAI_COMPATIBLE: &str = "http://localhost:11434/v1";

/// The provider's API key, which must be set for a provider that needs one.
fn key(provider: Provider) -> Result<String> {
    optional_key(provider)?.with_context(|| format!("{provider} has no API key"))
}

fn optional_key(provider: Provider) -> Result<Option<String>> {
    Ok(koharu_config::secret(&provider.to_string())?.map(|key| key.expose_secret().to_owned()))
}

pub(crate) async fn model(provider: Provider, id: &str) -> Result<DynModel<Completion>> {
    let http = Http::new()?;
    let providers: Providers = koharu_config::get()?;
    Ok(match provider {
        Provider::Local => bail!("a local model isn't hosted"),
        Provider::OpenAi => OpenAIConfig::new(key(provider)?)
            .connect(http)
            .responses(id)
            .erase(),
        Provider::ChatGpt => crate::chatgpt::model(id, http).await?,
        Provider::Claude => AnthropicConfig::new(key(provider)?)
            .connect(http)
            .completion(id)
            .erase(),
        Provider::Gemini => GeminiConfig::new(key(provider)?)
            .connect(http)
            .completion(id)
            .erase(),
        Provider::Xai => OpenAIConfig::with_key(&xai::DIALECT, key(provider)?)
            .connect(http)
            .completion(id)
            .erase(),
        Provider::DeepSeek => OpenAIConfig::with_key(&DEEPSEEK, key(provider)?)
            .connect(http)
            .chat(id)
            .erase(),
        Provider::MiniMax => OpenAIConfig::with_key(&MINIMAX, key(provider)?)
            .connect(http)
            .chat(id)
            .erase(),
        Provider::OpenRouter => OpenAIConfig::with_key(&OPENROUTER, key(provider)?)
            .connect(http)
            .chat(id)
            .erase(),
        Provider::LmStudio => OpenAIConfig::new(optional_key(provider)?.unwrap_or_default())
            .with_base_url(format!("{}/v1", providers.lm_studio.base_url(LM_STUDIO)))
            .connect(http)
            .chat(id)
            .erase(),
        Provider::OpenAiCompatible => {
            OpenAIConfig::new(optional_key(provider)?.unwrap_or_default())
                .with_base_url(providers.openai_compatible.base_url(OPENAI_COMPATIBLE))
                .connect(http)
                .chat(id)
                .erase()
        }
    })
}

/// Koharu's HTTP client for rig. A 429 or 5xx reply is sent again, up to
/// three times, after the wait rig reads from it, or a second, then two,
/// then four when it names none; rig itself retries nothing.
#[derive(Clone)]
pub(crate) struct Http(ReqwestClient);

const RETRIES: u32 = 3;

impl Http {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self(ReqwestClient::from(koharu_runtime::http_client()?)))
    }
}

/// How long to wait before sending again after `error`, or `None` when it
/// isn't worth sending again.
fn wait(error: &http_client::Error, attempt: u32) -> Option<Duration> {
    let http_client::Error::InvalidStatusCodeWithDetails {
        status,
        body,
        headers,
    } = error
    else {
        return None;
    };
    if attempt >= RETRIES || !(status.as_u16() == 429 || status.is_server_error()) {
        return None;
    }
    let report = ProviderError::from(http_client::Error::InvalidStatusCodeWithDetails {
        status: *status,
        body: body.clone(),
        headers: headers.clone(),
    })
    .report();
    Some(
        report
            .retry_after(SystemTime::now())
            .unwrap_or(Duration::from_secs(1 << attempt)),
    )
}

impl HttpClientExt for Http {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let client = self.0.clone();
        let (parts, body) = request.into_parts();
        let body = body.into();
        async move {
            let mut attempt = 0;
            loop {
                let request = http::Request::from_parts(parts.clone(), body.clone());
                match client.send(request).await {
                    Err(error) => match wait(&error, attempt) {
                        Some(wait) => tokio::time::sleep(wait).await,
                        None => return Err(error),
                    },
                    reply => return reply,
                }
                attempt += 1;
            }
        }
    }

    fn send_multipart<U>(
        &self,
        request: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        self.0.send_multipart(request)
    }

    fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let client = self.0.clone();
        let (parts, body) = request.into_parts();
        let body: Bytes = body.into();
        async move {
            let mut attempt = 0;
            loop {
                let request = http::Request::from_parts(parts.clone(), body.clone());
                match client.send_streaming(request).await {
                    Err(error) => match wait(&error, attempt) {
                        Some(wait) => tokio::time::sleep(wait).await,
                        None => return Err(error),
                    },
                    reply => return reply,
                }
                attempt += 1;
            }
        }
    }
}

/// The models `provider` lists, or none when it has no key.
pub(crate) async fn models(provider: Provider) -> Result<Vec<ModelInfo>> {
    let client = koharu_runtime::http_client()?;
    let providers: Providers = koharu_config::get()?;
    let get = |url: &str| client.get(url);
    let info =
        |id: String, name: Option<String>, images: bool, tools: bool, reasoning: bool| ModelInfo {
            provider,
            name: name.unwrap_or_else(|| id.clone()),
            id,
            quantizations: Vec::new(),
            images,
            tools,
            reasoning,
            json: provider.json(),
            languages: None,
        };
    let key = match provider {
        Provider::Local => return Ok(Vec::new()),
        Provider::ChatGpt => return crate::chatgpt::models(&client).await,
        Provider::LmStudio | Provider::OpenAiCompatible => optional_key(provider)?,
        _ => match optional_key(provider)? {
            Some(key) => Some(key),
            None => return Ok(Vec::new()),
        },
    };
    let bearer = |request: reqwest::RequestBuilder| match &key {
        Some(key) => request.bearer_auth(key),
        None => request,
    };
    Ok(match provider {
        // https://developers.openai.com/api/reference/resources/models/methods/list
        // The list says nothing of capabilities, so they come from the
        // model's family.
        Provider::OpenAi => list(bearer(get("https://api.openai.com/v1/models")), "data")
            .await?
            .into_iter()
            .filter_map(|model| {
                let id = string(&model, "id")?;
                openai_family(&id).then(|| info(id, None, true, true, true))
            })
            .collect(),
        // https://platform.claude.com/docs/en/api/models/list
        Provider::Claude => list(
            get("https://api.anthropic.com/v1/models?limit=1000")
                .header("x-api-key", key.as_deref().unwrap_or_default())
                .header("anthropic-version", "2023-06-01"),
            "data",
        )
        .await?
        .into_iter()
        .filter_map(|model| {
            let supported = |pointer: &str| {
                model
                    .pointer(pointer)
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            };
            let images = supported("/capabilities/image_input/supported");
            let reasoning = supported("/capabilities/thinking/types/adaptive/supported");
            Some(info(
                string(&model, "id")?,
                string(&model, "display_name"),
                images,
                true,
                reasoning,
            ))
        })
        .collect(),
        // https://ai.google.dev/api/models
        Provider::Gemini => list(
            get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000")
                .header("x-goog-api-key", key.as_deref().unwrap_or_default()),
            "models",
        )
        .await?
        .into_iter()
        .filter_map(|model| {
            let generates = strings(&model, "supportedGenerationMethods")
                .iter()
                .any(|method| method == "generateContent");
            let id = string(&model, "name")?.strip_prefix("models/")?.to_owned();
            let thinking = model
                .get("thinking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (generates && gemini_family(&id))
                .then(|| info(id, string(&model, "displayName"), true, true, thinking))
        })
        .collect(),
        // https://docs.x.ai/developers/rest-api-reference/inference/models
        Provider::Xai => list(bearer(get("https://api.x.ai/v1/language-models")), "models")
            .await?
            .into_iter()
            .filter_map(|model| {
                let images = strings(&model, "input_modalities")
                    .iter()
                    .any(|m| m == "image");
                Some(info(string(&model, "id")?, None, images, true, true))
            })
            .collect(),
        // https://api-docs.deepseek.com/api/list-models
        Provider::DeepSeek => list(bearer(get("https://api.deepseek.com/models")), "data")
            .await?
            .into_iter()
            .filter_map(|model| {
                let images = strings(&model, "input_modalities")
                    .iter()
                    .any(|m| m == "image");
                Some(info(string(&model, "id")?, None, images, true, true))
            })
            .collect(),
        // https://platform.minimax.io/docs/api-reference/models/openai/list-models
        Provider::MiniMax => list(bearer(get("https://api.minimax.io/v1/models")), "data")
            .await?
            .into_iter()
            .filter_map(|model| Some(info(string(&model, "id")?, None, false, true, true)))
            .collect(),
        // https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties
        Provider::OpenRouter => list(bearer(get("https://openrouter.ai/api/v1/models")), "data")
            .await?
            .into_iter()
            .filter_map(|model| {
                let parameters = strings(&model, "supported_parameters");
                let images = model
                    .pointer("/architecture/input_modalities")
                    .and_then(Value::as_array)
                    .is_some_and(|modalities| modalities.iter().any(|m| m == "image"));
                let takes = |parameter: &str| parameters.iter().any(|p| p == parameter);
                Some(info(
                    string(&model, "id")?,
                    string(&model, "name"),
                    images,
                    takes("tools"),
                    takes("reasoning"),
                ))
            })
            .collect(),
        // https://lmstudio.ai/docs/developer/rest/list
        Provider::LmStudio => list(
            bearer(get(&format!(
                "{}/api/v1/models",
                providers.lm_studio.base_url(LM_STUDIO)
            ))),
            "models",
        )
        .await?
        .into_iter()
        .filter(|model| {
            matches!(
                model.get("type").and_then(Value::as_str),
                Some("llm" | "vlm")
            )
        })
        .filter_map(|model| {
            let images = model
                .pointer("/capabilities/vision")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let tools = model
                .pointer("/capabilities/trained_for_tool_use")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reasoning = model.pointer("/capabilities/reasoning").is_some();
            Some(info(
                string(&model, "key")?,
                string(&model, "display_name"),
                images,
                tools,
                reasoning,
            ))
        })
        .collect(),
        Provider::OpenAiCompatible => list(
            bearer(get(&format!(
                "{}/models",
                providers.openai_compatible.base_url(OPENAI_COMPATIBLE)
            ))),
            "data",
        )
        .await?
        .into_iter()
        .filter_map(|model| Some(info(string(&model, "id")?, None, true, true, true)))
        .collect(),
        Provider::Local | Provider::ChatGpt => unreachable!("answered above"),
    })
}

/// The array at `field` of the JSON `request` answers.
async fn list(request: reqwest::RequestBuilder, field: &str) -> Result<Vec<Value>> {
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("the model list answered {status}: {body}");
    }
    let mut body: Value = response.json().await?;
    match body.get_mut(field).map(Value::take) {
        Some(Value::Array(models)) => Ok(models),
        _ => bail!("the model list has no `{field}` array"),
    }
}

fn string(value: &Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(str::to_owned)
}

fn strings(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// OpenAI's text models that take images and a schema, fine-tuned ones
/// included: GPT-5, GPT-4.1, GPT-4o, and o3, less their audio, image, search,
/// and other variants.
fn openai_family(id: &str) -> bool {
    let id = id
        .strip_prefix("ft:")
        .and_then(|id| id.split(':').next())
        .unwrap_or(id)
        .to_ascii_lowercase();
    let family = ["gpt-5", "gpt-4.1", "gpt-4o", "o3-"]
        .iter()
        .any(|family| id.starts_with(family))
        || id == "o3";
    let variant = [
        "-audio",
        "-codex",
        "-deep-research",
        "-image",
        "-pro",
        "-realtime",
        "-search",
        "-transcribe",
        "-tts",
    ]
    .iter()
    .any(|variant| id.contains(variant));
    family && !variant
}

/// Gemini's text models, less its image, audio, video, and agent ones.
fn gemini_family(id: &str) -> bool {
    ![
        "antigravity",
        "computer-use",
        "deep-research",
        "embedding",
        "image",
        "imagen",
        "live",
        "lyria",
        "native-audio",
        "omni",
        "robotics",
        "tts",
        "veo",
    ]
    .iter()
    .any(|variant| id.contains(variant))
}
