// https://www.orcarouter.ai/auth
// https://www.orcarouter.ai/api/v1/auth/keys
//
// OAuth 2.0 authorization-code + PKCE (RFC 7636) without a client secret.
// The verifier never leaves this process and is never logged.

use std::{net::Ipv4Addr, time::Duration};

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng as _;
use reqwest::Client;
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};

/// Authorize endpoint. Fixed: the consent screen is not an API.
pub const AUTHORIZE_PATH: &str = "/auth";
/// Code exchange endpoint. Never `/v1/auth/keys` — the relay is at `/v1`, auth is not.
pub const EXCHANGE_PATH: &str = "/api/v1/auth/keys";
/// Consent screen for the out-of-band delivery mode.
const CALLBACK_OOB: &str = "oob";
const SCOPE: &str = "api";
const VERIFIER_BYTES: usize = 32;
const STATE_BYTES: usize = 16;
const LOGIN_TTL: Duration = Duration::from_secs(10 * 60);
const APP_NAME: &str = "Koharu";

/// A fresh verifier/challenge/state triple. Every authorization attempt builds
/// one of these from the operating system's cryptographic RNG.
pub struct Pkce {
    verifier: String,
    challenge: String,
    state: String,
}

impl std::fmt::Debug for Pkce {
    /// The verifier is a bearer secret until it is spent: keep it out of logs.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pkce")
            .field("challenge", &self.challenge)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Pkce {
    #[must_use]
    pub fn new() -> Self {
        let verifier = random(VERIFIER_BYTES);
        let challenge = challenge(&verifier);
        Self {
            verifier,
            challenge,
            state: random(STATE_BYTES),
        }
    }

    #[must_use]
    pub fn challenge(&self) -> &str {
        &self.challenge
    }

    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// The verifier is handed to [`exchange`] and to nothing else.
    #[must_use]
    pub fn verifier(&self) -> &str {
        &self.verifier
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// The key the user's account issued to this app, plus the scope actually granted.
#[derive(Clone)]
pub struct IssuedKey {
    pub key: String,
    /// Granted scope, read back from the response. A client that asked for
    /// `connector` and reads `api` was approved for less than it requested.
    pub scope: Option<String>,
}

impl std::fmt::Debug for IssuedKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedKey")
            .field("key", &"[REDACTED]")
            .field("scope", &self.scope)
            .finish()
    }
}

/// `base64url(sha256(verifier))`, unpadded. This is what rides the authorize URL.
#[must_use]
pub fn challenge(verifier: &str) -> String {
    use sha2::{Digest as _, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

/// Build the consent URL. Always S256: even in the loopback flow the user may
/// choose "Show me a code", which puts the code in human hands.
pub fn authorize_url(auth_base: &str, pkce: &Pkce, callback_url: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(auth_base)
        .with_context(|| format!("invalid OrcaRouter auth base URL {auth_base}"))?;
    url.set_path(AUTHORIZE_PATH);
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("callback_url", callback_url)
        .append_pair("code_challenge", pkce.challenge())
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", pkce.state())
        .append_pair("app_name", APP_NAME)
        .append_pair("scope", SCOPE);
    Ok(url.into())
}

#[derive(Deserialize)]
struct ExchangeResponse {
    key: String,
    #[serde(default)]
    scope: Option<String>,
}

/// Redeem an auth code. Codes are single use with a 10 minute TTL.
pub async fn exchange(
    client: &Client,
    auth_base: &str,
    code: &str,
    verifier: &str,
) -> Result<IssuedKey> {
    let mut url = reqwest::Url::parse(auth_base)
        .with_context(|| format!("invalid OrcaRouter auth base URL {auth_base}"))?;
    url.set_path(EXCHANGE_PATH);
    url.set_query(None);

    let response = client
        .post(url)
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await
        .context("OrcaRouter code exchange failed")?;

    let status = response.status();
    if !status.is_success() {
        // 400: unrecognised or downgraded code_challenge_method.
        // 403: code unknown, expired, already used, or verifier mismatch.
        // 429: too many PKCE keys issued for this account in 24 hours.
        let body = response.text().await.unwrap_or_default();
        return Err(ExchangeError {
            status: status.as_u16(),
            detail: body.chars().take(512).collect(),
        }
        .into());
    }

    let issued: ExchangeResponse = response
        .json()
        .await
        .context("OrcaRouter code exchange returned an invalid body")?;
    if issued.key.trim().is_empty() {
        bail!("OrcaRouter code exchange returned an empty key");
    }
    Ok(IssuedKey {
        key: issued.key,
        scope: issued.scope,
    })
}

/// A terminal exchange failure. The message never contains the code or verifier.
#[derive(Debug, thiserror::Error)]
#[error("OrcaRouter authorization failed ({status}): {detail}")]
pub struct ExchangeError {
    pub status: u16,
    pub detail: String,
}

impl ExchangeError {
    /// The user can act on a denial, an expired code, or the 10-keys-per-day cap.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        matches!(self.status, 400 | 403 | 429)
    }
}

/// Why a loopback callback did not produce a code.
#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error("authorization was denied")]
    Denied,
    #[error("authorization response did not match this attempt")]
    StateMismatch,
    #[error("authorization timed out")]
    Timeout,
    #[error("authorization returned an error: {0}")]
    Other(String),
}

/// Serve the loopback redirect for a single attempt and return the auth code.
///
/// The listener binds before the browser opens so the port is known and no
/// callback can race the server start.
pub async fn listen(
    listener: TcpListener,
    expected_state: &str,
    deadline: Duration,
) -> Result<String, CallbackError> {
    let accepted = tokio::time::timeout(deadline, listener.accept())
        .await
        .map_err(|_| CallbackError::Timeout)?
        .map_err(|error| CallbackError::Other(error.to_string()))?;
    let (mut stream, _) = accepted;

    let mut buffer = vec![0u8; 8 * 1024];
    let read = stream
        .read(&mut buffer)
        .await
        .map_err(|error| CallbackError::Other(error.to_string()))?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let target = request
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let query = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|error| CallbackError::Other(error.to_string()))?
        .query_pairs()
        .into_owned()
        .collect::<Vec<_>>();

    let value = |name: &str| {
        query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };

    let body = "<!doctype html><meta charset=utf-8><title>Koharu</title>\
                <p>Connected. You can close this tab and return to Koharu.</p>";
    let _ = stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await;
    let _ = stream.shutdown().await;

    // Constant-time compare: this is the only thing standing between this
    // listener and a code some other page dropped on it.
    if !constant_time_eq(
        value("state").as_deref().unwrap_or_default(),
        expected_state,
    ) {
        return Err(CallbackError::StateMismatch);
    }
    if let Some(error) = value("error") {
        return Err(if error == "access_denied" {
            CallbackError::Denied
        } else {
            CallbackError::Other(error)
        });
    }
    value("code").ok_or(CallbackError::Other("missing code".to_owned()))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Bind a loopback listener for the redirect flow. Flow A is used because
/// Koharu is a desktop application that owns a process able to listen.
pub async fn loopback_listener() -> Result<(TcpListener, String)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .context("failed to bind the OrcaRouter loopback callback")?;
    let port = listener.local_addr()?.port();
    Ok((listener, format!("http://127.0.0.1:{port}/cb")))
}

/// The literal callback marker for the out-of-band delivery mode.
#[must_use]
pub fn out_of_band_callback() -> &'static str {
    CALLBACK_OOB
}

pub const TTL: Duration = LOGIN_TTL;

#[cfg(test)]
mod tests {
    use super::*;

    const VECTOR_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const VECTOR_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn challenge_matches_rfc7636_appendix_b() {
        assert_eq!(challenge(VECTOR_VERIFIER), VECTOR_CHALLENGE);
    }

    #[test]
    fn challenge_is_unpadded_base64url() {
        let challenge = challenge(VECTOR_VERIFIER);
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn every_attempt_gets_a_fresh_verifier_and_state() {
        let attempts = (0..64).map(|_| Pkce::new()).collect::<Vec<_>>();
        for (index, attempt) in attempts.iter().enumerate() {
            assert_ne!(attempt.verifier(), "");
            for other in &attempts[index + 1..] {
                assert_ne!(attempt.verifier(), other.verifier());
                assert_ne!(attempt.state(), other.state());
                assert_ne!(attempt.challenge(), other.challenge());
            }
        }
    }

    #[test]
    fn verifier_is_never_derived_from_a_guessable_input() {
        // Same-verifier reuse and timestamp/user/salt derivation would show up
        // as repeats or as a value that can be rebuilt from public inputs.
        let first = Pkce::new();
        let second = Pkce::new();
        assert_ne!(first.verifier(), second.verifier());
        assert_eq!(first.verifier().len(), 43);
    }

    #[test]
    fn verifier_is_absent_from_its_own_debug_output() {
        let pkce = Pkce::new();
        let rendered = format!("{pkce:?}");
        assert!(!rendered.contains(pkce.verifier()));
        assert!(rendered.contains(pkce.challenge()));
    }

    #[test]
    fn issued_key_is_absent_from_its_own_debug_output() {
        let issued = IssuedKey {
            key: "sk-orca-not-a-real-key".to_owned(),
            scope: Some("api".to_owned()),
        };
        let rendered = format!("{issued:?}");
        assert!(!rendered.contains("sk-orca-not-a-real-key"));
    }

    #[test]
    fn authorize_url_targets_the_auth_origin_with_s256() {
        let pkce = Pkce::new();
        let url = reqwest::Url::parse(
            &authorize_url(
                "https://www.orcarouter.ai",
                &pkce,
                "http://127.0.0.1:51733/cb",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("www.orcarouter.ai"));
        assert_eq!(url.path(), "/auth");
        let query = url.query_pairs().into_owned().collect::<Vec<_>>();
        let get = |name: &str| {
            query
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(get("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(get("code_challenge").as_deref(), Some(pkce.challenge()));
        assert_eq!(get("state").as_deref(), Some(pkce.state()));
        assert_eq!(
            get("callback_url").as_deref(),
            Some("http://127.0.0.1:51733/cb")
        );
        assert_eq!(get("scope").as_deref(), Some("api"));
        // The verifier must never ride the URL.
        assert!(!url.as_str().contains(pkce.verifier()));
    }

    #[test]
    fn authorize_url_honours_an_explicit_self_hosted_base() {
        let pkce = Pkce::new();
        let url = authorize_url("https://orca.internal.example", &pkce, "oob").unwrap();
        assert!(url.starts_with("https://orca.internal.example/auth?"));
    }

    #[test]
    fn exchange_error_message_carries_no_credential_material() {
        let error = ExchangeError {
            status: 403,
            detail: "invalid code".to_owned(),
        };
        let rendered = format!("{error}");
        assert!(rendered.contains("403"));
        assert!(error.is_actionable());
        assert!(
            !ExchangeError {
                status: 500,
                detail: String::new()
            }
            .is_actionable()
        );
    }
}
