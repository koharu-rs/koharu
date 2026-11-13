//! Hosted models through rig's providers, on Koharu's HTTP client, and the
//! models each vendor lists, as rig's catalog describes them.

mod http;
mod lm_studio;

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use rig_core::{
    catalog::{Catalog, ModelSpec, ReasoningSupport},
    driver::DynModel,
    operation::Completion,
    providers::{
        anthropic, chatgpt, deepseek, gemini, minimax, ollama, openai, openrouter,
        registry::{ConnectOptions, Format, ProviderConfig, ProviderId},
        xai,
    },
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

pub(crate) use http::{Http, reqwest};

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
    ollama::PROVIDER_NAME,
    lm_studio::PROVIDER_NAME,
];

/// `[providers]`: where a vendor's server is when it isn't where rig, or
/// Koharu for LM Studio, puts it, as Codex's `[model_providers.<id>]` holds a
/// `base_url`. Keyed by rig's vendor name.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Providers(pub BTreeMap<String, Endpoint>);

impl koharu_config::Section for Providers {
    const NAME: &'static str = "providers";
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Endpoint {
    pub base_url: url::Url,
}

/// The base URL `[providers]` sets for `vendor`, if it sets one.
fn base_url(vendor: &str) -> Result<Option<String>> {
    let providers: Providers = koharu_config::get()?;
    Ok(providers
        .0
        .get(vendor)
        .map(|endpoint| endpoint.base_url.as_str().trim_end_matches('/').to_owned()))
}

/// The vendor's API key, stored under its rig name.
fn key(vendor: &str) -> Result<Option<String>> {
    Ok(koharu_config::secrets::get(vendor)?.map(|key| key.expose_secret().to_owned()))
}

/// The id rig's catalog files `vendor`'s models under.
fn catalog_id(vendor: &str) -> Result<ProviderId> {
    ProviderId::catalog(vendor).with_context(|| format!("rig knows no vendor `{vendor}`"))
}

/// The family Koharu reaches `vendor` in when it isn't the catalog's:
/// MiniMax's OpenAI dialect, which Koharu's prompted answers are for.
fn family(vendor: &str) -> Option<Format> {
    (vendor == minimax::PROVIDER_NAME).then_some(Format::OpenAi)
}

pub(crate) fn connect(vendor: &str, id: &str) -> Result<DynModel<Completion>> {
    let base_url = base_url(vendor)?;
    if vendor == lm_studio::PROVIDER_NAME {
        return lm_studio::connect(id, base_url);
    }
    let selection = catalog_id(vendor)?;
    let options = match vendor {
        // The HTTP client puts the sign-in's credential on each request.
        chatgpt::PROVIDER_NAME => ConnectOptions::new()
            .api_key(String::new())
            .http(crate::chatgpt::http()?),
        _ => {
            // Given a key, even an empty one, rig reads no environment
            // variable; an optional bearer then sends none.
            let key = key(vendor)?;
            ensure!(
                key.is_some() || !selection.requires_credential(),
                "{vendor} has no API key"
            );
            ConnectOptions::new()
                .api_key(key.unwrap_or_default())
                .http(Http::default())
        }
    };
    let options = match family(vendor) {
        Some(format) => options.format(format),
        None => options,
    };
    let options = match base_url {
        Some(base_url) => options.base_url(base_url),
        None => options,
    };
    Ok(Catalog::builtin().connect_with(&spec(selection, id), options)?)
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

/// The models `vendor` lists, or none when it needs a key and has none, as
/// rig's catalog describes them.
pub(crate) async fn models(vendor: &str) -> Result<Vec<ModelSpec>> {
    match vendor {
        chatgpt::PROVIDER_NAME => return crate::chatgpt::models(&reqwest()?).await,
        lm_studio::PROVIDER_NAME => return lm_studio::models(base_url(vendor)?).await,
        _ => {}
    }
    let filed = catalog_id(vendor)?;
    let key = key(vendor)?;
    if key.is_none() && filed.requires_credential() {
        return Ok(Vec::new());
    }
    let config = match family(vendor) {
        Some(format) => ProviderId::new(vendor, format),
        None => Some(filed),
    }
    .and_then(|selection| selection.config(key.unwrap_or_default()))
    .with_context(|| format!("rig can't configure {vendor}"))?;
    let config = match base_url(vendor)? {
        Some(base_url) => config.with_base_url(base_url),
        None => config,
    };
    let http = Http::default();
    let listed = match config {
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
        for vendor in VENDORS
            .iter()
            .filter(|vendor| **vendor != lm_studio::PROVIDER_NAME)
        {
            let spec = spec(catalog_id(vendor).unwrap(), "a-model-from-next-year");
            let mut options = ConnectOptions::new().api_key("key").http(Http::default());
            if let Some(format) = family(vendor) {
                options = options.format(format);
            }
            let model = Catalog::builtin().connect_with(&spec, options).unwrap();
            assert_eq!(model.spec().unwrap().id, "a-model-from-next-year");
        }
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
