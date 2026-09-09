// Ported from:
// https://github.com/koharu-rs/koharu/blob/f4ce03999ed1ae2faaec938dd52c2f41a87d03d9/crates/koharu-llm/src/providers/claude.rs
// Model discovery:
// https://platform.claude.com/docs/en/api/models/list

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::send_json;
use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image, prompt,
};

const URL: &str = "https://api.anthropic.com/v1/messages";
const MODELS_URL: &str = "https://api.anthropic.com/v1/models?limit=1000";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct ClaudeConfig {}

pub(super) async fn models(client: &Client) -> Result<Vec<Model>> {
    let Some(api_key) = koharu_secrets::get("claude")? else {
        return Ok(Vec::new());
    };
    let response: ModelsResponse = send_json(
        "claude",
        client
            .get(MODELS_URL)
            .header("x-api-key", api_key.expose_secret())
            .header("anthropic-version", "2023-06-01"),
    )
    .await?;
    Ok(response
        .data
        .into_iter()
        .map(|model| {
            let reasoning = model.capabilities.thinking.types.adaptive.supported;
            Model {
                provider: Provider::Claude,
                model: Some(model.id),
                name: model.display_name,
                quantizations: Vec::new(),
                vision: model.capabilities.image_input.supported,
                reasoning,
            }
        })
        .collect())
}

pub(super) async fn translate(
    client: &Client,
    _config: &ClaudeConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("claude")?.context("claude API key is not configured")?;
    let (system, user) = prompt::prompts(request)?;
    let body = Request {
        model,
        max_tokens: generation.max_tokens.unwrap_or(8192),
        system: &system,
        messages: [Message::user(&user, request.image.as_deref())?],
        temperature: generation.temperature,
        thinking: generation.reasoning.map(|enabled| ThinkingConfig {
            kind: if enabled { "adaptive" } else { "disabled" },
        }),
        output_config: OutputConfig {
            format: JsonOutputFormat {
                kind: "json_schema",
                schema: prompt::output_schema(request.segments.len()),
            },
        },
    };
    let response: Response = send_json(
        "claude",
        client
            .post(URL)
            .header("x-api-key", api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .json(&body),
    )
    .await?;
    let text = response
        .content
        .into_iter()
        .find_map(|block| (block.kind == "text").then_some(block.text).flatten())
        .context("Claude returned no text content")?;
    Ok(prompt::translations("claude", &text, &request.segments)?)
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: [Message<'a>; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    output_config: OutputConfig,
}

#[derive(Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct OutputConfig {
    format: JsonOutputFormat,
}

#[derive(Serialize)]
struct JsonOutputFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    schema: serde_json::Value,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: Vec<ContentBlock<'a>>,
}

impl<'a> Message<'a> {
    fn user(text: &'a str, image: Option<&image::DynamicImage>) -> anyhow::Result<Self> {
        let mut content = vec![ContentBlock::Text { text }];
        if let Some(image) = image {
            content.push(ContentBlock::Image {
                source: ImageSource {
                    kind: "base64",
                    media_type: "image/jpeg",
                    data: encode_image(image)?.data,
                },
            });
        }
        Ok(Self {
            role: "user",
            content,
        })
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock<'a> {
    Text { text: &'a str },
    Image { source: ImageSource },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'static str,
    data: String,
}

#[derive(Deserialize)]
struct Response {
    content: Vec<Content>,
}

#[derive(Deserialize)]
struct Content {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    display_name: String,
    capabilities: ModelCapabilities,
}

#[derive(Deserialize)]
struct ModelCapabilities {
    image_input: CapabilitySupport,
    thinking: ThinkingCapability,
}

#[derive(Deserialize)]
struct ThinkingCapability {
    types: ThinkingTypes,
}

#[derive(Deserialize)]
struct ThinkingTypes {
    adaptive: CapabilitySupport,
}

#[derive(Deserialize)]
struct CapabilitySupport {
    supported: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_text_and_base64_image_blocks() {
        let message =
            Message::user("translate", Some(&image::DynamicImage::new_rgb8(1, 1))).unwrap();
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "image");
        assert_eq!(value["content"][1]["source"]["type"], "base64");
        assert_eq!(value["content"][1]["source"]["media_type"], "image/jpeg");
    }

    #[test]
    fn serializes_structured_output_with_thinking() {
        let body = Request {
            model: "claude-opus-4-8",
            max_tokens: 1024,
            system: "system",
            messages: [Message::user("translate", None).unwrap()],
            temperature: None,
            thinking: Some(ThinkingConfig { kind: "adaptive" }),
            output_config: OutputConfig {
                format: JsonOutputFormat {
                    kind: "json_schema",
                    schema: prompt::output_schema(2),
                },
            },
        };
        let value = serde_json::to_value(body).unwrap();

        assert_eq!(value["thinking"]["type"], "adaptive");
        assert_eq!(value["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            value["output_config"]["format"]["schema"]["properties"]["translations"]["maxItems"],
            2
        );
    }

    #[test]
    fn reads_adaptive_thinking_capability_from_model_list() {
        let response: ModelsResponse = serde_json::from_value(serde_json::json!({
            "data": [{
                "id": "claude-opus-4-8",
                "display_name": "Claude Opus 4.8",
                "capabilities": {
                    "image_input": { "supported": true },
                    "thinking": {
                        "types": {
                            "adaptive": { "supported": true }
                        }
                    }
                }
            }]
        }))
        .unwrap();
        assert!(
            response.data[0]
                .capabilities
                .thinking
                .types
                .adaptive
                .supported
        );
    }
}
