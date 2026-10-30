//! Hosted models through rig's providers, on Koharu's HTTP client, and the
//! models each provider lists, with what rig's catalog says each can do.

use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use rig_core::{
    catalog::Catalog,
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
        registry::ProviderId,
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
        Provider::ChatGpt => crate::chatgpt::model(id, http).await?,
        Provider::Claude => AnthropicConfig::new(key(provider)?)
            .connect(http)
            .completion(id)
            .erase(),
        Provider::Gemini => GeminiConfig::new(key(provider)?)
            .connect(http)
            .completion(id)
            .erase(),
        Provider::OpenAi => openai(provider, &providers)?
            .connect(http)
            .responses(id)
            .erase(),
        Provider::Xai => openai(provider, &providers)?
            .connect(http)
            .completion(id)
            .erase(),
        Provider::DeepSeek
        | Provider::MiniMax
        | Provider::OpenRouter
        | Provider::LmStudio
        | Provider::OpenAiCompatible => {
            openai(provider, &providers)?.connect(http).chat(id).erase()
        }
    })
}

/// How rig reaches a provider that speaks OpenAI's dialect.
fn openai(provider: Provider, providers: &Providers) -> Result<OpenAIConfig> {
    Ok(match provider {
        Provider::OpenAi => OpenAIConfig::new(key(provider)?),
        Provider::Xai => OpenAIConfig::with_key(&xai::DIALECT, key(provider)?),
        Provider::DeepSeek => OpenAIConfig::with_key(&DEEPSEEK, key(provider)?),
        Provider::MiniMax => OpenAIConfig::with_key(&MINIMAX, key(provider)?),
        Provider::OpenRouter => OpenAIConfig::with_key(&OPENROUTER, key(provider)?),
        Provider::LmStudio => OpenAIConfig::new(optional_key(provider)?.unwrap_or_default())
            .with_base_url(format!("{}/v1", providers.lm_studio.base_url(LM_STUDIO))),
        Provider::OpenAiCompatible => {
            OpenAIConfig::new(optional_key(provider)?.unwrap_or_default())
                .with_base_url(providers.openai_compatible.base_url(OPENAI_COMPATIBLE))
        }
        Provider::Local | Provider::ChatGpt | Provider::Claude | Provider::Gemini => {
            bail!("{provider} doesn't speak OpenAI's dialect")
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

/// The models `provider` lists, or none when it needs a key and has none,
/// with what rig's catalog says each can do.
pub(crate) async fn models(provider: Provider) -> Result<Vec<ModelInfo>> {
    let providers: Providers = koharu_config::get()?;
    match provider {
        Provider::Local => return Ok(Vec::new()),
        Provider::ChatGpt => return crate::chatgpt::models(&koharu_runtime::http_client()?).await,
        Provider::LmStudio => return lm_studio(&providers).await,
        Provider::OpenAiCompatible => {}
        _ if optional_key(provider)?.is_none() => return Ok(Vec::new()),
        _ => {}
    }
    let http = Http::new()?;
    let listed = match provider {
        Provider::Claude => {
            AnthropicConfig::new(key(provider)?)
                .connect(http)
                .list_models()
                .await?
        }
        Provider::Gemini => {
            GeminiConfig::new(key(provider)?)
                .connect(http)
                .list_models()
                .await?
        }
        _ => {
            openai(provider, &providers)?
                .connect(http)
                .list_models()
                .await?
        }
    };
    // The catalog's vendor for the provider's models; an OpenAI-compatible
    // server's models have none.
    let vendor = match provider {
        Provider::OpenAi => Some("openai"),
        Provider::Claude => Some("anthropic"),
        Provider::Gemini => Some("gcp.gemini"),
        Provider::Xai => Some("xai"),
        Provider::DeepSeek => Some("deepseek"),
        Provider::MiniMax => Some("minimax"),
        Provider::OpenRouter => Some("openrouter"),
        _ => None,
    }
    .map(|vendor| {
        ProviderId::catalog(vendor).with_context(|| format!("rig's catalog has no {vendor}"))
    })
    .transpose()?;
    let catalog = Catalog::builtin();
    Ok(listed
        .into_iter()
        .filter_map(|model| {
            let spec = vendor
                .and_then(|vendor| catalog.get(vendor, &model.id))
                .map(|resolved| resolved.spec);
            // A model the catalog knows as neither calling tools nor keeping
            // to a schema, such as an embedding or image model, writes no
            // text Koharu reads.
            if spec.is_some_and(|spec| !spec.tools && !spec.structured_output) {
                return None;
            }
            Some(ModelInfo {
                provider,
                name: model.name.unwrap_or_else(|| model.id.clone()),
                id: model.id,
                quantizations: Vec::new(),
                images: spec.is_some_and(|spec| spec.input.image),
                tools: spec.is_some_and(|spec| spec.tools),
                reasoning: spec.is_some_and(|spec| spec.reasoning.supported()),
                json: provider.json(),
                languages: None,
            })
        })
        .collect())
}

/// LM Studio's language models, from its own list, which says what each can
/// do: rig neither lists nor catalogs them.
async fn lm_studio(providers: &Providers) -> Result<Vec<ModelInfo>> {
    // https://lmstudio.ai/docs/developer/rest/list
    let mut request = koharu_runtime::http_client()?.get(format!(
        "{}/api/v1/models",
        providers.lm_studio.base_url(LM_STUDIO)
    ));
    if let Some(key) = optional_key(Provider::LmStudio)? {
        request = request.bearer_auth(key);
    }
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("the model list answered {status}: {body}");
    }
    let body: Value = response.json().await?;
    let Some(models) = body.get("models").and_then(Value::as_array) else {
        bail!("the model list has no `models` array");
    };
    Ok(models
        .iter()
        .filter(|model| {
            matches!(
                model.get("type").and_then(Value::as_str),
                Some("llm" | "vlm")
            )
        })
        .filter_map(|model| {
            let id = model.get("key")?.as_str()?.to_owned();
            Some(ModelInfo {
                provider: Provider::LmStudio,
                name: model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map_or_else(|| id.clone(), str::to_owned),
                id,
                quantizations: Vec::new(),
                images: model
                    .pointer("/capabilities/vision")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                tools: model
                    .pointer("/capabilities/trained_for_tool_use")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                reasoning: model.pointer("/capabilities/reasoning").is_some(),
                json: Provider::LmStudio.json(),
                languages: None,
            })
        })
        .collect())
}
