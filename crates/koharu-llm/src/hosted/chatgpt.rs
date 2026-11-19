//! ChatGPT, the user's plan beside rig's `openai` for OpenAI's API, signed in
//! as OpenAI documents for open-source and locally hosted apps: a client
//! OpenAI registers on the first sign-in, an authorization code with PKCE
//! answered on a loopback port, and the public Responses API, never
//! `backend-api`: <https://developers.openai.com/siwc/token-sharing-open-source>.
//! rig's `chatgpt` dialect speaks the stateless, always-streamed Responses
//! format that route requires; only its endpoint and credential are replaced.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail, ensure};
use futures::future::BoxFuture;
use http::{HeaderValue, header::AUTHORIZATION, request::Parts};
use jsonwebtoken::{DecodingKey, Validation, jwk::JwkSet};
use oauth2::{CsrfToken, PkceCodeChallenge};
use rig_core::{
    catalog::{Catalog, ModelSpec},
    providers::registry::ProviderId,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
use url::Url;

use super::{Http, SignIn, reqwest, unlisted};
use crate::SignInPrompt;

pub(super) use rig_core::providers::chatgpt::PROVIDER_NAME;

const ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
/// Names the signing keys and the revocation endpoint.
const DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";
/// The API the plan's tokens are for, and where its models and Responses are.
pub(super) const RESOURCE: &str = "https://api.openai.com/v1";
/// What a first sign-in registers a client under; OpenAI answers with the
/// client's own id, which every later request uses.
const REGISTRATION_CLIENT: &str = "dynamic_agent_client";
const AGENT_NAME: &str = "Koharu";
/// `chatgpt.tokens.use.direct` is the plan's consent, which the user may
/// decline while signing in.
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// How long before its expiry an access token is refreshed.
const SKEW: Duration = Duration::from_secs(60);
/// Refresh errors after which the sign-in is over, OpenAI's list.
const ENDED: [&str; 6] = [
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

/// The client OpenAI registered for this host, kept under its own secret so
/// signing out and in again reuses it, as OpenAI asks.
#[derive(Deserialize, Serialize)]
struct Registration {
    /// Names this host to OpenAI, an opaque `urn:uuid:` chosen before the
    /// first sign-in, which isn't a credential.
    host_id: String,
    /// Issued on the first sign-in.
    client_id: Option<String>,
}

const REGISTRATION_SECRET: &str = "chatgpt-client";

fn registration() -> Result<Registration> {
    if let Some(record) = koharu_config::secrets::get(REGISTRATION_SECRET)? {
        return Ok(serde_json::from_str(record.expose_secret())?);
    }
    let registration = Registration {
        host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        client_id: None,
    };
    save_registration(&registration)?;
    Ok(registration)
}

fn save_registration(registration: &Registration) -> Result<()> {
    let record = SecretString::from(serde_json::to_string(registration)?);
    Ok(koharu_config::secrets::set(REGISTRATION_SECRET, &record)?)
}

/// A sign-in, as Koharu keeps it under the vendor's name.
#[derive(Deserialize, Serialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    /// When the access token expires, in seconds since the Unix epoch.
    expires_at: u64,
}

/// The sign-in in use, read from the secrets on first use and held while a
/// request refreshes it, as each refresh replaces the refresh token.
static SIGNED_IN: Mutex<Option<Tokens>> = Mutex::const_new(None);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn load() -> Result<Tokens> {
    let record = koharu_config::secrets::get(PROVIDER_NAME)?.context("ChatGPT isn't signed in")?;
    Ok(serde_json::from_str(record.expose_secret())?)
}

fn save(tokens: &Tokens) -> Result<()> {
    let record = SecretString::from(serde_json::to_string(tokens)?);
    Ok(koharu_config::secrets::set(PROVIDER_NAME, &record)?)
}

/// What the token endpoint answers a grant with.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    scope: String,
}

/// A grant at the token endpoint: its answer, or the OAuth error it
/// refused with.
async fn token(form: &[(&str, &str)]) -> Result<Result<TokenResponse, String>> {
    let response = reqwest()?.post(TOKEN_URL).form(form).send().await?;
    let status = response.status();
    if status.is_success() {
        return Ok(Ok(response.json().await?));
    }
    let body = response.text().await.unwrap_or_default();
    let refused = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|body| body.get("error")?.as_str().map(str::to_owned));
    match refused {
        Some(error) if status.is_client_error() => Ok(Err(error)),
        _ => bail!("OpenAI's token endpoint answered {status}: {body}"),
    }
}

#[derive(Deserialize)]
struct Discovery {
    jwks_uri: String,
    revocation_endpoint: String,
}

async fn discovery() -> Result<Discovery> {
    Ok(reqwest()?
        .get(DISCOVERY_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Checks the ID token's signature against OpenAI's keys, and its issuer,
/// audience, expiry, and nonce.
async fn verify(id_token: &str, client_id: &str, nonce: &str) -> Result<()> {
    #[derive(Deserialize)]
    struct Claims {
        nonce: Option<String>,
    }
    let header = jsonwebtoken::decode_header(id_token)?;
    let kid = header.kid.context("the ID token names no signing key")?;
    let keys: JwkSet = reqwest()?
        .get(discovery().await?.jwks_uri)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let key = keys
        .find(&kid)
        .context("OpenAI doesn't publish the ID token's signing key")?;
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims =
        jsonwebtoken::decode::<Claims>(id_token, &DecodingKey::from_jwk(key)?, &validation)?.claims;
    ensure!(
        claims.nonce.as_deref() == Some(nonce),
        "the ID token answers another sign-in"
    );
    Ok(())
}

/// What OpenAI's redirect carries back.
#[derive(Default)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    client_id: Option<String>,
    scope: Option<String>,
    error: Option<String>,
}

/// Waits on `listener` for OpenAI's redirect, and tells the browser the user
/// may return to Koharu.
async fn listen(listener: TcpListener) -> Result<Callback> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        let mut chunk = [0; 2048];
        while !request.contains(&b'\n') && request.len() < 8192 {
            match stream.read(&mut chunk).await? {
                0 => break,
                read => request.extend_from_slice(&chunk[..read]),
            }
        }
        let request = String::from_utf8_lossy(&request);
        let Some(target) = request.split_whitespace().nth(1) else {
            continue;
        };
        let Ok(url) = Url::parse("http://127.0.0.1").and_then(|base| base.join(target)) else {
            continue;
        };
        // A browser also asks for the page's icon, which isn't the answer.
        if url.path() != "/auth/callback" {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let page = "You can close this tab and return to Koharu.";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.shutdown().await;
        let mut callback = Callback::default();
        for (name, value) in url.query_pairs() {
            let value = Some(value.into_owned());
            match name.as_ref() {
                "code" => callback.code = value,
                "state" => callback.state = value,
                "client_id" => callback.client_id = value,
                "scope" => callback.scope = value,
                "error" => callback.error = value,
                _ => {}
            }
        }
        return Ok(callback);
    }
}

/// Signs in with the user's ChatGPT plan in the system browser: `prompt`
/// hears the page to open. The browser's answer comes to a loopback port,
/// and a sign-in that is never answered waits until it's stopped.
pub(super) async fn sign_in(prompt: impl Fn(SignInPrompt) + Send + Sync) -> Result<()> {
    let mut registration = registration()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("couldn't listen for ChatGPT's answer")?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener.local_addr()?.port()
    );
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let state = CsrfToken::new_random();
    let nonce = CsrfToken::new_random();

    let mut url = Url::parse(AUTHORIZE_URL)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair(
                "client_id",
                registration
                    .client_id
                    .as_deref()
                    .unwrap_or(REGISTRATION_CLIENT),
            )
            .append_pair("ext_agent_host_id", &registration.host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", SCOPES)
            .append_pair("resource", RESOURCE)
            .append_pair("state", state.secret())
            .append_pair("nonce", nonce.secret())
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", challenge.as_str());
        if registration.client_id.is_none() {
            query.append_pair("agent_name_hint", AGENT_NAME);
        }
    }
    prompt(SignInPrompt {
        url: url.into(),
        opened: false,
        code: None,
    });

    let callback = listen(listener).await?;
    ensure!(
        callback.state.as_deref() == Some(state.secret()),
        "ChatGPT's answer belongs to another sign-in"
    );
    if let Some(error) = callback.error {
        bail!("ChatGPT didn't sign in: {error}");
    }
    let client_id = match (&registration.client_id, callback.client_id) {
        (Some(known), Some(issued)) => {
            ensure!(*known == issued, "ChatGPT answered for another client");
            issued
        }
        (Some(known), None) => known.clone(),
        (None, Some(issued)) => issued,
        (None, None) => bail!("ChatGPT issued no client"),
    };
    let code = callback.code.context("ChatGPT's answer has no code")?;
    let response = token(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", &code),
        ("code_verifier", verifier.secret()),
        ("redirect_uri", &redirect),
        ("resource", RESOURCE),
    ])
    .await?
    .map_err(|error| anyhow::anyhow!("ChatGPT didn't sign in: {error}"))?;

    verify(
        response
            .id_token
            .as_deref()
            .context("ChatGPT gave no ID token")?,
        &client_id,
        nonce.secret(),
    )
    .await
    .context("ChatGPT's ID token isn't valid")?;
    // The client is OpenAI's now, whether or not the user granted the plan.
    registration.client_id = Some(client_id);
    save_registration(&registration)?;
    let granted = if response.scope.is_empty() {
        callback.scope.unwrap_or_default()
    } else {
        response.scope
    };
    ensure!(
        granted.split(' ').any(|scope| scope == PLAN_SCOPE),
        "ChatGPT plan usage wasn't allowed; sign in again and allow it"
    );
    let tokens = Tokens {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .context("ChatGPT gave no refresh token")?,
        expires_at: now() + response.expires_in,
    };
    let mut signed_in = SIGNED_IN.lock().await;
    save(&tokens)?;
    *signed_in = Some(tokens);
    Ok(())
}

/// Signs out: ChatGPT revokes the refresh token, and the sign-in is deleted
/// whether or not it confirmed. The client stays registered.
pub(super) async fn sign_out() -> Result<()> {
    let mut signed_in = SIGNED_IN.lock().await;
    *signed_in = None;
    if let Ok(tokens) = load()
        && let Err(error) = revoke(&tokens.refresh_token).await
    {
        tracing::warn!("OpenAI didn't revoke ChatGPT's sign-in: {error:#}");
    }
    Ok(koharu_config::secrets::delete(PROVIDER_NAME)?)
}

async fn revoke(refresh_token: &str) -> Result<()> {
    let client_id = registration()?
        .client_id
        .context("no client was registered")?;
    reqwest()?
        .post(discovery().await?.revocation_endpoint)
        .form(&[
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", &client_id),
        ])
        .send()
        .await?
        .error_for_status()?;
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
        let client_id = registration()?
            .client_id
            .context("ChatGPT's client is gone; sign in again")?;
        // The sign-in stays held while a refresh that failed may be tried
        // again, unless OpenAI ended it.
        let renewed = token(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client_id),
            ("refresh_token", &tokens.refresh_token),
            ("resource", RESOURCE),
        ])
        .await;
        match renewed {
            Ok(Ok(response)) => {
                tokens = Tokens {
                    access_token: response.access_token,
                    refresh_token: response.refresh_token.unwrap_or(tokens.refresh_token),
                    expires_at: now() + response.expires_in,
                };
                save(&tokens)?;
            }
            Ok(Err(error)) if ENDED.contains(&error.as_str()) => {
                koharu_config::secrets::delete(PROVIDER_NAME)?;
                bail!("ChatGPT's sign-in has ended ({error}); sign in again");
            }
            Ok(Err(error)) => {
                *signed_in = Some(tokens);
                bail!("OpenAI didn't renew ChatGPT's sign-in: {error}");
            }
            Err(error) => {
                *signed_in = Some(tokens);
                return Err(error.context("couldn't renew ChatGPT's sign-in"));
            }
        }
    }
    let access_token = tokens.access_token.clone();
    *signed_in = Some(tokens);
    Ok(access_token)
}

/// The signed-in plan's credential, on each request.
struct Plan;

impl SignIn for Plan {
    fn authorize<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let bearer = format!("Bearer {}", access_token().await?);
            parts
                .headers
                .insert(AUTHORIZATION, HeaderValue::try_from(bearer)?);
            Ok(())
        })
    }
}

/// The HTTP client for the signed-in plan's models.
pub(super) fn http() -> Result<Http> {
    if koharu_config::secrets::get(PROVIDER_NAME)?.is_none() {
        bail!("ChatGPT isn't signed in");
    }
    Ok(Http::signed_in(Plan))
}

/// The plan's models, from OpenAI's list for the signed-in account, or none
/// when nobody signed in, each as rig's catalog describes it.
pub(super) async fn models(client: &reqwest::Client) -> Result<Vec<ModelSpec>> {
    if koharu_config::secrets::get(PROVIDER_NAME)?.is_none() {
        return Ok(Vec::new());
    }
    let response = client
        .get(format!("{RESOURCE}/models"))
        .bearer_auth(access_token().await?)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("the plan's model list answered {status}: {body}");
    }
    let body: Value = response.json().await?;
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .context("the plan's model list has no `models` array")?;
    let vendor = ProviderId::catalog(PROVIDER_NAME).context("rig knows no ChatGPT vendor")?;
    let catalog = Catalog::builtin();
    Ok(models
        .iter()
        // The models meant for display, in the order OpenAI lists them.
        .filter(|model| model.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter_map(|model| {
            let id = model.get("slug")?.as_str()?;
            if let Some(found) = catalog.get(vendor, id) {
                return Some(found.spec.clone());
            }
            let spec = unlisted(vendor, id);
            Some(match model.get("display_name").and_then(Value::as_str) {
                Some(name) => spec.with_display_name(name),
                None => spec,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The credential is read at each request, in place of the empty key the
    /// model was connected with.
    #[tokio::test]
    async fn a_signed_in_request_carries_the_current_credential() {
        *SIGNED_IN.lock().await = Some(Tokens {
            access_token: "token".to_owned(),
            refresh_token: "refresh".to_owned(),
            expires_at: u64::MAX,
        });
        let (mut parts, ()) = http::Request::post(RESOURCE)
            .header(AUTHORIZATION, "Bearer ")
            .body(())
            .unwrap()
            .into_parts();
        Plan.authorize(&mut parts).await.unwrap();
        assert_eq!(parts.headers[AUTHORIZATION], "Bearer token");
        assert!(!parts.headers.contains_key("chatgpt-account-id"));
    }
}
