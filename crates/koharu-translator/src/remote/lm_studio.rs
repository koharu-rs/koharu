// https://lmstudio.ai/docs/developer/rest/list
// https://lmstudio.ai/docs/developer/openai-compat/chat-completions
// https://lmstudio.ai/docs/developer/openai-compat/structured-output

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use super::send_json;
use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image,
    display_name, prompt,
};

const DEFAULT_BASE_URL: &str = "http://localhost:1234";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct LmStudioConfig {
    pub base_url: Option<Url>,
}

impl Default for LmStudioConfig {
    fn default() -> Self {
        Self {
            base_url: Some(Url::parse(DEFAULT_BASE_URL).expect("default LM Studio URL is valid")),
        }
    }
}

pub(super) async fn translate(
    client: &Client,
    config: &LmStudioConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("lm-studio")?;
    let (system, user) = prompt::prompts(request)?;
    let user_content = match request.image.as_deref() {
        Some(image) => MessageContent::Parts(vec![
            ContentPart::Text { text: user },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: encode_image(image)?.data_url(),
                },
            },
        ]),
        None => MessageContent::Text(user),
    };
    let body = ChatRequest {
        model,
        messages: [
            Message {
                role: "system",
                content: MessageContent::Text(system),
            },
            Message {
                role: "user",
                content: user_content,
            },
        ],
        temperature: generation.temperature,
        top_p: generation.top_p,
        max_tokens: generation.max_tokens,
        frequency_penalty: generation.frequency_penalty,
        presence_penalty: generation.presence_penalty,
        reasoning_effort: generation
            .reasoning
            .map(|enabled| if enabled { "medium" } else { "none" }),
        response_format: ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchema {
                name: "manga_translation",
                strict: true,
                schema: prompt::output_schema(request.segments.len()),
            },
        },
    };
    let http = client
        .post(endpoint(config.base_url.as_ref(), "v1/chat/completions"))
        .json(&body);
    let http = match api_key {
        Some(api_key) => http.bearer_auth(api_key.expose_secret()),
        None => http,
    };
    let response: ChatResponse = send_json("lm-studio", http).await?;
    let text = response
        .choices
        .into_iter()
        .next()
        .context("LM Studio returned no choices")?
        .message
        .content
        .context("LM Studio returned no message content")?;
    Ok(prompt::translations("lm-studio", &text, &request.segments)?)
}

pub(super) async fn models(client: &Client, config: &LmStudioConfig) -> Result<Vec<Model>> {
    let api_key = koharu_secrets::get("lm-studio")?;
    let request = client.get(endpoint(config.base_url.as_ref(), "api/v1/models"));
    let request = match api_key {
        Some(api_key) => request.bearer_auth(api_key.expose_secret()),
        None => request,
    };
    let response: ModelsResponse = send_json("lm-studio", request).await?;
    Ok(response
        .models
        .into_iter()
        .filter(|model| matches!(model.kind.as_str(), "llm" | "vlm"))
        .map(|model| {
            let capabilities = model.capabilities;
            Model {
                provider: Provider::LmStudio,
                name: display_name(&model.key),
                model: Some(model.key),
                quantizations: Vec::new(),
                vision: capabilities
                    .as_ref()
                    .is_some_and(|capabilities| capabilities.vision),
                reasoning: capabilities
                    .is_some_and(|capabilities| capabilities.reasoning.is_some()),
            }
        })
        .collect())
}

fn endpoint(base_url: Option<&Url>, suffix: &str) -> String {
    let base_url = base_url.map_or(DEFAULT_BASE_URL, Url::as_str);
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    json_schema: JsonSchema,
}

#[derive(Serialize)]
struct JsonSchema {
    name: &'static str,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: MessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    #[serde(rename = "type")]
    kind: String,
    key: String,
    capabilities: Option<ListedModelCapabilities>,
}

#[derive(Deserialize)]
struct ListedModelCapabilities {
    vision: bool,
    reasoning: Option<ListedModelReasoning>,
}

#[derive(Deserialize)]
struct ListedModelReasoning {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_structured_chat_and_native_model_endpoints() {
        let base_url = Url::parse("http://localhost:1234/").unwrap();
        assert_eq!(
            endpoint(Some(&base_url), "v1/chat/completions"),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            endpoint(Some(&base_url), "api/v1/models"),
            "http://localhost:1234/api/v1/models"
        );
    }

    #[test]
    fn model_discovery_keeps_language_and_vision_models() {
        let response: ModelsResponse = serde_json::from_value(serde_json::json!({
            "models": [
                {
                    "type": "llm",
                    "key": "publisher/chat-model",
                    "capabilities": { "vision": false }
                },
                {
                    "type": "llm",
                    "key": "publisher/vision-model",
                    "capabilities": {
                        "vision": true,
                        "reasoning": {
                            "allowed_options": ["off", "on"],
                            "default": "on"
                        }
                    }
                },
                {
                    "type": "embedding",
                    "key": "publisher/embed-model"
                }
            ]
        }))
        .unwrap();
        let models = response
            .models
            .into_iter()
            .filter(|model| matches!(model.kind.as_str(), "llm" | "vlm"))
            .map(|model| {
                let capabilities = model.capabilities;
                (
                    model.key,
                    capabilities
                        .as_ref()
                        .is_some_and(|capabilities| capabilities.vision),
                    capabilities.is_some_and(|capabilities| capabilities.reasoning.is_some()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            models,
            [
                ("publisher/chat-model".to_owned(), false, false),
                ("publisher/vision-model".to_owned(), true, true)
            ]
        );
    }

    #[test]
    fn serializes_lm_studio_request_contract() {
        let body = ChatRequest {
            model: "publisher/model",
            messages: [
                Message {
                    role: "system",
                    content: MessageContent::Text("system".to_owned()),
                },
                Message {
                    role: "user",
                    content: MessageContent::Text("user".to_owned()),
                },
            ],
            temperature: None,
            top_p: None,
            max_tokens: Some(1024),
            frequency_penalty: None,
            presence_penalty: None,
            reasoning_effort: Some("none"),
            response_format: ResponseFormat {
                kind: "json_schema",
                json_schema: JsonSchema {
                    name: "manga_translation",
                    strict: true,
                    schema: prompt::output_schema(2),
                },
            },
        };
        let value = serde_json::to_value(body).unwrap();

        assert_eq!(value["max_tokens"], 1024);
        assert_eq!(value["reasoning_effort"], "none");
        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn serializes_openai_compatible_image_input() {
        let content = MessageContent::Parts(vec![
            ContentPart::Text {
                text: "translate".to_owned(),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: "data:image/jpeg;base64,image".to_owned(),
                },
            },
        ]);
        let value = serde_json::to_value(content).unwrap();

        assert_eq!(value[0]["type"], "text");
        assert_eq!(value[1]["type"], "image_url");
    }
}
