use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use specta::Type;
use tokio::sync::Mutex;

use crate::{Control, LoginEvent};

use super::token_store::TokenStore;

const AUTH_BASE: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
// This is OpenAI's server-side device-flow exchange identifier. Koharu never
// listens for an OAuth redirect or starts a callback server.
const DEVICE_AUTH_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEVICE_LOGIN_TTL: Duration = Duration::from_secs(15 * 60);
const AUTH_CLAIM: &str = "https://api.openai.com/auth";
const PROFILE_CLAIM: &str = "https://api.openai.com/profile";

#[derive(Clone, Debug, serde::Serialize, Type)]
pub struct Account {
    pub id: String,
    pub email: Option<String>,
    pub plan: Option<String>,
}

#[derive(Clone)]
pub(super) struct Auth {
    client: Client,
    store: TokenStore,
    refresh: Arc<Mutex<()>>,
}

impl fmt::Debug for Auth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Auth").finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(super) struct Session {
    pub(super) access: String,
    pub(super) account: Account,
}

pub(super) struct Tokens {
    pub(super) access: String,
    pub(super) refresh: String,
    pub(super) id: Option<String>,
    pub(super) expires_at_ms: u64,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tokens")
            .field("access", &"[REDACTED]")
            .field("refresh", &"[REDACTED]")
            .field("id", &self.id.as_ref().map(|_| "[REDACTED]"))
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(Clone, Debug)]
struct DeviceCode {
    id: String,
    code: String,
    interval: Duration,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

impl Auth {
    pub(super) fn new(client: Client) -> Self {
        Self {
            client,
            store: TokenStore,
            refresh: Arc::new(Mutex::new(())),
        }
    }

    pub(super) fn account(&self) -> Result<Option<Account>> {
        self.store
            .load()?
            .map(|tokens| account(&tokens))
            .transpose()
    }

    pub(super) async fn session(&self) -> Result<Session> {
        let tokens = self.store.load()?.context("Codex is not signed in")?;
        if tokens.expires_at_ms <= now_ms().saturating_add(60_000) {
            return self.refresh().await;
        }
        Ok(Session {
            account: account(&tokens)?,
            access: tokens.access,
        })
    }

    pub(super) async fn refresh(&self) -> Result<Session> {
        let _refresh = self.refresh.lock().await;
        let current = self.store.load()?.context("Codex is not signed in")?;
        if current.expires_at_ms > now_ms().saturating_add(60_000) {
            return Ok(Session {
                account: account(&current)?,
                access: current.access,
            });
        }
        self.refresh_tokens(current).await
    }

    pub(super) async fn force_refresh(&self) -> Result<Session> {
        let _refresh = self.refresh.lock().await;
        let current = self.store.load()?.context("Codex is not signed in")?;
        self.refresh_tokens(current).await
    }

    async fn refresh_tokens(&self, current: Tokens) -> Result<Session> {
        let response = self
            .client
            .post(format!("{AUTH_BASE}/oauth/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", current.refresh.as_str()),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .context("failed to refresh Codex OAuth credentials")?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.store.delete()?;
        }
        let response = success(response).await?;
        let body: TokenResponse = response
            .json()
            .await
            .context("invalid Codex OAuth refresh response")?;
        let tokens = tokens(body)?;
        self.store.store(&tokens)?;
        Ok(Session {
            account: account(&tokens)?,
            access: tokens.access,
        })
    }

    pub(super) async fn login_device<F>(&self, control: &Control, mut publish: F) -> Result<Account>
    where
        F: FnMut(LoginEvent),
    {
        publish(LoginEvent::Progress {
            message: "Requesting a Codex device code".to_owned(),
        });
        let response = self
            .client
            .post(format!("{AUTH_BASE}/api/accounts/deviceauth/usercode"))
            .json(&serde_json::json!({ "client_id": CLIENT_ID }))
            .send()
            .await?;
        let response = success(response).await?;
        let value: Value = response.json().await?;
        let device = DeviceCode {
            id: string(&value, "device_auth_id")?,
            code: value
                .get("user_code")
                .or_else(|| value.get("usercode"))
                .and_then(Value::as_str)
                .context("device-code response is missing user_code")?
                .to_owned(),
            interval: Duration::from_secs(
                value
                    .get("interval")
                    .and_then(|value| {
                        value
                            .as_u64()
                            .or_else(|| value.as_str()?.parse::<u64>().ok())
                    })
                    .unwrap_or(5)
                    .max(1),
            ),
        };
        publish(LoginEvent::DeviceCode {
            verification_url: format!("{AUTH_BASE}/codex/device"),
            user_code: device.code.clone(),
        });

        let started = tokio::time::Instant::now();
        let (authorization_code, verifier) = loop {
            control.ensure_running()?;
            if started.elapsed() >= DEVICE_LOGIN_TTL {
                bail!("Codex device login expired");
            }
            let response = self
                .client
                .post(format!("{AUTH_BASE}/api/accounts/deviceauth/token"))
                .json(&serde_json::json!({
                    "device_auth_id": device.id,
                    "user_code": device.code,
                }))
                .send()
                .await?;
            if matches!(response.status().as_u16(), 403 | 404) {
                tokio::select! {
                    () = tokio::time::sleep(device.interval) => {}
                    () = control.cancelled() => control.ensure_running()?,
                }
                continue;
            }
            let response = success(response).await?;
            let value: Value = response.json().await?;
            break (
                string(&value, "authorization_code")?,
                string(&value, "code_verifier")?,
            );
        };
        publish(LoginEvent::Progress {
            message: "Completing Codex sign-in".to_owned(),
        });
        let response = self
            .client
            .post(format!("{AUTH_BASE}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT_ID),
                ("code", authorization_code.as_str()),
                ("code_verifier", verifier.as_str()),
                ("redirect_uri", DEVICE_AUTH_REDIRECT_URI),
            ])
            .send()
            .await?;
        let response = success(response).await?;
        let body: TokenResponse = response.json().await?;
        let tokens = tokens(body)?;
        let account = account(&tokens)?;
        self.store.store(&tokens)?;
        Ok(account)
    }

    pub(super) fn logout(&self) -> Result<()> {
        self.store.delete()
    }
}

fn tokens(response: TokenResponse) -> Result<Tokens> {
    let expires_at_ms = response
        .expires_in
        .and_then(|seconds| now_ms().checked_add(seconds.saturating_mul(1000)))
        .or_else(|| {
            jwt(&response.access_token)
                .and_then(|value| value.get("exp")?.as_u64())
                .map(|value| value.saturating_mul(1000))
        })
        .context("Codex OAuth response does not contain a valid expiry")?;
    Ok(Tokens {
        access: response.access_token,
        refresh: response.refresh_token,
        id: response.id_token,
        expires_at_ms,
    })
}

fn account(tokens: &Tokens) -> Result<Account> {
    let claims = jwt(&tokens.access)
        .or_else(|| tokens.id.as_deref().and_then(jwt))
        .context("Codex OAuth token is not a valid JWT")?;
    let auth = claims
        .get(AUTH_CLAIM)
        .and_then(Value::as_object)
        .context("Codex OAuth token does not contain ChatGPT account claims")?;
    let profile = claims.get(PROFILE_CLAIM).and_then(Value::as_object);
    Ok(Account {
        id: auth
            .get("chatgpt_account_id")
            .and_then(Value::as_str)
            .context("Codex OAuth token does not contain a ChatGPT account ID")?
            .to_owned(),
        email: profile
            .and_then(|profile| profile.get("email"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        plan: auth
            .get("chatgpt_plan_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn jwt(value: &str) -> Option<Value> {
    let mut parts = value.split('.');
    let (_header, payload, _signature, None) =
        (parts.next()?, parts.next()?, parts.next()?, parts.next())
    else {
        return None;
    };
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

fn string(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("response is missing {field}"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

async fn success(response: reqwest::Response) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let mut body = response.text().await.unwrap_or_default();
    body.truncate(16 * 1024);
    Err(anyhow!("Codex returned {status}: {body}"))
}
