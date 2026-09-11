// https://www.orcarouter.ai
//
// OrcaRouter is an OpenAI-compatible gateway. One API key reaches every model it
// routes, and the same key can be obtained either by pasting an `sk-orca-…`
// value or by authorizing this app in a browser (OAuth 2.0 + PKCE).

pub mod catalog;
pub mod credential;
pub mod pkce;

use anyhow::Context as _;
use koharu_secrets::ExposeSecret as _;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

use super::send_json;
use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image, prompt,
};
use credential::CredentialProvider as _;

/// Inference origin. Authentication lives on a different origin entirely; never
/// derive one from the other and never append `/auth` here.
pub const DEFAULT_API_BASE_URL: &str = "https://api.orcarouter.ai/v1";
/// Consent-screen origin.
pub const DEFAULT_AUTH_BASE_URL: &str = "https://www.orcarouter.ai";

const ORCA_BASE_URL: &str = "ORCA_BASE_URL";
const ORCA_AUTH_BASE_URL: &str = "ORCA_AUTH_BASE_URL";
const ORCA_API_BASE_URL: &str = "ORCA_API_BASE_URL";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct OrcaRouterConfig {
    /// Explicit inference override. Takes precedence over the shared base.
    pub api_base_url: Option<Url>,
    /// Explicit auth override. Takes precedence over the shared base.
    pub auth_base_url: Option<Url>,
}

impl Default for OrcaRouterConfig {
    fn default() -> Self {
        Self {
            api_base_url: None,
            auth_base_url: None,
        }
    }
}

impl OrcaRouterConfig {
    /// Inference base. Explicit config, then the API override, then the shared
    /// self-hosted base, then the public default.
    #[must_use]
    pub fn api_base(&self) -> String {
        self.api_base_url
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| override_from_env(ORCA_API_BASE_URL))
            .or_else(|| override_from_env(ORCA_BASE_URL))
            .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned())
    }

    /// Authentication base, resolved independently of [`Self::api_base`].
    #[must_use]
    pub fn auth_base(&self) -> String {
        self.auth_base_url
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| override_from_env(ORCA_AUTH_BASE_URL))
            .or_else(|| override_from_env(ORCA_BASE_URL))
            .unwrap_or_else(|| DEFAULT_AUTH_BASE_URL.to_owned())
    }
}

fn override_from_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
}

/// Reject a remote origin that would send a key in the clear. Loopback may use
/// plain HTTP for local development.
fn require_secure(origin: &str, what: &str) -> anyhow::Result<Url> {
    let url = Url::parse(origin).with_context(|| format!("invalid {what} base URL {origin}"))?;
    let loopback = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "{what} base URL must use HTTPS unless it is loopback"
    );
    Ok(url)
}

/// Append a path to an API base without losing the last base segment.
///
/// Relative URL resolution replaces the final segment, so joining against
/// `https://api.orcarouter.ai/v1` would silently drop `/v1` and hit the wrong
/// endpoint. The base is normalised to a directory first.
fn endpoint(base: &Url, path: &str) -> anyhow::Result<Url> {
    let directory = format!("{}/", base.as_str().trim_end_matches('/'));
    Url::parse(&directory)?
        .join(path)
        .with_context(|| format!("failed to build the OrcaRouter {path} URL"))
}

pub(super) async fn translate(
    client: &Client,
    config: &OrcaRouterConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let credential = credential::ApiKeyAdapter
        .credential()
        .await
        .map_err(crate::Error::Other)?;
    let base = require_secure(&config.api_base(), "OrcaRouter API").map_err(crate::Error::Other)?;
    let url = endpoint(&base, "chat/completions").map_err(crate::Error::Other)?;

    let (system, user) = prompt::prompts(request)?;
    let user_content = match request.image.as_deref() {
        Some(image) => MessageContent::Parts(vec![
            ContentPart::Text { text: user },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: encode_image(image)?.data_url(),
                },
            },
        ]),
        None => MessageContent::Text(user),
    };
    let body = ChatRequest {
        model,
        messages: [
            Message {
                role: "system",
                content: MessageContent::Text(system),
            },
            Message {
                role: "user",
                content: user_content,
            },
        ],
        temperature: generation.temperature,
        top_p: generation.top_p,
        max_tokens: generation.max_tokens,
        frequency_penalty: generation.frequency_penalty,
        presence_penalty: generation.presence_penalty,
        response_format: ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchema {
                name: "manga_translation",
                strict: true,
                schema: prompt::output_schema(request.segments.len()),
            },
        },
    };

    let response: ChatResponse = send_json(
        "orcarouter",
        client
            .post(url)
            .bearer_auth(credential.key.expose_secret())
            .json(&body),
    )
    .await
    .map_err(|error| classify(error, &credential))?;

    let text = response
        .choices
        .into_iter()
        .next()
        .context("OrcaRouter returned no choices")?
        .message
        .content
        .context("OrcaRouter returned no message content")?;
    Ok(prompt::translations(
        "orcarouter",
        &text,
        &request.segments,
    )?)
}

/// A revoked key is a terminal reauthentication requirement, not a retry loop:
/// record the exact generation the relay rejected and let the UI ask the user to
/// reconnect. A durable key is never refreshed.
fn classify(error: crate::Error, credential: &credential::Credential) -> crate::Error {
    if let crate::Error::Api { status: 401, .. } = &error
        && credential::invalidates(credential)
        && let Err(mark) = credential::mark_needs_reauth(&credential.generation)
    {
        tracing::warn!(%mark, "failed to record the OrcaRouter reauthentication requirement");
    }
    error
}

/// Discover the models this workspace can call. Text and multimodal entries use
/// the same live catalog, filtered by the capability the entry point needs.
pub(super) async fn models(client: &Client, config: &OrcaRouterConfig) -> Result<Vec<Model>> {
    let Ok(credential) = credential::ApiKeyAdapter.credential().await else {
        // No key yet: the picker stays empty and the settings UI asks for one.
        return Ok(Vec::new());
    };
    let Ok(base) = require_secure(&config.api_base(), "OrcaRouter API") else {
        return Ok(Vec::new());
    };
    let catalog = catalog::discover(
        client,
        &base.to_string(),
        &credential,
        catalog::Capability::Chat,
    )
    .await;
    Ok(catalog
        .models
        .into_iter()
        .map(|model| Model {
            provider: Provider::OrcaRouter,
            vision: model
                .input_modalities
                .iter()
                .any(|modality| modality == "image"),
            reasoning: false,
            name: if model.name.is_empty() {
                crate::display_name(&model.id)
            } else {
                model.name
            },
            model: Some(model.id),
            quantizations: Vec::new(),
        })
        .collect())
}

/// Options offered for the OrcaRouter provider, from the live catalog.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct OrcaRouterModel {
    pub id: String,
    pub name: String,
    pub context_length: Option<u64>,
    pub input_modalities: Vec<String>,
    pub degraded: bool,
}

/// The capability-filtered catalog an entry point should bind to its selector.
pub async fn available_models(
    capability: catalog::Capability,
) -> anyhow::Result<(Vec<OrcaRouterModel>, bool)> {
    let config = OrcaRouterConfig::default();
    let client = koharu_runtime::http_client()?;
    let base = require_secure(&config.api_base(), "OrcaRouter API")?;
    let catalog = match credential::ApiKeyAdapter.credential().await {
        Ok(credential) => {
            catalog::discover(&client, &base.to_string(), &credential, capability).await
        }
        Err(_) => catalog::discover_without_key(capability),
    };
    Ok((
        catalog
            .models
            .into_iter()
            .map(|model| OrcaRouterModel {
                id: model.id,
                name: model.name,
                context_length: model.context_length,
                input_modalities: model.input_modalities,
                degraded: model.degraded,
            })
            .collect(),
        catalog.degraded,
    ))
}

/// Start a browser authorization and return the consent URL immediately.
///
/// The URL is not a secret, so returning it lets the user open it manually when
/// no browser is registered. The code exchange happens off this call.
pub async fn start_authorization() -> anyhow::Result<String> {
    let config = OrcaRouterConfig::default();
    let auth_base = require_secure(&config.auth_base(), "OrcaRouter auth")?;
    let (listener, callback_url) = pkce::loopback_listener().await?;
    let attempt = pkce::Pkce::new();
    let url = pkce::authorize_url(&auth_base.to_string(), &attempt, &callback_url)?;
    // The verifier stays in this process; the UI only ever sees the URL.
    let client = koharu_runtime::http_client()?;
    tokio::spawn(async move {
        match pkce::listen(listener, attempt.state(), pkce::TTL).await {
            Ok(code) => {
                match pkce::exchange(&client, &auth_base.to_string(), &code, attempt.verifier())
                    .await
                {
                    Ok(issued) => {
                        if issued.scope.as_deref() != Some("api") {
                            tracing::warn!(
                                scope = issued.scope.as_deref().unwrap_or("unknown"),
                                "OrcaRouter granted less than the requested scope"
                            );
                        }
                        if let Err(error) = credential::persist(
                            issued.key.into(),
                            credential::CredentialSource::Authorization,
                        ) {
                            tracing::warn!(%error, "failed to store the OrcaRouter credential");
                        }
                    }
                    Err(error) => tracing::warn!(%error, "OrcaRouter code exchange failed"),
                }
            }
            Err(error) => tracing::warn!(%error, "OrcaRouter authorization did not complete"),
        }
    });
    Ok(url)
}

/// Present the consent screen and wait for it to complete.
pub async fn connect() -> anyhow::Result<()> {
    let config = OrcaRouterConfig::default();
    let auth_base = require_secure(&config.auth_base(), "OrcaRouter auth")?;
    let client = koharu_runtime::http_client()?;
    credential::connect(&client, &auth_base.to_string()).await?;
    Ok(())
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    json_schema: JsonSchema,
}

#[derive(Serialize)]
struct JsonSchema {
    name: &'static str,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: MessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Tests share these environment variables and the process-wide keyring, so
    /// they run one at a time.
    pub(crate) struct StoreGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

    pub(crate) fn store_guard() -> StoreGuard {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        StoreGuard(
            LOCK.get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    use super::*;

    #[test]
    fn serializes_orcarouter_request_contract() {
        let body = ChatRequest {
            model: "provider/model",
            messages: [
                Message {
                    role: "system",
                    content: MessageContent::Text("system".to_owned()),
                },
                Message {
                    role: "user",
                    content: MessageContent::Text("user".to_owned()),
                },
            ],
            temperature: None,
            top_p: None,
            max_tokens: Some(1024),
            frequency_penalty: None,
            presence_penalty: None,
            response_format: ResponseFormat {
                kind: "json_schema",
                json_schema: JsonSchema {
                    name: "manga_translation",
                    strict: true,
                    schema: prompt::output_schema(2),
                },
            },
        };
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
        assert_eq!(value["max_tokens"], 1024);
    }

    #[test]
    fn inference_and_auth_default_to_different_origins() {
        // This reads the process environment, so it must hold the same lock the
        // override tests use.
        let _guard = store_guard();
        unsafe {
            std::env::remove_var(ORCA_BASE_URL);
            std::env::remove_var(ORCA_AUTH_BASE_URL);
            std::env::remove_var(ORCA_API_BASE_URL);
        }
        let config = OrcaRouterConfig::default();
        assert_eq!(config.api_base(), "https://api.orcarouter.ai/v1");
        assert_eq!(config.auth_base(), "https://www.orcarouter.ai");
        // The single most common integration mistake: auth is not under /v1.
        assert!(!config.auth_base().contains("/v1"));
    }

    #[test]
    fn the_shared_base_applies_to_both_origins_and_explicit_overrides_win() {
        let _guard = store_guard();
        // SAFETY: guarded above; no other thread reads these concurrently.
        unsafe {
            std::env::set_var(ORCA_BASE_URL, "https://self-hosted.example");
            std::env::remove_var(ORCA_AUTH_BASE_URL);
            std::env::remove_var(ORCA_API_BASE_URL);
        }
        let shared = OrcaRouterConfig::default();
        assert_eq!(shared.api_base(), "https://self-hosted.example");
        assert_eq!(shared.auth_base(), "https://self-hosted.example");

        unsafe {
            std::env::set_var(ORCA_AUTH_BASE_URL, "https://auth.example");
            std::env::set_var(ORCA_API_BASE_URL, "https://api.example/v1");
        }
        let split = OrcaRouterConfig::default();
        assert_eq!(split.api_base(), "https://api.example/v1");
        assert_eq!(split.auth_base(), "https://auth.example");

        unsafe {
            std::env::remove_var(ORCA_BASE_URL);
            std::env::remove_var(ORCA_AUTH_BASE_URL);
            std::env::remove_var(ORCA_API_BASE_URL);
        }
    }

    #[test]
    fn explicit_config_beats_every_environment_override() {
        let _guard = store_guard();
        unsafe { std::env::set_var(ORCA_BASE_URL, "https://self-hosted.example") };
        let config = OrcaRouterConfig {
            api_base_url: Some(Url::parse("https://explicit.example/v1").unwrap()),
            auth_base_url: Some(Url::parse("https://explicit-auth.example").unwrap()),
        };
        assert_eq!(config.api_base(), "https://explicit.example/v1");
        // `Url` normalises a bare origin with a trailing slash; both forms are
        // equivalent origins.
        assert_eq!(config.auth_base(), "https://explicit-auth.example/");
        unsafe { std::env::remove_var(ORCA_BASE_URL) };
    }

    #[test]
    fn the_chat_endpoint_keeps_the_version_segment_of_the_base() {
        // Relative resolution would replace `v1` without the directory
        // normalisation, sending every request to the wrong path.
        let base = Url::parse(DEFAULT_API_BASE_URL).unwrap();
        assert_eq!(
            endpoint(&base, "chat/completions").unwrap().as_str(),
            "https://api.orcarouter.ai/v1/chat/completions"
        );

        let trailing = Url::parse("https://api.orcarouter.ai/v1/").unwrap();
        assert_eq!(
            endpoint(&trailing, "chat/completions").unwrap().as_str(),
            "https://api.orcarouter.ai/v1/chat/completions"
        );

        // A self-hosted base mounted under a prefix keeps the prefix too.
        let self_hosted = Url::parse("https://gateway.internal/openai").unwrap();
        assert_eq!(
            endpoint(&self_hosted, "chat/completions").unwrap().as_str(),
            "https://gateway.internal/openai/chat/completions"
        );
    }

    #[test]
    fn remote_origins_require_https_but_loopback_may_use_http() {
        assert!(require_secure("https://api.orcarouter.ai/v1", "test").is_ok());
        assert!(require_secure("http://api.orcarouter.ai/v1", "test").is_err());
        assert!(require_secure("http://127.0.0.1:8080/v1", "test").is_ok());
        assert!(require_secure("http://localhost:8080/v1", "test").is_ok());
        assert!(require_secure("not a url", "test").is_err());
    }

    #[test]
    fn a_revoked_key_marks_the_rejected_generation_without_refreshing() {
        let _guard = store_guard();
        let _ = credential::forget();
        let stale = credential::Credential {
            key: koharu_secrets::SecretString::from("sk-orca-test"),
            source: credential::CredentialSource::ApiKey,
            generation: "gen-1".to_owned(),
        };
        let error = crate::Error::Api {
            provider: "orcarouter",
            status: 401,
            message: "unauthorized".to_owned(),
        };
        let classified = classify(error, &stale);
        // The error is surfaced unchanged; the durable key is never re-minted.
        assert!(matches!(classified, crate::Error::Api { status: 401, .. }));
        assert_eq!(credential::reauth_generation().as_deref(), Some("gen-1"));
        let _ = credential::forget();
    }
}
