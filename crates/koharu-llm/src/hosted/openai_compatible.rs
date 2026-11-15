//! A server that speaks OpenAI's format at the base URL the user gives, such
//! as a relay or a server they run, as Zed's OpenAI-compatible provider
//! reaches one: rig's OpenAI client on Chat Completions, which such servers
//! speak far more widely than OpenAI's Responses.

use anyhow::{Context as _, Result};
use rig_core::{
    catalog::{ModelFacts, ModelSpec},
    driver::DynModel,
    operation::Completion,
    providers::{
        openai::{OpenAI, OpenAIConfig},
        registry::ProviderId,
    },
};

use super::{Http, key, unlisted};

pub(crate) const PROVIDER_NAME: &str = "openai-compatible";

/// The server at `base_url`, which ends where OpenAI's `/v1` does, such as
/// `https://example.com/v1`, with the key stored for it if any.
fn client(base_url: String) -> Result<OpenAI> {
    Ok(OpenAIConfig::new(key(PROVIDER_NAME)?.unwrap_or_default())
        .with_base_url(base_url)
        .connect(Http::default()))
}

pub(super) fn connect(id: &str, base_url: Option<String>) -> Result<DynModel<Completion>> {
    let base_url = base_url.context("the OpenAI-compatible server has no base URL")?;
    let mut model = client(base_url)?.chat(id);
    let vendor = ProviderId::declare(PROVIDER_NAME)?;
    model.wire = model.wire.with_facts(ModelFacts::new(unlisted(vendor, id)));
    Ok(model.erase())
}

/// The server's `/models` list, or none while it has no base URL. The list
/// says nothing of what a model can do, so each is offered with its limits
/// unknown.
pub(super) async fn models(base_url: Option<String>) -> Result<Vec<ModelSpec>> {
    let Some(base_url) = base_url else {
        return Ok(Vec::new());
    };
    let vendor = ProviderId::declare(PROVIDER_NAME)?;
    Ok(client(base_url)?
        .list_models()
        .await?
        .into_iter()
        .map(|model| {
            let name = model.name.clone().unwrap_or_else(|| model.id.clone());
            unlisted(vendor, &model.id).with_display_name(name)
        })
        .collect())
}
