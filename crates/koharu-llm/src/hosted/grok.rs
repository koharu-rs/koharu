//! Grok, the user's SuperGrok or X Premium plan beside rig's `xai` for xAI's
//! API, as rig's `chatgpt` is beside `openai`. It signs in with xAI's device
//! code through xAI's shared client, as Hermes Agent and OpenClaw sign in,
//! its tokens kept in Koharu's encrypted secrets, and reaches the plan's
//! models on the endpoint xAI serves the Grok CLI from, as OpenClaw does:
//! <https://github.com/openclaw/openclaw/blob/main/extensions/xai/xai-oauth.ts>
//! and `stream.ts` beside it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, prelude::BASE64_URL_SAFE_NO_PAD};
use futures::future::BoxFuture;
use http::{HeaderMap, HeaderValue, header::AUTHORIZATION, request::Parts};
use oauth2::{
    AccessToken, ClientId, DeviceAuthorizationUrl, EndpointNotSet, EndpointSet, HttpClientError,
    HttpRequest, HttpResponse, RefreshToken, RevocationUrl, Scope,
    StandardDeviceAuthorizationResponse, StandardRevocableToken, TokenResponse as _, TokenUrl,
    basic::{BasicClient, BasicTokenResponse},
};
use rig_core::{
    catalog::{Catalog, ModelFacts, ModelSpec},
    driver::DynModel,
    operation::Completion,
    providers::{openai::OpenAIConfig, registry::ProviderId, xai},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use super::{Http, SignIn, reqwest, unlisted, untimed};
use crate::SignInPrompt;

pub(super) const PROVIDER_NAME: &str = "grok";

/// xAI's shared client for its own tools, whose consent page may name Grok
/// Build.
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
/// `offline_access` brings a refresh token, and `grok-cli:access` the plan's
/// endpoint.
const SCOPES: [&str; 6] = [
    "openid",
    "profile",
    "email",
    "offline_access",
    "grok-cli:access",
    "api:access",
];
// The endpoints https://auth.x.ai/.well-known/openid-configuration names.
const DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const REVOKE_URL: &str = "https://auth.x.ai/oauth2/revoke";
/// The plan's endpoint, which serves the Grok CLI.
const URL: &str = "https://cli-chat-proxy.grok.com";
/// The oldest Grok CLI the endpoint serves, which it reads from each request.
const CLIENT_VERSION: &str = "1.0.13";
/// How long before its expiry an access token is refreshed.
const SKEW: Duration = Duration::from_secs(60);

/// A sign-in, as Koharu keeps it under the vendor's name.
#[derive(Deserialize, Serialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    /// When the access token expires, in seconds since the Unix epoch.
    expires_at: u64,
}

impl Tokens {
    /// The sign-in xAI's `response` gives, keeping `refresh_token` when it
    /// gives no new one.
    fn new(response: &BasicTokenResponse, refresh_token: Option<String>) -> Result<Self> {
        let access_token = response.access_token().secret().clone();
        let expires_at = match response.expires_in() {
            Some(lasts) => now() + lasts.as_secs(),
            // As OpenClaw reads it: the access token's own expiry, not the ID
            // token's, which is the session's.
            None => expiry(&access_token).context("xAI's token doesn't say when it expires")?,
        };
        Ok(Self {
            access_token,
            refresh_token: response
                .refresh_token()
                .map(|token| token.secret().clone())
                .or(refresh_token),
            expires_at,
        })
    }
}

/// The sign-in in use, read from the secrets on first use and held while a
/// request refreshes it, as a refresh token may be spent only once.
static SIGNED_IN: Mutex<Option<Tokens>> = Mutex::const_new(None);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// A JWT's `exp`.
fn expiry(jwt: &str) -> Option<u64> {
    let payload = BASE64_URL_SAFE_NO_PAD.decode(jwt.split('.').nth(1)?).ok()?;
    serde_json::from_slice::<Value>(&payload)
        .ok()?
        .get("exp")?
        .as_u64()
}

type Client = BasicClient<EndpointNotSet, EndpointSet, EndpointNotSet, EndpointSet, EndpointSet>;

fn client() -> Result<Client> {
    Ok(BasicClient::new(ClientId::new(CLIENT_ID.to_owned()))
        .set_device_authorization_url(DeviceAuthorizationUrl::new(DEVICE_URL.to_owned())?)
        .set_token_uri(TokenUrl::new(TOKEN_URL.to_owned())?)
        .set_revocation_url(RevocationUrl::new(REVOKE_URL.to_owned())?))
}

/// An OAuth request on rig's client, as oauth2 sends one on its own reqwest.
async fn send(request: HttpRequest) -> Result<HttpResponse, HttpClientError<reqwest::Error>> {
    let client = reqwest().map_err(|error| HttpClientError::Other(format!("{error:#}")))?;
    let response = client
        .execute(request.try_into().map_err(Box::new)?)
        .await
        .map_err(Box::new)?;
    let mut answer = http::Response::builder().status(response.status());
    for (name, value) in response.headers() {
        answer = answer.header(name, value);
    }
    Ok(answer.body(response.bytes().await.map_err(Box::new)?.to_vec())?)
}

fn load() -> Result<Tokens> {
    let record = koharu_config::secrets::get(PROVIDER_NAME)?.context("Grok isn't signed in")?;
    Ok(serde_json::from_str(record.expose_secret())?)
}

fn save(tokens: &Tokens) -> Result<()> {
    let record = SecretString::from(serde_json::to_string(tokens)?);
    Ok(koharu_config::secrets::set(PROVIDER_NAME, &record)?)
}

/// Signs in with the user's plan: `prompt` hears the page to enter a code
/// on, and the code. A sign-in that outlasts the code fails.
pub(super) async fn sign_in(prompt: impl Fn(SignInPrompt) + Send + Sync) -> Result<()> {
    let client = client()?;
    let device: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .add_scopes(SCOPES.map(|scope| Scope::new(scope.to_owned())))
        .request_async(&send)
        .await
        .context("xAI gave no code to sign in with")?;
    prompt(SignInPrompt {
        url: device.verification_uri_complete().map_or_else(
            || device.verification_uri().to_string(),
            |url| url.secret().clone(),
        ),
        opened: false,
        code: Some(device.user_code().secret().clone()),
    });
    let response = client
        .exchange_device_access_token(&device)
        .request_async(&send, tokio::time::sleep, None)
        .await
        .context("xAI didn't sign in")?;
    let tokens = Tokens::new(&response, None)?;
    let mut signed_in = SIGNED_IN.lock().await;
    save(&tokens)?;
    *signed_in = Some(tokens);
    Ok(())
}

/// Signs out: xAI revokes the sign-in, and it's deleted whether or not xAI
/// confirmed.
pub(super) async fn sign_out() -> Result<()> {
    let mut signed_in = SIGNED_IN.lock().await;
    *signed_in = None;
    if let Ok(tokens) = load()
        && let Err(error) = revoke(tokens).await
    {
        tracing::warn!("xAI didn't revoke Grok's sign-in: {error:#}");
    }
    Ok(koharu_config::secrets::delete(PROVIDER_NAME)?)
}

/// Revokes the refresh token, which ends the access tokens it brought too,
/// or the access token when there is none.
async fn revoke(tokens: Tokens) -> Result<()> {
    let token = match tokens.refresh_token {
        Some(token) => StandardRevocableToken::RefreshToken(RefreshToken::new(token)),
        None => StandardRevocableToken::AccessToken(AccessToken::new(tokens.access_token)),
    };
    client()?.revoke_token(token)?.request_async(&send).await?;
    Ok(())
}

/// The sign-in's access token, refreshed first when it's about to expire.
async fn access_token() -> Result<String> {
    let mut signed_in = SIGNED_IN.lock().await;
    let mut tokens = match signed_in.take() {
        Some(tokens) => tokens,
        None => load()?,
    };
    if tokens.expires_at <= now() + SKEW.as_secs() {
        let refresh_token = tokens
            .refresh_token
            .context("Grok's sign-in has expired; sign in again")?;
        let response = client()?
            .exchange_refresh_token(&RefreshToken::new(refresh_token.clone()))
            .request_async(&send)
            .await
            .context("xAI didn't renew Grok's sign-in; sign in again")?;
        tokens = Tokens::new(&response, Some(refresh_token))?;
        save(&tokens)?;
    }
    let access_token = tokens.access_token.clone();
    *signed_in = Some(tokens);
    Ok(access_token)
}

/// What the plan's endpoint requires of each request, as OpenClaw sends it:
/// the sign-in's token, the Grok CLI's identity, and the model asked for.
async fn headers(model: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", access_token().await?);
    headers.insert(AUTHORIZATION, HeaderValue::try_from(bearer)?);
    headers.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
    headers.insert(
        "x-grok-client-version",
        HeaderValue::from_static(CLIENT_VERSION),
    );
    if let Some(model) = model {
        headers.insert("x-grok-model-override", HeaderValue::try_from(model)?);
    }
    Ok(headers)
}

/// A request for the model this names.
struct Plan(String);

impl SignIn for Plan {
    fn authorize<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            parts.headers.extend(headers(Some(&self.0)).await?);
            Ok(())
        })
    }
}

/// What rig's catalog says of xAI's `id`, filed under the plan, or nothing
/// certain of a model it doesn't list.
fn spec(id: &str) -> Result<ModelSpec> {
    let plan = ProviderId::declare(PROVIDER_NAME)?;
    let api = ProviderId::catalog(xai::PROVIDER_NAME).context("rig knows no xAI vendor")?;
    Ok(match Catalog::builtin().get(api, id) {
        Some(found) => {
            let mut spec = found.spec.clone();
            spec.provider = plan;
            spec.id = id.to_owned();
            spec
        }
        None => unlisted(plan, id),
    })
}

/// The plan's model `id`, on rig's xAI dialect at `base_url` or the plan's
/// endpoint.
pub(super) fn connect(id: &str, base_url: Option<String>) -> Result<DynModel<Completion>> {
    if koharu_config::secrets::get(PROVIDER_NAME)?.is_none() {
        bail!("Grok isn't signed in");
    }
    // The HTTP client puts the sign-in's token on each request.
    let mut model = OpenAIConfig::with_key(&xai::DIALECT, String::new())
        .with_base_url(base_url.unwrap_or_else(|| URL.to_owned()))
        .connect(Http::signed_in(Plan(id.to_owned())))
        .completion(id);
    model.wire = model.wire.with_facts(ModelFacts::new(spec(id)?));
    Ok(untimed(model))
}

/// The plan's language models, from its endpoint's list, or none when
/// nobody signed in.
pub(super) async fn models(base_url: Option<String>) -> Result<Vec<ModelSpec>> {
    if koharu_config::secrets::get(PROVIDER_NAME)?.is_none() {
        return Ok(Vec::new());
    }
    let base_url = base_url.as_deref().unwrap_or(URL);
    let response = reqwest()?
        .get(format!("{base_url}/v1/models"))
        .headers(headers(None).await?)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("the plan's model list answered {status}: {body}");
    }
    let body: Value = response.json().await?;
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .context("the plan's model list has no `data` array")?;
    let mut models = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        // As OpenClaw keeps them: the models served on a text backend.
        let backend = row.get("api_backend").and_then(Value::as_str);
        if backend.is_some_and(|backend| {
            !["responses", "chat", "language"]
                .iter()
                .any(|text| backend.eq_ignore_ascii_case(text))
        }) {
            continue;
        }
        let mut spec = spec(id)?;
        // A model the catalog knows as neither calling tools nor keeping to a
        // schema writes no text Koharu reads.
        if !spec.tools && !spec.structured_output {
            continue;
        }
        if let Some(name) = row.get("name").and_then(Value::as_str) {
            spec = spec.with_display_name(name);
        }
        models.push(spec);
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The endpoint turns away a request without the sign-in's token, the
    /// Grok CLI's identity, or the model.
    #[tokio::test]
    async fn a_request_to_the_plan_carries_what_its_endpoint_requires() {
        *SIGNED_IN.lock().await = Some(Tokens {
            access_token: "token".to_owned(),
            refresh_token: None,
            expires_at: u64::MAX,
        });
        let (mut parts, ()) = http::Request::post(URL)
            .header(AUTHORIZATION, "Bearer ")
            .body(())
            .unwrap()
            .into_parts();
        Plan("grok-4.7".to_owned())
            .authorize(&mut parts)
            .await
            .unwrap();
        assert_eq!(parts.headers[AUTHORIZATION], "Bearer token");
        assert_eq!(parts.headers["x-xai-token-auth"], "xai-grok-cli");
        assert_eq!(parts.headers["x-grok-model-override"], "grok-4.7");
    }
}
