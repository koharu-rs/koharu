//! ChatGPT sign-in through rig's ChatGPT provider, the route Codex uses,
//! with rig's record kept in Koharu's encrypted secrets.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use http::{HeaderValue, header::AUTHORIZATION, request::Parts};
use rig_core::{
    catalog::{Catalog, ModelSpec, ReasoningSupport},
    error::BoxError,
    providers::auth::{DeviceCodeHandler, DeviceCodePrompt, TokenStore},
    providers::chatgpt::auth::{self, AuthSource, Authenticator, SignInMethod},
    providers::registry::ProviderId,
    wasm_compat::WasmBoxedFuture,
};
use rig_reqwest::ReqwestClient;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

use super::{Http, SignIn, SignInPrompt};

pub(super) use rig_core::providers::chatgpt::PROVIDER_NAME;

/// rig's sign-in record, kept as a secret under the key rig names,
/// `chatgpt`.
struct Secrets;

impl TokenStore for Secrets {
    fn load<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<Option<String>, BoxError>> {
        Box::pin(async move {
            Ok(koharu_config::secrets::get(key)?.map(|record| record.expose_secret().to_owned()))
        })
    }

    fn save<'a>(
        &'a self,
        key: &'a str,
        record: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            Ok(koharu_config::secrets::set(
                key,
                &SecretString::from(record),
            )?)
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move { Ok(koharu_config::secrets::delete(key)?) })
    }
}

fn authenticator(device_code: DeviceCodeHandler) -> Authenticator {
    Authenticator::new(AuthSource::OAuth, Secrets, device_code, true).with_app_name("Koharu")
}

/// Signs in with the user's ChatGPT plan in the system browser, or with a
/// device code when the browser's sign-in can't listen for its answer.
/// `prompt` hears what the user must do.
pub(super) async fn sign_in(prompt: impl Fn(SignInPrompt) + Send + Sync + 'static) -> Result<()> {
    let prompt = Arc::new(prompt);
    let device_code = Arc::clone(&prompt);
    authenticator(DeviceCodeHandler::new(move |code: DeviceCodePrompt| {
        device_code(SignInPrompt {
            url: code.verification_uri,
            opened: false,
            code: Some(code.user_code),
        });
    }))
    .sign_in(
        &Http::default(),
        SignInMethod::Browser,
        |asked| match asked {
            auth::SignInPrompt::Browser(page) => prompt(SignInPrompt {
                url: page.authorize_url,
                opened: page.browser_launched,
                code: None,
            }),
            auth::SignInPrompt::BrowserUnavailable(reason) => {
                tracing::info!(
                    "ChatGPT's browser sign-in can't listen, so a device code follows: {reason}"
                );
            }
        },
    )
    .await?;
    Ok(())
}

/// Signs out: ChatGPT revokes the tokens, and the record is deleted.
pub(super) async fn sign_out() -> Result<()> {
    authenticator(DeviceCodeHandler::default())
        .sign_out(&Http::default())
        .await?;
    Ok(())
}

/// The signed-in plan's credential and account, on each request.
struct Plan(Authenticator);

impl SignIn for Plan {
    fn authorize<'a>(&'a self, parts: &'a mut Parts) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let credential = self.0.auth_context(&ReqwestClient::default()).await?;
            let bearer = format!("Bearer {}", credential.api_key.expose());
            parts
                .headers
                .insert(AUTHORIZATION, HeaderValue::try_from(bearer)?);
            if let Some(account) = credential.account_id {
                parts
                    .headers
                    .insert("chatgpt-account-id", HeaderValue::try_from(account)?);
            }
            Ok(())
        })
    }
}

/// The HTTP client for the signed-in plan's models.
pub(super) fn http() -> Result<Http> {
    if koharu_config::secrets::get(PROVIDER_NAME)?.is_none() {
        bail!("ChatGPT isn't signed in");
    }
    Ok(Http::signed_in(Plan(authenticator(
        DeviceCodeHandler::default(),
    ))))
}

/// Codex's model list for the signed-in plan, or none when nobody signed in,
/// each as rig's catalog describes it, and as Codex's list does when the
/// catalog doesn't.
pub(super) async fn models(client: &reqwest::Client) -> Result<Vec<ModelSpec>> {
    if koharu_config::secrets::get(PROVIDER_NAME)?.is_none() {
        return Ok(Vec::new());
    }
    let context = authenticator(DeviceCodeHandler::default())
        .auth_context(&Http::default())
        .await?;
    // Codex sends its own release as `client_version`, which the list requires,
    // and the list offers only the models that release can drive, so this
    // follows Codex's current release.
    let mut request = client
        .get("https://chatgpt.com/backend-api/codex/models?client_version=0.170.0")
        .bearer_auth(context.api_key.expose());
    if let Some(account) = &context.account_id {
        request = request.header("chatgpt-account-id", account);
    }
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        bail!("Codex's model list answered {status}");
    }
    let body: Value = response.json().await?;
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .context("Codex's model list has no `models` array")?;
    let vendor = ProviderId::catalog(PROVIDER_NAME).context("rig knows no ChatGPT vendor")?;
    let catalog = Catalog::builtin();
    Ok(models
        .iter()
        .filter(|model| {
            let listed = model
                .get("visibility")
                .and_then(Value::as_str)
                .is_none_or(|visibility| visibility.eq_ignore_ascii_case("list"));
            listed && model.get("show_in_picker").and_then(Value::as_bool) != Some(false)
        })
        .filter_map(|model| {
            let id = model.get("slug")?.as_str()?;
            if let Some(found) = catalog.get(vendor, id) {
                return Some(found.spec.clone());
            }
            let mut spec = ModelSpec::new(vendor, id).with_tools(true);
            if let Some(name) = model.get("display_name").and_then(Value::as_str) {
                spec = spec.with_display_name(name);
            }
            let reasons = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .is_some_and(|levels| !levels.is_empty());
            if reasons {
                spec = spec.with_reasoning(ReasoningSupport::Unknown { default: None });
            }
            spec.input.image = true;
            Some(spec)
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
        let plan = Plan(Authenticator::new(
            AuthSource::AccessToken {
                access_token: "token".to_owned(),
                account_id: Some("account".to_owned()),
            },
            Secrets,
            DeviceCodeHandler::default(),
            true,
        ));
        let (mut parts, ()) = http::Request::post("https://chatgpt.com")
            .header(AUTHORIZATION, "Bearer ")
            .body(())
            .unwrap()
            .into_parts();
        plan.authorize(&mut parts).await.unwrap();
        assert_eq!(parts.headers[AUTHORIZATION], "Bearer token");
        assert_eq!(parts.headers["chatgpt-account-id"], "account");
    }
}
