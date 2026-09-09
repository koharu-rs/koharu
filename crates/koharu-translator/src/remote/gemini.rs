// Ported from:
// https://github.com/koharu-rs/koharu/blob/f4ce03999ed1ae2faaec938dd52c2f41a87d03d9/crates/koharu-llm/src/providers/gemini.rs
// Model discovery:
// https://ai.google.dev/api/models

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use super::send_json;
use crate::{
    GenerationConfig as TranslationGeneration, Model, Provider, Result, TranslationRequest,
    backend::encode_image, prompt,
};

const ROOT: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct GeminiConfig {}

pub(super) async fn models(client: &Client) -> Result<Vec<Model>> {
    let Some(api_key) = koharu_secrets::get("gemini")? else {
        return Ok(Vec::new());
    };
    let mut url = Url::parse(ROOT).expect("Gemini API root is valid");
    url.query_pairs_mut()
        .append_pair("key", api_key.expose_secret())
        .append_pair("pageSize", "1000");
    let response: ModelsResponse = send_json("gemini", client.get(url)).await?;
    Ok(response
        .models
        .into_iter()
        .filter(|model| {
            model
                .supported_generation_methods
                .iter()
                .any(|method| method == "generateContent")
                && supports_translation(&model.name)
        })
        .filter_map(|model| {
            model.name.strip_prefix("models/").map(|id| Model {
                provider: Provider::Gemini,
                model: Some(id.to_owned()),
                name: model.display_name,
                quantizations: Vec::new(),
                vision: true,
                reasoning: model.thinking,
            })
        })
        .collect())
}

fn supports_translation(id: &str) -> bool {
    ![
        "antigravity",
        "computer-use",
        "deep-research",
        "embedding",
        "image",
        "imagen",
        "live",
        "lyria",
        "native-audio",
        "omni",
        "robotics",
        "tts",
        "veo",
    ]
    .iter()
    .any(|marker| id.contains(marker))
}

pub(super) async fn translate(
    client: &Client,
    _config: &GeminiConfig,
    model: &str,
    generation: &TranslationGeneration,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("gemini")?.context("gemini API key is not configured")?;
    let (system, user) = prompt::prompts(request)?;
    let schema = prompt::output_schema(request.segments.len());
    let mut url =
        Url::parse(&format!("{ROOT}/{model}:generateContent")).expect("Gemini API root is valid");
    url.query_pairs_mut()
        .append_pair("key", api_key.expose_secret());
    let body = Request {
        system_instruction: Content::text(&system),
        contents: [Content::user(&user, request.image.as_deref())?],
        generation_config: GenerationConfig {
            temperature: generation.temperature,
            max_output_tokens: generation.max_tokens,
            thinking_config: generation.reasoning.map(|enabled| ThinkingConfig {
                thinking_budget: model.starts_with("gemini-2.5").then(|| {
                    if enabled {
                        -1
                    } else if model.starts_with("gemini-2.5-pro") {
                        // Gemini 2.5 Pro cannot disable thinking; 128 is its minimum budget.
                        128
                    } else {
                        0
                    }
                }),
                thinking_level: (!model.starts_with("gemini-2.5")).then_some(if enabled {
                    "high"
                } else if model.starts_with("gemma-4") {
                    "minimal"
                } else {
                    "low"
                }),
            }),
            response_mime_type: "application/json",
            response_json_schema: schema,
        },
    };
    let response: Response = send_json("gemini", client.post(url).json(&body)).await?;
    let text = response
        .candidates
        .into_iter()
        .next()
        .and_then(|candidate| candidate.content.parts.into_iter().next())
        .context("Gemini returned no candidate content")?
        .text;
    Ok(prompt::translations("gemini", &text, &request.segments)?)
}

#[derive(Serialize)]
struct Request<'a> {
    system_instruction: Content<'a>,
    contents: [Content<'a>; 1],
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct Content<'a> {
    parts: Vec<Part<'a>>,
}

impl<'a> Content<'a> {
    fn text(text: &'a str) -> Self {
        Self {
            parts: vec![Part::Text { text }],
        }
    }

    fn user(text: &'a str, image: Option<&image::DynamicImage>) -> anyhow::Result<Self> {
        let mut parts = vec![Part::Text { text }];
        if let Some(image) = image {
            parts.push(Part::InlineData {
                inline_data: InlineData {
                    mime_type: "image/jpeg",
                    data: encode_image(image)?.data,
                },
            });
        }
        Ok(Self { parts })
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part<'a> {
    Text {
        text: &'a str,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: &'static str,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<ThinkingConfig>,
    response_mime_type: &'static str,
    response_json_schema: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_level: Option<&'static str>,
}

#[derive(Deserialize)]
struct Response {
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    content: ResponseContent,
}

#[derive(Deserialize)]
struct ResponseContent {
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    text: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<ListedModel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedModel {
    name: String,
    display_name: String,
    supported_generation_methods: Vec<String>,
    #[serde(default)]
    thinking: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_structured_output_configuration() {
        let config = GenerationConfig {
            temperature: None,
            max_output_tokens: None,
            thinking_config: None,
            response_mime_type: "application/json",
            response_json_schema: prompt::output_schema(2),
        };
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["responseMimeType"], "application/json");
        assert!(value.get("thinkingConfig").is_none());
        assert_eq!(
            value["responseJsonSchema"]["properties"]["translations"]["items"]["properties"]["id"]
                ["maximum"],
            1
        );
    }

    #[test]
    fn serializes_model_specific_thinking_controls() {
        let enabled = serde_json::to_value(ThinkingConfig {
            thinking_budget: None,
            thinking_level: Some("high"),
        })
        .unwrap();
        assert_eq!(enabled["thinkingLevel"], "high");
        assert!(enabled.get("thinkingBudget").is_none());

        let disabled = serde_json::to_value(ThinkingConfig {
            thinking_budget: None,
            thinking_level: Some("minimal"),
        })
        .unwrap();
        assert_eq!(disabled["thinkingLevel"], "minimal");
        assert!(disabled.get("thinkingBudget").is_none());

        let budget = serde_json::to_value(ThinkingConfig {
            thinking_budget: Some(0),
            thinking_level: None,
        })
        .unwrap();
        assert_eq!(budget["thinkingBudget"], 0);
        assert!(budget.get("thinkingLevel").is_none());
    }

    #[test]
    fn serializes_text_and_inline_image_parts() {
        let content =
            Content::user("translate", Some(&image::DynamicImage::new_rgb8(1, 1))).unwrap();
        let value = serde_json::to_value(content).unwrap();
        assert_eq!(value["parts"][0]["text"], "translate");
        assert_eq!(value["parts"][1]["inlineData"]["mimeType"], "image/jpeg");
        assert!(value["parts"][1]["inlineData"]["data"].is_string());
    }

    #[test]
    fn filters_specialized_generate_content_models() {
        assert!(supports_translation("models/gemini-3.7-flash"));
        assert!(supports_translation("models/gemma-4-31b-it"));
        assert!(!supports_translation("models/gemini-3.1-flash-image"));
        assert!(!supports_translation("models/gemini-3.1-flash-tts-preview"));
    }

    #[test]
    fn reads_optional_thinking_capability_from_model_list() {
        let response: ModelsResponse = serde_json::from_value(serde_json::json!({
            "models": [
                {
                    "name": "models/gemini-3.7-flash",
                    "displayName": "Gemini 3.7 Flash",
                    "supportedGenerationMethods": ["generateContent"],
                    "thinking": true
                },
                {
                    "name": "models/gemini-2.0-flash-lite",
                    "displayName": "Gemini 2.0 Flash-Lite",
                    "supportedGenerationMethods": ["generateContent"]
                }
            ]
        }))
        .unwrap();
        assert!(response.models[0].thinking);
        assert!(!response.models[1].thinking);
    }
}
