//! LM Studio, which rig has no provider for: its OpenAI-compatible server
//! through rig's OpenAI client, and its own model list.

use anyhow::{Result, bail};
use rig_core::{
    catalog::{ModelFacts, ModelSpec, ReasoningSupport},
    driver::DynModel,
    operation::Completion,
    providers::{openai::OpenAIConfig, registry::ProviderId},
};
use serde_json::Value;

use super::{Http, key, reqwest, unlisted};

/// LM Studio, which rig has no dialect for, so Koharu declares it.
pub(super) const PROVIDER_NAME: &str = "lmstudio";
/// LM Studio's own server.
const URL: &str = "http://localhost:1234";

/// A model on LM Studio's server, which rig's registry can't configure.
pub(super) fn connect(id: &str, base_url: Option<String>) -> Result<DynModel<Completion>> {
    let base_url = base_url.as_deref().unwrap_or(URL);
    let mut model = OpenAIConfig::new(key(PROVIDER_NAME)?.unwrap_or_default())
        .with_base_url(format!("{base_url}/v1"))
        .connect(Http::default())
        .chat(id);
    let vendor = ProviderId::declare(PROVIDER_NAME)?;
    model.wire = model.wire.with_facts(ModelFacts::new(unlisted(vendor, id)));
    Ok(model.erase())
}

/// LM Studio's language models, from its own list, which says what each can
/// do: rig neither lists nor catalogs them.
pub(super) async fn models(base_url: Option<String>) -> Result<Vec<ModelSpec>> {
    // https://lmstudio.ai/docs/developer/rest/list
    let base_url = base_url.as_deref().unwrap_or(URL);
    let mut request = reqwest()?.get(format!("{base_url}/api/v1/models"));
    if let Some(key) = key(PROVIDER_NAME)? {
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
    let vendor = ProviderId::declare(PROVIDER_NAME)?;
    Ok(models
        .iter()
        .filter(|model| {
            matches!(
                model.get("type").and_then(Value::as_str),
                Some("llm" | "vlm")
            )
        })
        .filter_map(|model| {
            let id = model.get("key")?.as_str()?;
            let yes = |capability: &str| model.pointer(capability) == Some(&Value::Bool(true));
            let mut spec =
                ModelSpec::new(vendor, id).with_tools(yes("/capabilities/trained_for_tool_use"));
            if let Some(name) = model.get("display_name").and_then(Value::as_str) {
                spec = spec.with_display_name(name);
            }
            if model.pointer("/capabilities/reasoning").is_some() {
                spec = spec.with_reasoning(ReasoningSupport::Unknown { default: None });
            }
            spec.input.image = yes("/capabilities/vision");
            Some(spec)
        })
        .collect())
}
