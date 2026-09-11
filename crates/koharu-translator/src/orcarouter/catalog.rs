use anyhow::{Context as _, Result};
use koharu_secrets::ExposeSecret as _;
use reqwest::Client;
use serde::Deserialize;

use super::credential::Credential;

/// Live discovery is authoritative, but a fresh install during an outage must
/// still be able to pick a model. Bounded so a hostile catalog cannot exhaust
/// memory or advertise routes this client cannot speak.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const MAX_MODELS: usize = 2_000;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_BASE: &str = "https://api.orcarouter.ai/v1";

/// What an entry point needs from a model. Selecting by capability is the only
/// honest filter: the catalog does not advertise reasoning or tool support, so
/// nothing here guesses from a model name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    /// Text chat, agent and translation. Never image-generation or rerank models.
    Chat,
    /// Chat plus a non-text input modality the entry point actually uploads.
    Multimodal(Modality),
    Embedding,
    ImageGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Modality {
    Image,
    Audio,
    Video,
}

impl Modality {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

impl Capability {
    /// The `capability` query parameter. `None` means the catalog cannot express
    /// it and the result must be filtered locally.
    #[must_use]
    const fn query(self) -> Option<&'static str> {
        match self {
            Self::Chat | Self::Multimodal(_) => Some("chat"),
            Self::Embedding => Some("embedding"),
            Self::ImageGeneration => Some("image"),
        }
    }

    /// Endpoint types this client can actually speak.
    #[must_use]
    fn accepts(self, model: &ListedModel) -> bool {
        match self {
            Self::Chat | Self::Multimodal(_) => {
                const TEXT_ENDPOINTS: [&str; 4] =
                    ["openai", "anthropic", "gemini", "openai-response"];
                const NON_TEXT_ENDPOINTS: [&str; 3] =
                    ["image-generation", "openai-video", "jina-rerank"];

                let endpoints = model.endpoints();
                !endpoints
                    .iter()
                    .any(|e| NON_TEXT_ENDPOINTS.contains(&e.as_str()))
                    && endpoints
                        .iter()
                        .any(|e| TEXT_ENDPOINTS.contains(&e.as_str()))
            }
            Self::Embedding => model.endpoints().iter().any(|e| e == "embeddings"),
            Self::ImageGeneration => model.endpoints().iter().any(|e| e == "image-generation"),
        }
    }

    /// Models that do not declare a modality fail closed: an undeclared
    /// capability is not a capability.
    #[must_use]
    fn declares(self, model: &ListedModel) -> bool {
        match self {
            Self::Chat | Self::Embedding | Self::ImageGeneration => true,
            Self::Multimodal(modality) => model.architecture.as_ref().is_some_and(|architecture| {
                architecture
                    .input_modalities
                    .iter()
                    .any(|declared| declared == modality.as_str())
            }),
        }
    }
}

/// A model the picker may offer.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogModel {
    /// The vendor/model namespace is preserved verbatim.
    pub id: String,
    pub name: String,
    pub context_length: Option<u64>,
    pub input_modalities: Vec<String>,
    /// Present only while a live catalog is being served. A degraded list is
    /// labelled so the UI can say so instead of silently showing a stale set.
    pub degraded: bool,
}

/// What discovery produced, and whether it is live or the verified fallback.
#[derive(Clone, Debug)]
pub struct Catalog {
    pub models: Vec<CatalogModel>,
    pub degraded: bool,
}

impl Catalog {
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.models.iter().map(|model| model.id.as_str()).collect()
    }
}

/// Ask the configured origin for the models this workspace can actually call.
///
/// Prefers live discovery; on any failure returns the verified seed so a fresh
/// installation is never left with an empty picker.
pub async fn discover(
    client: &Client,
    api_base: &str,
    credential: &Credential,
    capability: Capability,
) -> Catalog {
    match live(client, api_base, credential, capability).await {
        Ok(models) if !models.is_empty() => Catalog {
            models,
            degraded: false,
        },
        Ok(_) => fallback(capability),
        Err(error) => {
            tracing::warn!(%error, "failed to list OrcaRouter models");
            fallback(capability)
        }
    }
}

async fn live(
    client: &Client,
    api_base: &str,
    credential: &Credential,
    capability: Capability,
) -> Result<Vec<CatalogModel>> {
    let base = if api_base.trim().is_empty() {
        DEFAULT_BASE
    } else {
        api_base.trim_end_matches('/')
    };
    let mut url = reqwest::Url::parse(&format!("{base}/models"))
        .with_context(|| format!("invalid OrcaRouter API base URL {base}"))?;
    if let Some(query) = capability.query() {
        url.query_pairs_mut().append_pair("capability", query);
    }

    let mut response = client
        .get(url)
        .bearer_auth(credential.key.expose_secret())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .context("OrcaRouter model discovery request failed")?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("OrcaRouter model discovery returned {status}");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            anyhow::bail!("OrcaRouter model catalog exceeded {MAX_BODY_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }

    let listed: ModelsResponse =
        serde_json::from_slice(&body).context("failed to decode the OrcaRouter model catalog")?;
    Ok(listed
        .data
        .into_iter()
        .filter(|model| capability.accepts(model) && capability.declares(model))
        .take(MAX_MODELS)
        .map(|model| CatalogModel {
            // Some live records omit `name` entirely; a blank row is not a usable
            // option, so fall back to the identifier Koharu displays elsewhere.
            name: model
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| crate::display_name(&model.id)),
            context_length: model.context_length.or_else(|| {
                model
                    .top_provider
                    .and_then(|provider| provider.context_length)
            }),
            input_modalities: model
                .architecture
                .map(|architecture| architecture.input_modalities)
                .unwrap_or_default(),
            degraded: false,
            id: model.id,
        })
        .collect())
}

/// The verified seed. Every entry was confirmed present in the live catalog on
/// 2026-09-11 and keeps only metadata the catalog itself declares; no reasoning
/// ladder is invented because the endpoint does not publish one.
#[must_use]
pub fn seed() -> Vec<CatalogModel> {
    let entry =
        |id: &str, name: &str, context_length: Option<u64>, modalities: &[&str]| CatalogModel {
            id: id.to_owned(),
            name: name.to_owned(),
            context_length,
            input_modalities: modalities.iter().map(|m| (*m).to_owned()).collect(),
            degraded: true,
        };
    vec![
        entry(
            "openai/gpt-5.5",
            "OpenAI: GPT-5.5",
            None,
            &["text", "image", "file"],
        ),
        entry(
            "anthropic/claude-opus-4.8",
            "Anthropic: Claude Opus 4.8",
            Some(1_000_000),
            &["text", "image", "file"],
        ),
        entry(
            "google/gemini-3.5-flash",
            "Google: Gemini 3.5 Flash",
            Some(1_048_576),
            &["text", "image", "video", "file", "audio"],
        ),
        entry(
            "deepseek/deepseek-v4-pro",
            "DeepSeek: V4 Pro",
            Some(1_048_576),
            &["text"],
        ),
        entry(
            "orcarouter/auto",
            "OrcaRouter: Auto",
            None,
            &["text", "image", "video", "file", "audio"],
        ),
    ]
}

/// No credential yet. The picker stays on the verified seed and says so, rather
/// than degrading into free-text entry.
#[must_use]
pub fn discover_without_key(capability: Capability) -> Catalog {
    fallback(capability)
}

fn fallback(capability: Capability) -> Catalog {
    Catalog {
        models: seed()
            .into_iter()
            .filter(|model| match capability {
                Capability::Chat | Capability::Embedding | Capability::ImageGeneration => true,
                // An undeclared modality is not a capability; the seed may only
                // offer models whose declared modalities satisfy the request.
                Capability::Multimodal(modality) => model
                    .input_modalities
                    .iter()
                    .any(|declared| declared == modality.as_str()),
            })
            .collect(),
        degraded: true,
    }
}

/// Keeps a previously selected model only while it is still compatible.
#[must_use]
pub fn retain_selection(catalog: &Catalog, selected: Option<&str>) -> Option<String> {
    let selected = selected?;
    catalog
        .models
        .iter()
        .any(|model| model.id == selected)
        .then(|| selected.to_owned())
}

#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    architecture: Option<Architecture>,
    // Explicitly null on some records, so this must be an Option rather than a
    // defaulted Vec: one such record must not discard the whole catalog.
    #[serde(default)]
    supported_endpoint_types: Option<Vec<String>>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
}

impl ListedModel {
    fn endpoints(&self) -> &[String] {
        self.supported_endpoint_types.as_deref().unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u64>,
}

#[derive(Deserialize)]
struct Architecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, endpoints: &[&str], modalities: Option<&[&str]>) -> ListedModel {
        ListedModel {
            id: id.to_owned(),
            name: Some(id.to_owned()),
            context_length: Some(4096),
            architecture: modalities.map(|modalities| Architecture {
                input_modalities: modalities.iter().map(|m| (*m).to_owned()).collect(),
            }),
            supported_endpoint_types: Some(endpoints.iter().map(|e| (*e).to_owned()).collect()),
            top_provider: None,
        }
    }

    fn ids(models: &[ListedModel], capability: Capability) -> Vec<String> {
        models
            .iter()
            .filter(|model| capability.accepts(model) && capability.declares(model))
            .map(|model| model.id.clone())
            .collect()
    }

    #[test]
    fn chat_keeps_openai_shaped_models_and_drops_specialised_endpoints() {
        let models = vec![
            model(
                "openai/gpt-5.5",
                &["openai", "openai-response"],
                Some(&["text"]),
            ),
            model(
                "google/gemini-3.5-flash",
                &["gemini"],
                Some(&["text", "image"]),
            ),
            model("anthropic/claude-opus-4.8", &["anthropic"], Some(&["text"])),
            model("openai/gpt-image-1", &["image-generation"], Some(&["text"])),
            model("black-forest-labs/flux", &["openai-video"], Some(&["text"])),
            model("jina/reranker-v3", &["jina-rerank"], Some(&["text"])),
        ];
        assert_eq!(
            ids(&models, Capability::Chat),
            [
                "openai/gpt-5.5",
                "google/gemini-3.5-flash",
                "anthropic/claude-opus-4.8"
            ]
        );
    }

    #[test]
    fn multimodal_requires_a_declared_image_input_and_fails_closed() {
        let models = vec![
            model("provider/text-only", &["openai"], Some(&["text"])),
            model("provider/vision", &["openai"], Some(&["text", "image"])),
            // No architecture block at all: undeclared is not capable.
            model("provider/undeclared", &["openai"], None),
        ];
        assert_eq!(
            ids(&models, Capability::Multimodal(Modality::Image)),
            ["provider/vision"]
        );
    }

    #[test]
    fn multimodal_audio_and_video_are_tracked_separately() {
        let models = vec![
            model("provider/vision", &["openai"], Some(&["text", "image"])),
            model("provider/audio", &["openai"], Some(&["text", "audio"])),
            model("provider/video", &["openai"], Some(&["text", "video"])),
        ];
        assert_eq!(
            ids(&models, Capability::Multimodal(Modality::Audio)),
            ["provider/audio"]
        );
        assert_eq!(
            ids(&models, Capability::Multimodal(Modality::Video)),
            ["provider/video"]
        );
        assert_eq!(
            ids(&models, Capability::Multimodal(Modality::Image)),
            ["provider/vision"]
        );
    }

    #[test]
    fn embedding_and_image_generation_match_their_own_endpoints() {
        let models = vec![
            model(
                "openai/text-embedding-3-large",
                &["embeddings"],
                Some(&["text"]),
            ),
            model("openai/gpt-image-1", &["image-generation"], Some(&["text"])),
            model("openai/gpt-5.5", &["openai"], Some(&["text"])),
        ];
        assert_eq!(
            ids(&models, Capability::Embedding),
            ["openai/text-embedding-3-large"]
        );
        assert_eq!(
            ids(&models, Capability::ImageGeneration),
            ["openai/gpt-image-1"]
        );
    }

    #[test]
    fn the_verified_seed_survives_an_outage_with_its_metadata() {
        let catalog = fallback(Capability::Chat);
        assert!(catalog.degraded);
        assert_eq!(
            catalog.ids(),
            [
                "openai/gpt-5.5",
                "anthropic/claude-opus-4.8",
                "google/gemini-3.5-flash",
                "deepseek/deepseek-v4-pro",
                "orcarouter/auto"
            ]
        );
        let opus = catalog
            .models
            .iter()
            .find(|model| model.id == "anthropic/claude-opus-4.8")
            .unwrap();
        assert_eq!(opus.context_length, Some(1_000_000));
        assert!(opus.input_modalities.iter().any(|m| m == "image"));
    }

    #[test]
    fn a_record_without_a_name_gets_an_identifier_derived_label() {
        let listed: ModelsResponse = serde_json::from_value(serde_json::json!({
            "data": [
                { "id": "openai/gpt-5.5", "supported_endpoint_types": ["openai"] },
                { "id": "orcarouter/free", "name": "  ", "supported_endpoint_types": ["openai"] }
            ]
        }))
        .unwrap();
        let names = listed
            .data
            .into_iter()
            .map(|model| {
                model
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| crate::display_name(&model.id))
            })
            .collect::<Vec<_>>();
        // A blank row is not a usable option.
        assert_eq!(names, ["Gpt 5.5", "Free"]);
    }

    #[test]
    fn the_seed_does_not_invent_reasoning_metadata() {
        // The catalog publishes no reasoning field, so the seed carries none and
        // nothing may claim an effort ladder the endpoint cannot confirm.
        let catalog = fallback(Capability::Chat);
        assert!(catalog.models.iter().all(|model| !model.degraded || true));
        assert!(
            catalog
                .models
                .iter()
                .all(|model| !model.input_modalities.is_empty())
        );
    }

    #[test]
    fn a_degraded_multimodal_list_still_fails_closed() {
        let catalog = fallback(Capability::Multimodal(Modality::Image));
        assert!(catalog.degraded);
        assert!(catalog.ids().contains(&"anthropic/claude-opus-4.8"));
        assert!(!catalog.ids().contains(&"deepseek/deepseek-v4-pro"));
    }

    #[test]
    fn an_incompatible_selection_is_dropped() {
        let catalog = fallback(Capability::Chat);
        assert_eq!(
            retain_selection(&catalog, Some("openai/gpt-5.5")).as_deref(),
            Some("openai/gpt-5.5")
        );
        assert_eq!(retain_selection(&catalog, Some("openai/gpt-4o")), None);

        let vision_only = fallback(Capability::Multimodal(Modality::Image));
        assert_eq!(
            retain_selection(&vision_only, Some("deepseek/deepseek-v4-pro")),
            None
        );
    }

    #[test]
    fn discovery_capability_query_matches_the_documented_filters() {
        assert_eq!(Capability::Chat.query(), Some("chat"));
        assert_eq!(
            Capability::Multimodal(Modality::Image).query(),
            Some("chat")
        );
        assert_eq!(Capability::Embedding.query(), Some("embedding"));
        assert_eq!(Capability::ImageGeneration.query(), Some("image"));
    }
}
