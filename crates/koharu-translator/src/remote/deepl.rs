// Ported from:
// https://github.com/koharu-rs/koharu/blob/f4ce03999ed1ae2faaec938dd52c2f41a87d03d9/crates/koharu-llm/src/providers/deepl.rs

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use super::send_json;
use crate::{Error, Language, Model, Provider, Result, TranslationRequest};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct DeepLConfig {
    pub base_url: Option<Url>,
}

pub(super) async fn models() -> Result<Vec<Model>> {
    Ok(if koharu_secrets::get("deepl")?.is_some() {
        vec![Model::service(Provider::DeepL, "DeepL")]
    } else {
        Vec::new()
    })
}

pub(super) async fn translate(
    client: &Client,
    config: &DeepLConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("deepl")?.context("deepl API key is not configured")?;
    let target = target(request.target_language).ok_or(Error::UnsupportedLanguage {
        provider: "deepl",
        language: request.target_language,
    })?;
    let source = request
        .source_language
        .map(|language| {
            source(language).ok_or(Error::UnsupportedSourceLanguage {
                provider: "deepl",
                language,
            })
        })
        .transpose()?;
    let root = config.base_url.as_ref().map_or_else(
        || {
            if api_key.expose_secret().trim_end().ends_with(":fx") {
                "https://api-free.deepl.com".to_owned()
            } else {
                "https://api.deepl.com".to_owned()
            }
        },
        |url| url.as_str().trim_end_matches('/').to_owned(),
    );
    // DeepL accepts source-language context but does not translate it.
    let context = request
        .context
        .iter()
        .map(|entry| entry.source.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut form = request
        .segments
        .iter()
        .map(|text| ("text", text.as_str()))
        .collect::<Vec<_>>();
    form.push(("target_lang", target));
    if let Some(source) = source {
        form.push(("source_lang", source));
    }
    if !context.is_empty() {
        form.push(("context", &context));
    }
    let response: Response = send_json(
        "deepl",
        client
            .post(format!("{root}/v2/translate"))
            .header(
                "Authorization",
                format!("DeepL-Auth-Key {}", api_key.expose_secret()),
            )
            .form(&form),
    )
    .await?;
    Ok(response
        .translations
        .into_iter()
        .map(|translation| translation.text)
        .collect())
}

#[derive(Deserialize)]
struct Response {
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
struct Translation {
    text: String,
}

fn target(language: Language) -> Option<&'static str> {
    use Language::*;
    Some(match language {
        ChineseSimplified => "ZH-HANS",
        ChineseTraditional => "ZH-HANT",
        English => "EN-US",
        French => "FR",
        Portuguese => "PT-PT",
        BrazilianPortuguese => "PT-BR",
        Spanish => "ES",
        Japanese => "JA",
        Turkish => "TR",
        Russian => "RU",
        Arabic => "AR",
        Korean => "KO",
        Thai => "TH",
        Italian => "IT",
        German => "DE",
        Vietnamese => "VI",
        Malay => "MS",
        Indonesian => "ID",
        Hindi => "HI",
        Polish => "PL",
        Czech => "CS",
        Dutch => "NL",
        Gujarati => "GU",
        Urdu => "UR",
        Telugu => "TE",
        Marathi => "MR",
        Hebrew => "HE",
        Bengali => "BN",
        Bulgarian => "BG",
        Tamil => "TA",
        Ukrainian => "UK",
        Kazakh => "KK",
        Belarusian => "BE",
        Hungarian => "HU",
        Filipino | Khmer | Burmese | Persian | Tibetan | Mongolian | Uyghur | Cantonese => {
            return None;
        }
    })
}

fn source(language: Language) -> Option<&'static str> {
    use Language::*;
    Some(match language {
        ChineseSimplified | ChineseTraditional => "ZH",
        English => "EN",
        Portuguese | BrazilianPortuguese => "PT",
        language => target(language)?.split('-').next()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_languages_are_not_substituted() {
        assert_eq!(target(Language::Filipino), None);
        assert_eq!(target(Language::English), Some("EN-US"));
    }
}
