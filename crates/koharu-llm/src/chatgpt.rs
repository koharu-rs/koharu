//! ChatGPT sign-in through rig's ChatGPT provider, the route Codex uses,
//! with rig's record kept in Koharu's encrypted secrets.

use anyhow::{Context, Result, bail};
use rig_core::{
    driver::DynModel,
    error::BoxError,
    operation::Completion,
    providers::{
        chatgpt::{
            self,
            auth::{
                AuthSource, Authenticator, DeviceCodeHandler, DeviceCodePrompt, SignInMethod,
                SignInPrompt, TokenStore,
            },
        },
        openai::OpenAIConfig,
    },
    wasm_compat::WasmBoxedFuture,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

use crate::{ModelInfo, Provider, hosted::Http};

/// rig's sign-in record, kept as a secret under the key rig names,
/// `chatgpt`.
struct Secrets;

impl TokenStore for Secrets {
    fn load<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<Option<String>, BoxError>> {
        Box::pin(async move {
            Ok(koharu_config::secret(key)?.map(|record| record.expose_secret().to_owned()))
        })
    }

    fn save<'a>(
        &'a self,
        key: &'a str,
        record: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move { Ok(koharu_config::set_secret(key, &SecretString::from(record))?) })
    }

    fn delete<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move { Ok(koharu_config::delete_secret(key)?) })
    }
}

fn authenticator(device_code: DeviceCodeHandler) -> Authenticator {
    Authenticator::new(AuthSource::OAuth, Secrets, device_code, true)
}

/// Signs in with the user's ChatGPT plan: in the system browser, or with a
/// device code where no browser opens. `prompt` hears what the user must
/// do, and `device_code` gets the code to enter.
pub async fn sign_in(
    prompt: impl FnMut(SignInPrompt) + Send,
    device_code: impl Fn(DeviceCodePrompt) + Send + Sync + 'static,
) -> Result<()> {
    authenticator(DeviceCodeHandler::new(device_code))
        .sign_in(&Http::new()?, SignInMethod::Browser, prompt)
        .await?;
    Ok(())
}

/// Signs out: ChatGPT revokes the tokens, and the record is deleted.
pub async fn sign_out() -> Result<()> {
    authenticator(DeviceCodeHandler::default())
        .sign_out(&Http::new()?)
        .await?;
    Ok(())
}

/// `id` with the signed-in plan's current token, refreshed by rig when it
/// has expired.
pub(crate) async fn model(id: &str, http: Http) -> Result<DynModel<Completion>> {
    let context = authenticator(DeviceCodeHandler::default())
        .auth_context(&http)
        .await
        .context("ChatGPT isn't signed in")?;
    let mut config = OpenAIConfig::with_key(&chatgpt::DIALECT, context.api_key);
    config.account_id = context.account_id;
    Ok(config.connect(http).completion(id).erase())
}

/// Codex's model list for the signed-in plan, or none when nobody signed in.
pub(crate) async fn models(client: &reqwest::Client) -> Result<Vec<ModelInfo>> {
    if koharu_config::secret(chatgpt::PROVIDER_NAME)?.is_none() {
        return Ok(Vec::new());
    }
    let context = authenticator(DeviceCodeHandler::default())
        .auth_context(&Http::new()?)
        .await?;
    let mut request = client
        .get("https://chatgpt.com/backend-api/codex/models?client_version=0.146.0")
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
            let id = model.get("slug")?.as_str()?.to_owned();
            let reasoning = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .is_some_and(|levels| !levels.is_empty());
            Some(ModelInfo {
                provider: Provider::ChatGpt,
                name: model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_owned(),
                id,
                quantizations: Vec::new(),
                images: true,
                tools: true,
                reasoning,
                json: Provider::ChatGpt.json(),
                languages: None,
            })
        })
        .collect())
}
