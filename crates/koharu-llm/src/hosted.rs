//! Hosted models through rig's providers, on Koharu's HTTP client, and the
//! models each provider lists, with what rig's catalog says each can do.

mod http;

use anyhow::{Context, Result, bail};
use rig_core::{
    catalog::{Catalog, ModelFacts, ModelSpec, ReasoningSupport},
    driver::DynModel,
    operation::Completion,
    providers::{
        anthropic::wire::AnthropicConfig,
        gemini::GeminiConfig,
        openai::{
            OpenAIConfig,
            wire::{DEEPSEEK, MINIMAX, OPENROUTER},
        },
        registry::{ConnectOptions, Format, ProviderId},
        xai,
    },
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) use http::{Http, reqwest};

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

impl Provider {
    /// The vendor rig's registry and catalog know this provider as; the ones
    /// on the user's own server have none.
    pub(crate) fn vendor(self) -> Option<ProviderId> {
        let vendor = match self {
            Self::OpenAi => "openai",
            Self::ChatGpt => "chatgpt",
            Self::Claude => "anthropic",
            Self::Gemini => "gcp.gemini",
            Self::Xai => "xai",
            Self::DeepSeek => "deepseek",
            Self::MiniMax => "minimax",
            Self::OpenRouter => "openrouter",
            Self::Local | Self::LmStudio | Self::OpenAiCompatible => return None,
        };
        ProviderId::catalog(vendor)
    }
}

/// The provider's API key, which must be set for a provider that needs one.
fn key(provider: Provider) -> Result<String> {
    optional_key(provider)?.with_context(|| format!("{provider} has no API key"))
}

fn optional_key(provider: Provider) -> Result<Option<String>> {
    Ok(koharu_config::secrets::get(&provider.to_string())?
        .map(|key| key.expose_secret().to_owned()))
}

pub(crate) fn model(provider: Provider, id: &str) -> Result<DynModel<Completion>> {
    let Some(vendor) = provider.vendor() else {
        return own_server(provider, id);
    };
    let (key, http) = match provider {
        // The HTTP client puts the sign-in's credential on each request.
        Provider::ChatGpt => (String::new(), crate::chatgpt::http()?),
        _ => (key(provider)?, Http::default()),
    };
    Ok(Catalog::builtin().connect_with(&spec(vendor, id), options(provider, key, http))?)
}

/// Given a key, rig reads no environment variables.
fn options(provider: Provider, key: String, http: Http) -> ConnectOptions {
    let options = ConnectOptions::new().api_key(key).http(http);
    match provider {
        // rig's primary format for MiniMax is Anthropic's; Koharu's mapping of
        // how it keeps to a schema is for its OpenAI dialect.
        Provider::MiniMax => options.format(Format::OpenAi),
        _ => options,
    }
}

/// What the catalog says of `id`, or nothing certain of a model newer than it.
fn spec(vendor: ProviderId, id: &str) -> ModelSpec {
    match Catalog::builtin().get(vendor, id) {
        Some(found) => {
            // The catalog also answers for dated ids, which go out as asked.
            let mut spec = found.spec.clone();
            spec.id = id.to_owned();
            spec
        }
        None => unlisted(vendor, id),
    }
}

/// A model rig's catalog doesn't know: rig refuses nothing it might do, and its
/// limits are unknown.
fn unlisted(vendor: ProviderId, id: &str) -> ModelSpec {
    ModelSpec::new(vendor, id)
        .with_tools(true)
        .with_reasoning(ReasoningSupport::Unknown { default: None })
}

/// A provider on the user's own server, which rig's registry refuses as a
/// vendor it can't configure.
fn own_server(provider: Provider, id: &str) -> Result<DynModel<Completion>> {
    let providers: Providers = koharu_config::get()?;
    let vendor = ProviderId::declare(&provider.to_string())?;
    let mut model = openai(provider, &providers)?
        .connect(Http::default())
        .chat(id);
    model.wire = model.wire.with_facts(ModelFacts::new(unlisted(vendor, id)));
    Ok(model.erase())
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

impl ModelInfo {
    /// A hosted model, with what `spec`, its catalog entry if it has one, says
    /// it can do.
    pub(crate) fn hosted(
        provider: Provider,
        id: String,
        name: String,
        spec: Option<&ModelSpec>,
    ) -> Self {
        Self {
            provider,
            id,
            name,
            quantizations: Vec::new(),
            images: spec.is_some_and(|spec| spec.input.image),
            tools: spec.is_some_and(|spec| spec.tools),
            reasoning: spec.is_some_and(|spec| spec.reasoning.supported()),
            languages: None,
            context_window: spec.and_then(|spec| spec.context_window),
            max_output_tokens: spec.and_then(|spec| spec.max_output_tokens),
            pricing: spec.and_then(|spec| spec.pricing),
        }
    }
}

/// The models `provider` lists, or none when it needs a key and has none,
/// with what rig's catalog says each can do.
pub(crate) async fn models(provider: Provider) -> Result<Vec<ModelInfo>> {
    let providers: Providers = koharu_config::get()?;
    match provider {
        Provider::Local => return Ok(Vec::new()),
        Provider::ChatGpt => return crate::chatgpt::models(&reqwest()?).await,
        Provider::LmStudio => return lm_studio(&providers).await,
        Provider::OpenAiCompatible => {}
        _ if optional_key(provider)?.is_none() => return Ok(Vec::new()),
        _ => {}
    }
    let http = Http::default();
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
    let vendor = provider.vendor();
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
            let name = model.name.unwrap_or_else(|| model.id.clone());
            Some(ModelInfo::hosted(provider, model.id, name, spec))
        })
        .collect())
}

/// LM Studio's language models, from its own list, which says what each can
/// do: rig neither lists nor catalogs them.
async fn lm_studio(providers: &Providers) -> Result<Vec<ModelInfo>> {
    // https://lmstudio.ai/docs/developer/rest/list
    let mut request = reqwest()?.get(format!(
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
            let name = model
                .get("display_name")
                .and_then(Value::as_str)
                .map_or_else(|| id.clone(), str::to_owned);
            Some(ModelInfo {
                images: model
                    .pointer("/capabilities/vision")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                tools: model
                    .pointer("/capabilities/trained_for_tool_use")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                reasoning: model.pointer("/capabilities/reasoning").is_some(),
                ..ModelInfo::hosted(Provider::LmStudio, id, name, None)
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every vendor is one rig's registry can connect, in its OpenAI, Anthropic,
    /// or Gemini format, and a model the catalog doesn't list keeps its id.
    #[test]
    fn every_vendor_connects() {
        for provider in [
            Provider::OpenAi,
            Provider::ChatGpt,
            Provider::Claude,
            Provider::Gemini,
            Provider::Xai,
            Provider::DeepSeek,
            Provider::MiniMax,
            Provider::OpenRouter,
        ] {
            let vendor = provider.vendor().unwrap();
            let spec = spec(vendor, "a-model-from-next-year");
            let model = Catalog::builtin()
                .connect_with(&spec, options(provider, "key".to_owned(), Http::default()))
                .unwrap();
            assert_eq!(model.spec().unwrap().id, "a-model-from-next-year");
        }
    }

    #[test]
    fn a_listed_model_keeps_what_the_catalog_says() {
        let vendor = Provider::OpenAi.vendor().unwrap();
        let listed = Catalog::builtin()
            .iter()
            .find(|spec| spec.provider.vendor() == vendor.vendor() && spec.context_window.is_some())
            .unwrap();
        assert_eq!(
            spec(vendor, &listed.id).context_window,
            listed.context_window
        );
    }

    #[test]
    fn a_model_the_catalog_does_not_list_refuses_nothing() {
        let vendor = Provider::OpenAi.vendor().unwrap();
        let spec = spec(vendor, "a-model-from-next-year");
        assert!(spec.tools && spec.reasoning.supported());
        assert_eq!(spec.context_window, None);
    }
}
