use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use koharu_secrets::{ExposeSecret as _, SecretString};
use reqwest::Client;
use serde::Serialize;
use specta::Type;
use tokio::sync::Mutex;

use super::pkce::{self, Pkce};

/// The secret slot the OrcaRouter API key lives in. The pasted-key flow and the
/// PKCE flow both land here, so downstream code never learns which one was used.
const KEY: &str = "orcarouter";
const GENERATION_KEY: &str = "orcarouter_generation";
const REAUTH_KEY: &str = "orcarouter_needs_reauth";

/// How a credential was obtained. Used for status UI only — never for routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    /// The user pasted an `sk-orca-…` key.
    ApiKey,
    /// The user authorized this app in a browser.
    Authorization,
}

/// A plain OrcaRouter API key plus where it came from.
#[derive(Clone, Serialize, Type)]
pub struct Credential {
    #[serde(skip)]
    pub key: SecretString,
    pub source: CredentialSource,
    /// Which stored credential this is. A late `401` may only invalidate the
    /// generation that actually made the rejected request.
    pub generation: String,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credential")
            .field("key", &"[REDACTED]")
            .field("source", &self.source)
            .field("generation", &self.generation)
            .finish()
    }
}

/// The one place credentials enter the provider. Each way of obtaining a key is
/// an adapter over this trait; the provider, the model catalog and every AI
/// entry point consume [`Credential`] and cannot tell them apart.
#[async_trait::async_trait]
pub trait CredentialProvider: Send + Sync {
    async fn credential(&self) -> Result<Credential>;
}

/// Reads the key the user pasted into Settings → Providers.
pub struct ApiKeyAdapter;

#[async_trait::async_trait]
impl CredentialProvider for ApiKeyAdapter {
    async fn credential(&self) -> Result<Credential> {
        let key = koharu_secrets::get(KEY)?.context("OrcaRouter API key is not configured")?;
        if key.expose_secret().trim().is_empty() {
            bail!("OrcaRouter API key is not configured");
        }
        Ok(Credential {
            key,
            source: CredentialSource::ApiKey,
            generation: generation()?,
        })
    }
}

/// Runs OAuth 2.0 + PKCE in the browser and persists the issued key.
pub struct AuthorizationAdapter {
    client: Client,
    auth_base: String,
}

impl AuthorizationAdapter {
    #[must_use]
    pub fn new(client: Client, auth_base: impl Into<String>) -> Self {
        Self {
            client,
            auth_base: auth_base.into(),
        }
    }
}

#[async_trait::async_trait]
impl CredentialProvider for AuthorizationAdapter {
    async fn credential(&self) -> Result<Credential> {
        Ok(connect(&self.client, &self.auth_base).await?.0)
    }
}

/// Run Flow A: bind loopback, open the consent screen, redeem the code, persist.
///
/// Returns the credential and the granted scope so the caller can tell the user
/// when it was approved for less than requested.
pub async fn connect(client: &Client, auth_base: &str) -> Result<(Credential, Option<String>)> {
    let (listener, callback_url) = pkce::loopback_listener().await?;
    let attempt = Pkce::new();
    let url = pkce::authorize_url(auth_base, &attempt, &callback_url)?;

    // The caller owns presenting this URL; it contains no secret.
    tracing::info!(%url, "opening the OrcaRouter consent screen");
    open_browser(&url);

    let code = pkce::listen(listener, attempt.state(), pkce::TTL)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let issued = pkce::exchange(client, auth_base, &code, attempt.verifier()).await?;
    let credential = persist(
        SecretString::from(issued.key),
        CredentialSource::Authorization,
    )?;
    Ok((credential, issued.scope))
}

/// Store a key under the shared slot and open a new credential generation.
///
/// The previous secret is only replaced once the new one is safely stored, so a
/// failed reauthorization cannot destroy a working credential.
pub fn persist(key: SecretString, source: CredentialSource) -> Result<Credential> {
    let generation = uuid::Uuid::new_v4().simple().to_string();
    koharu_secrets::set(KEY, &key)?;
    koharu_secrets::set(GENERATION_KEY, &SecretString::from(generation.clone()))?;
    koharu_secrets::delete(REAUTH_KEY)?;
    Ok(Credential {
        key,
        source,
        generation,
    })
}

/// Mark the exact credential generation whose request the relay rejected.
///
/// A late failure from an older request must never mark a newer generation, and
/// a durable key is never refreshed — it requires the user to reconnect.
pub fn mark_needs_reauth(generation: &str) -> Result<()> {
    koharu_secrets::set(REAUTH_KEY, &SecretString::from(generation.to_owned()))?;
    Ok(())
}

#[must_use]
pub fn needs_reauth() -> bool {
    koharu_secrets::get(REAUTH_KEY)
        .ok()
        .flatten()
        .is_some_and(|secret| !secret.expose_secret().is_empty())
}

/// The generation currently marked for reauthentication, if any.
pub fn reauth_generation() -> Option<String> {
    koharu_secrets::get(REAUTH_KEY)
        .ok()
        .flatten()
        .map(|secret| secret.expose_secret().to_owned())
        .filter(|value| !value.is_empty())
}

/// True when `401` handling should invalidate `credential`.
#[must_use]
pub fn invalidates(credential: &Credential) -> bool {
    reauth_generation().is_none_or(|stale| stale == credential.generation)
}

pub fn forget() -> Result<()> {
    koharu_secrets::delete(KEY)?;
    koharu_secrets::delete(GENERATION_KEY)?;
    koharu_secrets::delete(REAUTH_KEY)?;
    Ok(())
}

/// Credentials are stored in the OS keychain, so a generation only exists once a
/// key does. A pasted key has no generation yet; mint one on first read.
fn generation() -> Result<String> {
    match koharu_secrets::get(GENERATION_KEY)? {
        Some(generation) if !generation.expose_secret().is_empty() => {
            Ok(generation.expose_secret().to_owned())
        }
        _ => {
            let generation = uuid::Uuid::new_v4().simple().to_string();
            koharu_secrets::set(GENERATION_KEY, &SecretString::from(generation.clone()))?;
            Ok(generation)
        }
    }
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(error) = std::process::Command::new(opener).arg(url).spawn() {
        // Not fatal: the UI also shows the URL to copy.
        tracing::warn!(%error, "failed to open a browser for OrcaRouter authorization");
    }
}

/// A login lock so two Connect clicks cannot race each other's callbacks.
#[derive(Clone, Default)]
pub struct LoginLock {
    held: Arc<Mutex<()>>,
}

impl LoginLock {
    /// Run `login`, releasing the lock on success, denial, error and timeout.
    pub async fn run<F, T>(&self, login: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        let _held = self.held.lock().await;
        login.await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orcarouter::tests::store_guard;

    #[tokio::test]
    async fn api_key_adapter_reports_the_pasted_key() {
        let _guard = store_guard();
        let _ = forget();
        koharu_secrets::set(KEY, &SecretString::from("sk-orca-test-pasted")).unwrap();

        let credential = ApiKeyAdapter.credential().await.unwrap();
        assert_eq!(credential.source, CredentialSource::ApiKey);
        assert_eq!(credential.key.expose_secret(), "sk-orca-test-pasted");
        let _ = forget();
    }

    #[tokio::test]
    async fn api_key_adapter_fails_closed_without_a_key() {
        let _guard = store_guard();
        let _ = forget();
        assert!(ApiKeyAdapter.credential().await.is_err());
    }

    #[test]
    fn both_adapters_share_one_credential_type_and_one_storage_slot() {
        let _guard = store_guard();
        let _ = forget();
        // Persisting an authorization-issued key and reading it back through the
        // API-key adapter yields the same shape, from the same slot.
        let issued = persist(
            SecretString::from("sk-orca-test-issued"),
            CredentialSource::Authorization,
        )
        .unwrap();
        assert_eq!(issued.source, CredentialSource::Authorization);

        let read_back = koharu_secrets::get(KEY).unwrap().unwrap();
        assert_eq!(read_back.expose_secret(), "sk-orca-test-issued");
        let _ = forget();
    }

    #[test]
    fn key_is_absent_from_credential_debug_output() {
        let _guard = store_guard();
        let credential = Credential {
            key: SecretString::from("sk-orca-test-secret"),
            source: CredentialSource::ApiKey,
            generation: "g1".to_owned(),
        };
        assert!(!format!("{credential:?}").contains("sk-orca-test-secret"));
    }

    #[test]
    fn credential_is_serialized_without_its_key() {
        let _guard = store_guard();
        let credential = Credential {
            key: SecretString::from("sk-orca-test-secret"),
            source: CredentialSource::Authorization,
            generation: "g1".to_owned(),
        };
        let rendered = serde_json::to_string(&credential).unwrap();
        assert!(!rendered.contains("sk-orca-test-secret"));
        assert!(rendered.contains("authorization"));
    }

    #[test]
    fn a_rejected_generation_does_not_disable_a_newer_one() {
        let _guard = store_guard();
        let _ = forget();
        let older = Credential {
            key: SecretString::from("sk-orca-old"),
            source: CredentialSource::ApiKey,
            generation: "old".to_owned(),
        };
        mark_needs_reauth("old").unwrap();
        let newer = Credential {
            key: SecretString::from("sk-orca-new"),
            source: CredentialSource::Authorization,
            generation: "new".to_owned(),
        };

        // A 401 that arrives for the old generation is not allowed to touch the
        // credential that just replaced it.
        assert!(invalidates(&older));
        assert!(!invalidates(&newer));
        assert!(needs_reauth());
        let _ = forget();
    }

    #[test]
    fn reauthorizing_clears_the_reauth_marker() {
        let _guard = store_guard();
        let _ = forget();
        mark_needs_reauth("stale").unwrap();
        assert!(needs_reauth());
        persist(
            SecretString::from("sk-orca-fresh"),
            CredentialSource::Authorization,
        )
        .unwrap();
        assert!(!needs_reauth());
        let _ = forget();
    }

    #[tokio::test]
    async fn login_lock_is_released_after_a_failed_login() {
        let lock = LoginLock::default();
        let failed = lock
            .run(async { Err::<(), _>(anyhow::anyhow!("denied")) })
            .await;
        assert!(failed.is_err());
        // A denial must not leave the lock held: a second attempt still runs.
        let second = lock.run(async { Ok::<_, anyhow::Error>(7) }).await;
        assert_eq!(second.unwrap(), 7);
    }
}
