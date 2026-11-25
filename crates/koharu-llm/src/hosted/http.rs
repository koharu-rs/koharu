//! The HTTP client hosted models go through: rig's own, shared by the process,
//! sending a request that failed for now again, after the wait rig reads from
//! its reply, or a growing one when it names none; rig itself retries nothing.
//! For a vendor the user signs in to, it also puts the sign-in's current
//! credential on each request, so a long-lived model outlives any one token.

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use backon::{ExponentialBuilder, Retryable};
use bytes::Bytes;
use futures::future::BoxFuture;
use http::{Method, request::Parts};
use rig_core::{
    error::{ErrorReport, ProviderError, RateLimits},
    http_client::{self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse},
};
use rig_reqwest::ReqwestClient;

/// What a vendor the user signs in to puts on each request: the sign-in's
/// credential, refreshed first when it has expired, and whatever else the
/// vendor's endpoint requires of a signed-in request.
pub(crate) trait SignIn: Send + Sync {
    fn authorize<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<()>>;
}

#[derive(Clone, Default)]
pub(crate) struct Http {
    client: ReqwestClient,
    sign_in: Option<Arc<dyn SignIn>>,
}

/// A reply is asked for ten times, a second apart, then two, and so on to
/// thirty: an overloaded provider takes minutes to recover, and Gemini CLI asks
/// as often and waits as long (`DEFAULT_RETRY_OPTIONS`,
/// packages/core/src/utils/retry.ts at 60f8082b).
const REPLY: ExponentialBuilder = ExponentialBuilder::new()
    .with_max_delay(Duration::from_secs(30))
    .with_max_times(9);

/// A model list is asked for four times, as the interface waits for it.
const LIST: ExponentialBuilder = ExponentialBuilder::new();

/// rig's own client, for the requests Koharu makes outside rig.
pub(crate) fn reqwest() -> Result<reqwest::Client> {
    ReqwestClient::default()
        .inner()
        .cloned()
        .context("couldn't build the HTTP client")
}

/// The longest wait a request is sent again after, as OpenAI's Python library
/// has it; a reply that asks for more fails with what it says.
const LONGEST_WAIT: Duration = Duration::from_secs(120);

/// The reply `error` holds, when the vendor answered, as rig reads it.
fn reply(error: &http_client::Error) -> Option<ErrorReport> {
    let http_client::Error::InvalidStatusCodeWithDetails {
        status,
        body,
        headers,
    } = error
    else {
        return None;
    };
    let error =
        http_client::Error::non_success_with_details(*status, headers.clone(), body.clone());
    Some(ProviderError::from(error).report())
}

/// Whether a request that failed with `error` may pass sent again: a reply rig
/// calls retryable, or a connection lost before one. A server that couldn't be
/// reached, such as a local one that isn't running, isn't tried again.
fn retryable(error: &http_client::Error) -> bool {
    if let Some(reply) = reply(error) {
        return reply.is_retryable();
    }
    let http_client::Error::Transient(source) = error else {
        return false;
    };
    !source
        .downcast_ref::<reqwest::Error>()
        .is_some_and(reqwest::Error::is_connect)
}

/// The wait before a request that failed with `error` is sent again: what its
/// reply asks for, else until a spent allowance resets, else `backoff`'s.
/// `None`, once `backoff` runs out or the wait is too long, sends it no more.
fn wait(error: &http_client::Error, backoff: Option<Duration>) -> Option<Duration> {
    let backoff = backoff?;
    let now = SystemTime::now();
    let asked = reply(error)
        .and_then(|reply| reply.retry_after(now))
        .or_else(|| reset(error, now))
        .filter(|wait| !wait.is_zero());
    match asked {
        Some(asked) => (asked <= LONGEST_WAIT).then_some(asked),
        None => Some(backoff),
    }
}

/// How long after `now` the allowances `error`'s rate-limit headers say are
/// spent reset. OpenRouter's say so in place of a wait.
fn reset(error: &http_client::Error, now: SystemTime) -> Option<Duration> {
    let limits = RateLimits::from_headers(error.non_success_headers()?, now);
    [limits.requests, limits.tokens]
        .into_iter()
        .flatten()
        .filter(|allowance| allowance.remaining == 0)
        .filter_map(|allowance| allowance.resets_in)
        .max()
}

impl Http {
    /// A client whose requests carry `sign_in`'s credential.
    pub(crate) fn signed_in(sign_in: impl SignIn + 'static) -> Self {
        Self {
            client: ReqwestClient::default(),
            sign_in: Some(Arc::new(sign_in)),
        }
    }

    /// `parts` as the sign-in authorizes them.
    #[expect(clippy::result_large_err, reason = "rig's HTTP error is rig's")]
    async fn authorized(&self, mut parts: Parts) -> http_client::Result<Parts> {
        if let Some(sign_in) = &self.sign_in {
            sign_in
                .authorize(&mut parts)
                .await
                .map_err(http_client::Error::permanent)?;
        }
        Ok(parts)
    }

    /// `send` of `parts` as the sign-in authorizes them, again while it fails
    /// for now.
    #[expect(clippy::result_large_err, reason = "rig's HTTP error is rig's")]
    async fn retried<T, F>(&self, parts: Parts, send: impl Fn(Parts) -> F) -> http_client::Result<T>
    where
        F: Future<Output = http_client::Result<T>>,
    {
        let backoff = match parts.method {
            Method::GET => LIST,
            _ => REPLY,
        };
        (|| async { send(self.authorized(parts.clone()).await?).await })
            .retry(backoff)
            .when(retryable)
            .adjust(wait)
            .notify(|error, wait| tracing::warn!("{error}; asking again in {wait:?}"))
            .await
    }
}

impl HttpClientExt for Http {
    fn send(
        &self,
        request: http::Request<Bytes>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody>>> + Send + 'static {
        let http = self.clone();
        let (parts, body) = request.into_parts();
        async move {
            http.retried(parts, |parts| {
                http.client
                    .send(http::Request::from_parts(parts, body.clone()))
            })
            .await
        }
    }

    fn send_multipart(
        &self,
        request: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody>>> + Send + 'static {
        self.client.send_multipart(request)
    }

    fn send_streaming(
        &self,
        request: http::Request<Bytes>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send {
        let http = self.clone();
        let (parts, body) = request.into_parts();
        async move {
            http.retried(parts, |parts| {
                http.client
                    .send_streaming(http::Request::from_parts(parts, body.clone()))
            })
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use http::header::AUTHORIZATION;

    use super::*;

    #[tokio::test]
    async fn another_providers_request_is_left_alone() {
        let (parts, ()) = http::Request::post("https://example.com")
            .header(AUTHORIZATION, "Bearer key")
            .body(())
            .unwrap()
            .into_parts();
        let parts = Http::default().authorized(parts).await.unwrap();
        assert_eq!(parts.headers[AUTHORIZATION], "Bearer key");
    }
}
