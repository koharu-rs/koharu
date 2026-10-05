mod auth;
mod catalog;
mod protocol;
mod stream;
mod token_store;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use std::{
    fmt,
    time::{Duration, Instant},
};

use reqwest::{
    Client, StatusCode,
    header::{CONTENT_TYPE, RETRY_AFTER},
};
use serde::Serialize;
use specta::Type;

use crate::{Control, Reasoning};

pub use auth::Account;
use auth::Auth;
pub(crate) use protocol::{Request, function_output, message, project_context};
pub(crate) use stream::{Delta, Turn};

const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub(crate) const MAX_ATTEMPTS: usize = 3;

/// A response the provider refused, kept as data so the retry and split decisions
/// read it where the status and body are still known, instead of re-guessing them
/// from formatted message text far downstream.
#[derive(Debug)]
pub(crate) struct ProviderRefusal {
    status: Option<StatusCode>,
    body: String,
}

impl fmt::Display for ProviderRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(formatter, "Codex returned {status}: {}", self.body),
            None => write!(formatter, "{}", self.body),
        }
    }
}

impl std::error::Error for ProviderRefusal {}

impl ProviderRefusal {
    /// A non-2xx HTTP response.
    fn http(status: StatusCode, body: String) -> anyhow::Error {
        anyhow::Error::new(Self {
            status: Some(status),
            body,
        })
    }

    /// A failure carried by the event stream, which has no status to wait on.
    fn stream(context: &str, message: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(Self {
            status: None,
            body: format!("{context}: {}", message.into()),
        })
    }

    /// Only an HTTP status the caller can wait on is retryable; a stream failure
    /// has already been sent and carries no `Retry-After`.
    fn transient(&self) -> bool {
        self.status.is_some_and(is_transient_status)
    }

    /// Only a batch split answers an overflow, so it is recognised from the narrow
    /// phrasings that name the limit itself — never from an auth or quota message
    /// that happens to mention a token.
    fn context_limit(&self) -> bool {
        self.status.is_none_or(|status| status.is_client_error())
            && mentions_context_limit(&self.body)
    }
}

/// An event stream that ended before its completion event. A truncated SSE body
/// has no reqwest error kind, so it carries its own type to stay retryable.
#[derive(Debug)]
pub(crate) struct StreamTruncated;

impl fmt::Display for StreamTruncated {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Codex response stream ended before completion")
    }
}

impl std::error::Error for StreamTruncated {}

/// The provider reports a context overflow as ordinary failure prose.
fn mentions_context_limit(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    [
        "context window",
        "maximum context",
        "prompt is too long",
        "exceeds the context",
    ]
    .iter()
    .any(|phrase| body.contains(phrase))
}

/// A refused HTTP response, as an error the retry loop can read.
pub(crate) fn response_error(status: StatusCode, body: String) -> anyhow::Error {
    ProviderRefusal::http(status, body)
}

/// A failure reported inside the stream, classified the same way as a refused HTTP
/// status so an overflow reaches the batch splitter either way.
pub(crate) fn provider_failure(context: &str, message: impl Into<String>) -> anyhow::Error {
    ProviderRefusal::stream(context, message)
}

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
        let mut attempt = 0_usize;
        loop {
            control.ensure_running()?;
            let (response, payload_bytes) = match self.send_with_auth(request, control).await {
                Ok(sent) => sent,
                Err(error) if should_retry(attempt, &error, false) => {
                    retry_wait(backoff(attempt), control).await?;
                    attempt = attempt.saturating_add(1);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if !response.status().is_success() {
                let status = response.status();
                let retry_after = retry_after(&response);
                let mut body = response.text().await.unwrap_or_default();
                body.truncate(16 * 1024);
                let error = response_error(status, body);
                if should_retry(attempt, &error, false) {
                    retry_wait(retry_after.unwrap_or_else(|| backoff(attempt)), control).await?;
                    attempt = attempt.saturating_add(1);
                    continue;
                }
                return Err(error);
            }
            let sent_at = Instant::now();
            let mut published = false;
            let mut first_delta = None;
            let read = {
                let mut emit = |delta| {
                    published = true;
                    if first_delta.is_none() {
                        first_delta = Some(sent_at.elapsed());
                    }
                    publish(delta);
                };
                stream::read(response, control, &mut emit).await
            };
            match read {
                Ok(mut turn) => {
                    turn.payload_bytes = payload_bytes;
                    turn.first_delta_ms = first_delta
                        .unwrap_or_else(|| sent_at.elapsed())
                        .as_millis()
                        .try_into()
                        .unwrap_or(u32::MAX);
                    log_response(request, attempt, &turn);
                    return Ok(turn);
                }
                Err(error) if should_retry(attempt, &error, published) => {
                    retry_wait(backoff(attempt), control).await?;
                    attempt = attempt.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Send once with token refresh on 401; no transient or status retry.
    pub(crate) async fn respond_once(&self, request: &Request, control: &Control) -> Result<Turn> {
        let (response, payload_bytes) = self.send_with_auth(request, control).await?;
        if !response.status().is_success() {
            let status = response.status();
            let mut body = response.text().await.unwrap_or_default();
            body.truncate(16 * 1024);
            return Err(response_error(status, body));
        }
        let sent_at = Instant::now();
        let mut publish = |_| {};
        let mut turn = stream::read(response, control, &mut publish).await?;
        turn.payload_bytes = payload_bytes;
        turn.first_delta_ms = sent_at.elapsed().as_millis().try_into().unwrap_or(u32::MAX);
        log_response(request, 0, &turn);
        Ok(turn)
    }

    /// Authorized single attempt with a single 401 refresh.
    async fn send_with_auth(
        &self,
        request: &Request,
        control: &Control,
    ) -> Result<(reqwest::Response, usize)> {
        let mut session = self.auth.session().await?;
        let (response, payload_bytes) = self.send(request, &session, control).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            session = self.auth.force_refresh().await?;
            return self.send(request, &session, control).await;
        }
        Ok((response, payload_bytes))
    }

    async fn send(
        &self,
        request: &Request,
        session: &auth::Session,
        control: &Control,
    ) -> Result<(reqwest::Response, usize)> {
        // Encoded here once so the exact wire size can be reported; `.json()`
        // would perform the same encode and throw the length away.
        let body = serde_json::to_vec(request).context("agent request must serialize")?;
        let payload_bytes = body.len();
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
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send();
        tokio::select! {
            response = send => Ok((response?, payload_bytes)),
            () = control.cancelled() => {
                control.ensure_running()?;
                unreachable!("cancelled control must fail ensure_running")
            }
        }
    }
}

/// One structured record per Codex response, so cost and latency are measured on
/// the chat loop and not only inside the review worker.
fn log_response(request: &Request, attempt: usize, turn: &Turn) {
    let usage = turn.usage.unwrap_or_default();
    tracing::info!(
        target: "koharu_agent::codex",
        model = %request.model,
        attempt = attempt,
        payload_bytes = turn.payload_bytes,
        first_delta_ms = turn.first_delta_ms,
        input_tokens = usage.input_tokens,
        cached_input_tokens = usage.cached_input_tokens,
        output_tokens = usage.output_tokens,
        reasoning_tokens = usage.reasoning_tokens,
        "codex response completed"
    );
}

#[async_trait]
impl ReviewClient for Codex {
    async fn respond_once(&self, request: &Request, control: &Control) -> Result<Turn> {
        self.respond_once(request, control).await
    }
}

/// Whether another attempt is worth sending. A turn whose text already reached the
/// user cannot be resent without showing it twice, so publication ends the budget
/// even when the failure looks transient.
pub(crate) fn should_retry(attempt: usize, error: &anyhow::Error, published: bool) -> bool {
    !published && is_transient_error(error) && attempt + 1 < MAX_ATTEMPTS
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
    error.chain().any(|source| {
        source
            .downcast_ref::<ProviderRefusal>()
            .is_some_and(ProviderRefusal::transient)
            || source.downcast_ref::<StreamTruncated>().is_some()
            || source
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| {
                    error.is_timeout()
                    || error.is_connect()
                    || error.is_body()
                    // A pooled keep-alive connection the server has already closed
                    // surfaces here as a failed send ("connection closed before message
                    // completed"), which reqwest reports as `Kind::Request`. Retrying
                    // gets a fresh connection, so this is the common transient case.
                    || error.is_request()
                })
    })
}

/// Whether this failure means the request itself must shrink.
pub(crate) fn is_context_limit(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<ProviderRefusal>()
            .is_some_and(ProviderRefusal::context_limit)
    })
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

    fn refusal(code: u16, body: &str) -> anyhow::Error {
        response_error(
            StatusCode::from_u16(code).expect("valid status"),
            body.to_owned(),
        )
    }

    #[test]
    fn classifies_transient_failures_by_type_not_prose() {
        for code in [429, 500, 502, 503, 504] {
            assert!(is_transient_error(&refusal(code, "try again later")));
        }
        assert!(is_transient_error(&anyhow::Error::new(StreamTruncated)));
        assert!(!is_transient_error(&refusal(401, "refresh token expired")));
        assert!(!is_transient_error(&anyhow!("invalid request")));
        // Text that used to be sniffed for is no longer a retry signal on its own.
        assert!(!is_transient_error(&anyhow!(
            "connection reset by peer (os error 10054)"
        )));
        // A stream failure has no status to wait on, so resending it would loop.
        assert!(!is_transient_error(&provider_failure(
            "Codex response failed",
            "the model backend is overloaded",
        )));
    }

    #[tokio::test]
    async fn a_send_the_peer_dropped_is_transient() {
        // The real failure this has to classify: a pooled keep-alive connection the
        // server had already closed, reported while the request was being sent.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local listener");
        let url = format!(
            "http://{}/",
            listener.local_addr().expect("listener address")
        );
        let accept = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        let response = reqwest::Client::new()
            .post(&url)
            .body(vec![b'x'; 64 * 1024])
            .send()
            .await;
        accept.abort();
        let error = anyhow::Error::new(response.expect_err("a dropped peer must fail the send"));
        assert!(is_transient_error(&error), "{error:?} must be retryable");
        assert!(should_retry(0, &error, false));
    }

    #[test]
    fn marks_only_an_overflow_as_a_context_limit() {
        assert!(is_context_limit(&refusal(
            400,
            "This model's maximum context length is 8192 tokens",
        )));
        assert!(is_context_limit(&provider_failure(
            "Codex response failed",
            "your input exceeds the context window of this model",
        )));
        // An auth or quota refusal that merely mentions tokens must not split every
        // batch of a job that no smaller batch can solve.
        for (code, body) in [
            (429, "insufficient_quota: token budget exhausted"),
            (401, "refresh token reuse detected"),
            (400, "unknown field in tool schema"),
        ] {
            assert!(
                !is_context_limit(&refusal(code, body)),
                "{code} {body} is not a context limit"
            );
        }
    }

    #[test]
    fn spends_its_attempts_and_never_reshows_published_text() {
        let rate_limited = refusal(429, "try again later");
        assert!(should_retry(0, &rate_limited, false));
        assert!(should_retry(1, &rate_limited, false));
        assert!(!should_retry(2, &rate_limited, false));
        assert!(!should_retry(0, &rate_limited, true));
        assert!(!should_retry(0, &anyhow!("invalid request"), false));
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
