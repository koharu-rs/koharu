//! The HTTP client hosted models go through: rig's own, shared by the process,
//! sending a 429 or 5xx reply again, up to three times, after the wait rig reads
//! from it, or a second, then two, then four when it names none; rig itself
//! retries nothing. For ChatGPT, it also puts the signed-in plan's credential
//! on each request, so a long-lived model outlives any one token.

use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use backon::{ExponentialBuilder, Retryable};
use bytes::Bytes;
use http::{HeaderValue, header::AUTHORIZATION, request::Parts};
use rig_core::{
    error::ProviderError,
    http_client::{self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse},
    providers::chatgpt::auth::Authenticator,
};
use rig_reqwest::ReqwestClient;

#[derive(Clone, Default)]
pub(crate) struct Http {
    client: ReqwestClient,
    sign_in: Option<Authenticator>,
}

/// A second, then two, then four.
const BACKOFF: ExponentialBuilder = ExponentialBuilder::new();

/// rig's own client, for the requests Koharu makes outside rig.
pub(crate) fn reqwest() -> Result<reqwest::Client> {
    ReqwestClient::default()
        .inner()
        .cloned()
        .context("couldn't build the HTTP client")
}

/// Whether `error` is a 429 or 5xx reply, worth sending again.
fn retryable(error: &http_client::Error) -> bool {
    matches!(
        error,
        http_client::Error::InvalidStatusCodeWithDetails { status, .. }
            if status.as_u16() == 429 || status.is_server_error()
    )
}

/// The wait `error` names in place of `backoff`'s, until `backoff` runs out.
fn retry_after(error: &http_client::Error, backoff: Option<Duration>) -> Option<Duration> {
    let backoff = backoff?;
    let http_client::Error::InvalidStatusCodeWithDetails {
        status,
        body,
        headers,
    } = error
    else {
        return Some(backoff);
    };
    let report = ProviderError::from(http_client::Error::InvalidStatusCodeWithDetails {
        status: *status,
        body: body.clone(),
        headers: headers.clone(),
    })
    .report();
    Some(report.retry_after(SystemTime::now()).unwrap_or(backoff))
}

impl Http {
    /// A client whose requests carry `sign_in`'s credential.
    pub(crate) fn signed_in(sign_in: Authenticator) -> Self {
        Self {
            client: ReqwestClient::default(),
            sign_in: Some(sign_in),
        }
    }

    /// `parts` with the sign-in's current credential, refreshed by rig when it
    /// has expired.
    #[expect(clippy::result_large_err, reason = "rig's HTTP error is rig's")]
    async fn authorized(&self, mut parts: Parts) -> http_client::Result<Parts> {
        let Some(sign_in) = &self.sign_in else {
            return Ok(parts);
        };
        let credential = sign_in
            .auth_context(&self.client)
            .await
            .map_err(|error| http_client::Error::Instance(error.into()))?;
        let bearer = format!("Bearer {}", credential.api_key.expose());
        parts
            .headers
            .insert(AUTHORIZATION, HeaderValue::try_from(bearer)?);
        if let Some(account) = credential.account_id {
            parts
                .headers
                .insert("chatgpt-account-id", HeaderValue::try_from(account)?);
        }
        Ok(parts)
    }
}

impl HttpClientExt for Http {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let http = self.clone();
        let (parts, body) = request.into_parts();
        let body: Bytes = body.into();
        async move {
            (|| async {
                let parts = http.authorized(parts.clone()).await?;
                http.client
                    .send(http::Request::from_parts(parts, body.clone()))
                    .await
            })
            .retry(BACKOFF)
            .when(retryable)
            .adjust(retry_after)
            .await
        }
    }

    fn send_multipart<U>(
        &self,
        request: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        self.client.send_multipart(request)
    }

    fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let http = self.clone();
        let (parts, body) = request.into_parts();
        let body: Bytes = body.into();
        async move {
            (|| async {
                let parts = http.authorized(parts.clone()).await?;
                http.client
                    .send_streaming(http::Request::from_parts(parts, body.clone()))
                    .await
            })
            .retry(BACKOFF)
            .when(retryable)
            .adjust(retry_after)
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use rig_core::providers::chatgpt::auth::{AuthSource, DeviceCodeHandler};

    use super::*;

    /// The credential is read at each request, in place of the empty key the
    /// model was connected with.
    #[tokio::test]
    async fn a_signed_in_request_carries_the_current_credential() {
        let sign_in = Authenticator::new(
            AuthSource::AccessToken {
                access_token: "token".to_owned(),
                account_id: Some("account".to_owned()),
            },
            crate::chatgpt::Secrets,
            DeviceCodeHandler::default(),
            true,
        );
        let (parts, ()) = http::Request::post("https://chatgpt.com")
            .header(AUTHORIZATION, "Bearer ")
            .body(())
            .unwrap()
            .into_parts();
        let parts = Http::signed_in(sign_in).authorized(parts).await.unwrap();
        assert_eq!(parts.headers[AUTHORIZATION], "Bearer token");
        assert_eq!(parts.headers["chatgpt-account-id"], "account");
    }

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
