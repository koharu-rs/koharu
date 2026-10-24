mod auth;
mod catalog;
mod protocol;
mod stream;
mod token_store;

use anyhow::{Result, bail};
use async_trait::async_trait;
use std::time::Duration;

use reqwest::{Client, StatusCode, header::RETRY_AFTER};
use serde::Serialize;
use specta::Type;

use crate::{Control, Reasoning};

pub use auth::Account;
use auth::Auth;
pub(crate) use protocol::{Request, function_output, message, project_context};
pub(crate) use stream::{Delta, Turn};

const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub(crate) const MAX_ATTEMPTS: usize = 3;

/// Single-shot model client used by the bulk review worker. Implementations
/// must not retry internally so the caller controls attempt accounting.
#[async_trait]
pub(crate) trait ReviewClient: Send + Sync {
    async fn respond_once(&self, request: &Request, control: &Control) -> Result<Turn>;
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct CodexModel {
    pub id: String,
    pub name: String,
    pub reasoning: Vec<Reasoning>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoginEvent {
    Progress {
        message: String,
    },
    DeviceCode {
        verification_url: String,
        user_code: String,
    },
}

#[derive(Clone, Debug)]
pub struct Codex {
    client: Client,
    auth: Auth,
}

impl Codex {
    pub fn new() -> Result<Self> {
        let client = koharu_runtime::http_client()?;
        Ok(Self {
            auth: Auth::new(client.clone()),
            client,
        })
    }

    pub fn account(&self) -> Result<Option<Account>> {
        self.auth.account()
    }

    #[tracing::instrument(skip_all)]
    pub async fn login_device<F>(&self, control: &Control, publish: F) -> Result<Account>
    where
        F: FnMut(LoginEvent),
    {
        self.auth.login_device(control, publish).await
    }

    pub fn logout(&self) -> Result<()> {
        self.auth.logout()
    }

    #[tracing::instrument(skip_all)]
    pub async fn models(&self) -> Result<Vec<CodexModel>> {
        catalog::models(&self.client, &self.auth).await
    }

    pub(crate) async fn respond<F>(
        &self,
        request: &Request,
        control: &Control,
        mut publish: F,
    ) -> Result<Turn>
    where
        F: FnMut(Delta),
    {
        for attempt in 0..MAX_ATTEMPTS {
            control.ensure_running()?;
            let response = match self.send_with_auth(request, control).await {
                Ok(response) => response,
                Err(error) if should_retry(attempt, is_transient_error(&error)) => {
                    retry_wait(backoff(attempt), control).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if !response.status().is_success() {
                let status = response.status();
                let retry_after = retry_after(&response);
                let mut body = response.text().await.unwrap_or_default();
                body.truncate(16 * 1024);
                if should_retry(attempt, is_transient_status(status)) {
                    retry_wait(retry_after.unwrap_or_else(|| backoff(attempt)), control).await?;
                    continue;
                }
                bail!("Codex returned {status}: {body}");
            }
            match stream::read(response, control, &mut publish).await {
                Ok(turn) => return Ok(turn),
                Err(error) if should_retry(attempt, is_transient_error(&error)) => {
                    retry_wait(backoff(attempt), control).await?;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("retry loop returns on its final attempt")
    }

    /// Send once with token refresh on 401; no transient/status retry.
    pub(crate) async fn respond_once(&self, request: &Request, control: &Control) -> Result<Turn> {
        let response = self.send_with_auth(request, control).await?;
        if !response.status().is_success() {
            let status = response.status();
            let mut body = response.text().await.unwrap_or_default();
            body.truncate(16 * 1024);
            bail!("Codex returned {status}: {body}");
        }
        let mut publish = |_| {};
        stream::read(response, control, &mut publish).await
    }

    /// Authorized single attempt with a single 401 refresh.
    async fn send_with_auth(
        &self,
        request: &Request,
        control: &Control,
    ) -> Result<reqwest::Response> {
        let mut session = self.auth.session().await?;
        let response = self.send(request, &session, control).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            session = self.auth.force_refresh().await?;
            return self.send(request, &session, control).await;
        }
        Ok(response)
    }

    async fn send(
        &self,
        request: &Request,
        session: &auth::Session,
        control: &Control,
    ) -> Result<reqwest::Response> {
        let request_id = request.prompt_cache_key.clone();
        let send = self
            .client
            .post(RESPONSES_URL)
            .bearer_auth(&session.access)
            .header("chatgpt-account-id", &session.account.id)
            .header("originator", "koharu")
            .header("OpenAI-Beta", "responses=experimental")
            .header("accept", "text/event-stream")
            .header("session_id", &request_id)
            .header("x-client-request-id", request_id)
            .json(request)
            .send();
        tokio::select! {
            response = send => Ok(response?),
            () = control.cancelled() => {
                control.ensure_running()?;
                unreachable!("cancelled control must fail ensure_running")
            }
        }
    }
}

#[async_trait]
impl ReviewClient for Codex {
    async fn respond_once(&self, request: &Request, control: &Control) -> Result<Turn> {
        self.respond_once(request, control).await
    }
}

pub(crate) fn should_retry(attempt: usize, transient: bool) -> bool {
    transient && attempt + 1 < MAX_ATTEMPTS
}

pub(crate) fn backoff(attempt: usize) -> Duration {
    Duration::from_secs([2, 5].get(attempt).copied().unwrap_or(5))
}

pub(crate) fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(|seconds| Duration::from_secs(seconds.min(60)))
}

pub(crate) fn is_transient_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || matches!(status.as_u16(), 500 | 502 | 503 | 504)
}

pub(crate) fn is_transient_error(error: &anyhow::Error) -> bool {
    if error.chain().any(|source| {
        source
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout() || error.is_connect() || error.is_body())
    }) {
        return true;
    }
    let message = format!("{error:#}").to_ascii_lowercase();
    [
        "connection reset",
        "connection error",
        "os error 10054",
        "timed out",
        "response stream ended before completion",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

pub(crate) async fn retry_wait(duration: Duration, control: &Control) -> Result<()> {
    tokio::select! {
        () = tokio::time::sleep(duration) => Ok(()),
        () = control.cancelled() => {
            control.ensure_running()?;
            unreachable!("cancelled control must fail ensure_running")
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn retries_only_transient_failures_with_a_fixed_limit() {
        assert!(is_transient_error(&anyhow!(
            "connection reset by peer (os error 10054)"
        )));
        assert!(!is_transient_error(&anyhow!("invalid request")));
        assert!(should_retry(0, true));
        assert!(should_retry(1, true));
        assert!(!should_retry(2, true));
        assert!(!should_retry(0, false));
    }

    #[test]
    fn retries_only_supported_http_statuses() {
        for status in [429, 500, 502, 503, 504] {
            assert!(is_transient_status(StatusCode::from_u16(status).unwrap()));
        }
        for status in [400, 401, 403, 404] {
            assert!(!is_transient_status(StatusCode::from_u16(status).unwrap()));
        }
    }
}
