// Ported from:
// https://github.com/koharu-rs/koharu/blob/f4ce03999ed1ae2faaec938dd52c2f41a87d03d9/crates/koharu-llm/src/providers/google_translate.rs

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use super::send_json;
use crate::{Model, Provider, Result, TranslationRequest};

const URL: &str = "https://translation.googleapis.com/language/translate/v2";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct GoogleCloudConfig {}

pub(super) async fn models() -> Result<Vec<Model>> {
    Ok(
        if koharu_secrets::get("google-cloud-translation")?.is_some() {
            vec![Model::service(
                Provider::GoogleCloudTranslation,
                "Google Cloud Translation",
            )]
        } else {
            Vec::new()
        },
    )
}

pub(super) async fn translate(
    client: &Client,
    _config: &GoogleCloudConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("google-cloud-translation")?
        .context("google-cloud-translation API key is not configured")?;
    let mut url = Url::parse(URL).expect("Google API URL is valid");
    url.query_pairs_mut()
        .append_pair("key", api_key.expose_secret());
    let response: Response = send_json(
        "google-cloud-translation",
        client.post(url).json(&Request {
            q: &request.segments,
            target: request.target_language.tag(),
            source: request.source_language.map(|language| language.tag()),
            format: "text",
        }),
    )
    .await?;
    Ok(response
        .data
        .translations
        .into_iter()
        .map(|translation| translation.translated_text)
        .collect())
}

#[derive(Serialize)]
struct Request<'a> {
    q: &'a [String],
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    format: &'static str,
}

#[derive(Deserialize)]
struct Response {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
struct Translation {
    #[serde(rename = "translatedText")]
    translated_text: String,
}
