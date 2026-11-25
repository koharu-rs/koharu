//! Hosted models through rig's providers, on Koharu's HTTP client, and the
//! models each vendor lists, as rig's catalog describes them.

mod chatgpt;
mod grok;
mod http;
mod lm_studio;
pub(crate) mod openai_compatible;

use std::{collections::BTreeMap, num::NonZeroU32};

use anyhow::{Context, Result, bail, ensure};
use rig_core::{
    catalog::{Catalog, ModelFacts, ModelSpec, ReasoningSupport},
    driver::{DynModel, Model, Transport},
    http_client::DynHttpClient,
    operation::Completion,
    providers::{
        anthropic, deepseek, gemini, minimax, ollama,
        openai::{self, wire::OpenAiWire},
        openrouter,
        registry::{ProviderConfig, ProviderId},
        xai,
    },
    wire::Wire,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

pub(crate) use http::{Http, SignIn, reqwest};

/// The vendors Koharu lists models from, by rig's names.
pub const VENDORS: &[&str] = &[
    openai::PROVIDER_NAME,
    anthropic::PROVIDER_NAME,
    gemini::PROVIDER_NAME,
    xai::PROVIDER_NAME,
    deepseek::PROVIDER_NAME,
    minimax::PROVIDER_NAME,
    openrouter::PROVIDER_NAME,
    chatgpt::PROVIDER_NAME,
    grok::PROVIDER_NAME,
    ollama::PROVIDER_NAME,
    lm_studio::PROVIDER_NAME,
    openai_compatible::PROVIDER_NAME,
];

/// What a sign-in asks of the user: to sign in on a page, entering a code
/// there for a device code.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct SignInPrompt {
    pub url: String,
    /// Whether the browser was opened on the page already.
    pub opened: bool,
    pub code: Option<String>,
}

/// Signs in to `vendor`, ChatGPT or Grok, with the user's plan in place of
/// an API key, telling `prompt` what the user must do. The sign-in is kept
/// under the vendor's name, where a key would be.
pub async fn sign_in(
    vendor: &str,
    prompt: impl Fn(SignInPrompt) + Send + Sync + 'static,
) -> Result<()> {
    match vendor {
        chatgpt::PROVIDER_NAME => chatgpt::sign_in(prompt).await,
        grok::PROVIDER_NAME => grok::sign_in(prompt).await,
        _ => bail!("{vendor} takes an API key, not a sign-in"),
    }
}

/// Signs out of `vendor`, one [`sign_in`] signs in to: the vendor revokes
/// the sign-in, and Koharu deletes it.
pub async fn sign_out(vendor: &str) -> Result<()> {
    match vendor {
        chatgpt::PROVIDER_NAME => chatgpt::sign_out().await,
        grok::PROVIDER_NAME => grok::sign_out().await,
        _ => bail!("{vendor} takes an API key, not a sign-in"),
    }
}

/// `[providers]`: where a vendor's server is when it isn't where rig, or
/// Koharu for LM Studio, puts it, as Codex's `[model_providers.<id>]` holds a
/// `base_url`. Keyed by rig's vendor name.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
#[derive(specta::Type)]
pub struct Providers(pub BTreeMap<String, Endpoint>);

impl koharu_config::Section for Providers {
    const NAME: &'static str = "providers";
}

/// A vendor's server: where rig, or Koharu for LM Studio, puts it while
/// `base_url` is unset, as in an empty table.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct Endpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<url::Url>,
    /// The context window, in tokens, an OpenAI-compatible server runs its
    /// models with, which it doesn't say: llama.cpp's server runs every model
    /// it serves with one `--ctx-size`, and Zed's OpenAI-compatible provider
    /// is told each model's `max_tokens`. A conversation keeps inside it;
    /// unset, it grows until the server cuts it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<NonZeroU32>,
}

/// The base URL `[providers]` sets for `vendor`, if it sets one.
fn base_url(vendor: &str) -> Result<Option<String>> {
    let providers: Providers = koharu_config::get()?;
    Ok(providers
        .0
        .get(vendor)
        .and_then(|endpoint| endpoint.base_url.as_ref())
        .map(|url| url.as_str().trim_end_matches('/').to_owned()))
}

/// The context window `[providers]` gives `vendor`'s server, if it gives one.
fn context_window(vendor: &str) -> Result<Option<u32>> {
    let providers: Providers = koharu_config::get()?;
    Ok(providers
        .0
        .get(vendor)
        .and_then(|endpoint| endpoint.context_window)
        .map(NonZeroU32::get))
}

/// The vendor's API key, stored under its rig name.
fn key(vendor: &str) -> Result<Option<String>> {
    Ok(koharu_config::secrets::get(vendor)?.map(|key| key.expose_secret().to_owned()))
}

/// The id rig's catalog files `vendor`'s models under.
fn catalog_id(vendor: &str) -> Result<ProviderId> {
    ProviderId::catalog(vendor).with_context(|| format!("rig knows no vendor `{vendor}`"))
}

pub(crate) fn connect(vendor: &str, id: &str) -> Result<DynModel<Completion>> {
    let base_url = base_url(vendor)?;
    match vendor {
        grok::PROVIDER_NAME => return grok::connect(id, base_url),
        lm_studio::PROVIDER_NAME => return lm_studio::connect(id, base_url),
        openai_compatible::PROVIDER_NAME => {
            return openai_compatible::connect(id, base_url, context_window(vendor)?);
        }
        _ => {}
    }
    let filed = catalog_id(vendor)?;
    let (key, http) = match vendor {
        // The HTTP client puts the sign-in's credential on each request.
        chatgpt::PROVIDER_NAME => (String::new(), chatgpt::http()?),
        _ => {
            // Given a key, even an empty one, rig reads no environment
            // variable; an optional bearer then sends none.
            let key = key(vendor)?;
            ensure!(
                key.is_some() || !filed.requires_credential(),
                "{vendor} has no API key"
            );
            (key.unwrap_or_default(), Http::default())
        }
    };
    Ok(model(
        config(filed, key, base_url)?,
        spec(filed, id),
        DynHttpClient::new(http),
    ))
}

/// How rig reaches `filed`'s vendor, at `base_url` or its own.
fn config(filed: ProviderId, key: String, base_url: Option<String>) -> Result<ProviderConfig> {
    let config = filed
        .config(key)
        .with_context(|| format!("rig can't configure {}", filed.vendor()))?;
    Ok(match base_url {
        Some(base_url) => config.with_base_url(base_url),
        None => config,
    })
}

/// The model `spec` describes, as rig's catalog connects it, but that in the
/// Responses format its vendor is asked not to store replies, which OpenAI and
/// xAI otherwise do for 30 days.
fn model(config: ProviderConfig, spec: ModelSpec, http: DynHttpClient) -> DynModel<Completion> {
    let id = spec.id.clone();
    let facts = ModelFacts::new(spec).with_catalog(Catalog::builtin());
    match config {
        ProviderConfig::OpenAi(config) => {
            let mut model = config.connect(http).completion(id);
            model.wire = match model.wire.with_facts(facts) {
                OpenAiWire::Responses(wire) => OpenAiWire::Responses(wire.with_store(false)),
                chat => chat,
            };
            untimed(model)
        }
        ProviderConfig::Anthropic(config) => {
            let mut model = config.connect(http).completion(id);
            model.wire = model.wire.with_facts(facts);
            untimed(model)
        }
        ProviderConfig::Gemini(config) => {
            let mut model = config.connect(http).completion(id);
            model.wire = model.wire.with_facts(facts);
            untimed(model)
        }
    }
}

/// `model`, waiting for a reply until it ends or the user stops it. rig fails a
/// streamed one that says nothing for five minutes, which a server can take to
/// load a model, read a long prompt, or think.
fn untimed<W: Wire, T: Transport<W>>(model: Model<W, T>) -> DynModel<W::Op> {
    model.with_idle_timeout(None).erase()
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

/// A model rig's catalog doesn't know: rig refuses nothing it might do, such
/// as calling tools or reading an image, which its provider refuses itself if
/// it can't, and its limits are unknown.
fn unlisted(vendor: ProviderId, id: &str) -> ModelSpec {
    let mut spec = ModelSpec::new(vendor, id)
        .with_tools(true)
        .with_reasoning(ReasoningSupport::Unknown { default: None });
    spec.input.image = true;
    spec
}

/// The models `vendor` lists, or none when it needs a key and has none, as
/// rig's catalog describes them.
pub(crate) async fn models(vendor: &str) -> Result<Vec<ModelSpec>> {
    match vendor {
        chatgpt::PROVIDER_NAME => return chatgpt::models(&reqwest()?).await,
        grok::PROVIDER_NAME => return grok::models(base_url(vendor)?).await,
        lm_studio::PROVIDER_NAME => return lm_studio::models(base_url(vendor)?).await,
        openai_compatible::PROVIDER_NAME => {
            return openai_compatible::models(base_url(vendor)?, context_window(vendor)?).await;
        }
        _ => {}
    }
    let filed = catalog_id(vendor)?;
    let key = key(vendor)?;
    if key.is_none() && filed.requires_credential() {
        return Ok(Vec::new());
    }
    let http = Http::default();
    let listed = match config(filed, key.unwrap_or_default(), base_url(vendor)?)? {
        ProviderConfig::OpenAi(config) => config.connect(http).list_models().await?,
        ProviderConfig::Anthropic(config) => config.connect(http).list_models().await?,
        ProviderConfig::Gemini(config) => config.connect(http).list_models().await?,
    };
    let catalog = Catalog::builtin();
    Ok(listed
        .into_iter()
        .filter_map(|model| {
            let found = catalog
                .get(filed, &model.id)
                .map(|found| found.spec.clone());
            // A model the catalog knows as neither calling tools nor keeping
            // to a schema, such as an embedding or image model, writes no text
            // Koharu reads.
            if found
                .as_ref()
                .is_some_and(|spec| !spec.tools && !spec.structured_output)
            {
                return None;
            }
            Some(found.unwrap_or_else(|| {
                let name = model.name.clone().unwrap_or_else(|| model.id.clone());
                unlisted(filed, &model.id).with_display_name(name)
            }))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every vendor rig configures connects in its OpenAI, Anthropic, or
    /// Gemini format, and a model the catalog doesn't list keeps its id.
    #[test]
    fn every_vendor_connects() {
        for vendor in VENDORS.iter().filter(|vendor| {
            ![
                grok::PROVIDER_NAME,
                lm_studio::PROVIDER_NAME,
                openai_compatible::PROVIDER_NAME,
            ]
            .contains(vendor)
        }) {
            let filed = catalog_id(vendor).unwrap();
            let model = model(
                config(filed, "key".to_owned(), None).unwrap(),
                spec(filed, "a-model-from-next-year"),
                DynHttpClient::new(Http::default()),
            );
            assert_eq!(model.spec().unwrap().id, "a-model-from-next-year");
        }
    }

    /// A model newer than rig's catalog, such as a new Gemini, isn't refused
    /// a page image before its provider has a say.
    #[test]
    fn a_model_the_catalog_does_not_list_may_read_images() {
        let vendor = catalog_id(gemini::PROVIDER_NAME).unwrap();
        assert!(spec(vendor, "a-model-from-next-year").input.image);
    }

    #[test]
    fn a_listed_model_keeps_what_the_catalog_says() {
        let vendor = catalog_id(openai::PROVIDER_NAME).unwrap();
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
        let vendor = catalog_id(openai::PROVIDER_NAME).unwrap();
        let spec = spec(vendor, "a-model-from-next-year");
        assert!(spec.tools && spec.reasoning.supported());
        assert_eq!(spec.context_window, None);
    }
}
