//! Language models for Koharu, local and hosted, named, described, and
//! connected with rig's own types: a `vendor/model` reference, its
//! `ModelSpec`, and a `DynModel`.

mod agent;
mod chatgpt;
mod context;
mod hosted;
mod local;
mod model;

use anyhow::{Context as _, Result};
use rig_core::{catalog::ModelSpec, driver::DynModel, operation::Completion};

pub use agent::{Agent, AgentConfig, Event, Host, Invocation, Tool};
pub use chatgpt::{sign_in, sign_out};
pub use context::Context;
pub use hosted::{Endpoint, Providers, VENDORS};
pub use local::{Local, Quantization, Sampling, Size, local};
pub use model::answers;
pub use rig_agent::agent::AgentBuilder;
/// rig, whose requests and streams every model here takes and gives.
pub use rig_core as rig;
pub use rig_memory::{CompactReason, SummaryPrompts};

/// The vendor of the models Koharu runs itself, on llama.cpp.
pub const KOHARU: &str = "koharu";

/// The `details.kind` of a model trained only to translate, as Hugging Face's
/// pipeline tag names one. rig keeps models.dev's kinds there, such as
/// `image` and `embedding`.
pub const TRANSLATION: &str = "translation";

/// The model `reference` names, `vendor/model` as rig's `ModelSpec::reference`
/// writes it, its files fetched first for one of Koharu's own. One of those
/// may name a quantization after a colon, as Ollama's tags do
/// (`koharu/index-translate-2b:Q8_0`), and otherwise loads its first; it loads
/// with `size`, which a hosted model doesn't use.
pub async fn connect(reference: &str, size: Size) -> Result<DynModel<Completion>> {
    let (vendor, id) = reference
        .split_once('/')
        .with_context(|| format!("`{reference}` names no model: expected `vendor/model`"))?;
    match vendor {
        KOHARU => local::connect(id, size).await,
        _ => hosted::connect(vendor, id),
    }
}

/// Every model the user can pick: Koharu's own, and each vendor's that has a
/// key or a sign-in, with what rig's catalog says each can do. A vendor whose
/// list fails is left out, with the failure logged.
pub async fn models() -> Vec<ModelSpec> {
    let mut models = local::specs();
    let listed = futures::future::join_all(
        VENDORS
            .iter()
            .map(|&vendor| async move { (vendor, hosted::models(vendor).await) }),
    )
    .await;
    for (vendor, listed) in listed {
        match listed {
            Ok(listed) => models.extend(listed),
            Err(error) => tracing::warn!("couldn't list {vendor}'s models: {error:#}"),
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
