// https://openrouter.ai/docs/api/reference/overview
// https://openrouter.ai/docs/api/api-reference/models/get-models
// https://openrouter.ai/docs/guides/best-practices/reasoning-tokens
// https://openrouter.ai/docs/guides/features/structured-outputs
// https://openrouter.ai/docs/guides/routing/provider-selection

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::send_json;
use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image, prompt,
};

const CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct OpenRouterConfig {}

pub(super) async fn translate(
    client: &Client,
    _config: &OpenRouterConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key =
        koharu_secrets::get("openrouter")?.context("openrouter API key is not configured")?;
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
        reasoning: generation
            .reasoning
            .map(|enabled| ReasoningConfig { enabled }),
        response_format: ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchema {
                name: "manga_translation",
                strict: true,
                schema: prompt::output_schema(request.segments.len()),
            },
        },
        provider: ProviderPreferences {
            require_parameters: true,
        },
    };
    let response: ChatResponse = send_json(
        "openrouter",
        client
            .post(CHAT_URL)
            .bearer_auth(api_key.expose_secret())
            .json(&body),
    )
    .await?;
    let text = response
        .choices
        .into_iter()
        .next()
        .context("OpenRouter returned no choices")?
        .message
        .content
        .context("OpenRouter returned no message content")?;
    Ok(prompt::translations(
        "openrouter",
        &text,
        &request.segments,
    )?)
}

pub(super) async fn models(client: &Client) -> Result<Vec<Model>> {
    let Some(api_key) = koharu_secrets::get("openrouter")? else {
        return Ok(Vec::new());
    };
    let discovered: Result<ModelsResponse> = super::send_json(
        "openrouter",
        client.get(MODELS_URL).bearer_auth(api_key.expose_secret()),
    )
    .await;
    Ok(match discovered {
        Ok(discovered) => discovered
            .data
            .into_iter()
            .map(|model| Model {
                provider: Provider::OpenRouter,
                name: model.name,
                model: Some(model.id),
                quantizations: Vec::new(),
                vision: model
                    .architecture
                    .input_modalities
                    .iter()
                    .any(|modality| modality == "image"),
                reasoning: model.reasoning.is_some()
                    || model
                        .supported_parameters
                        .iter()
                        .any(|parameter| parameter == "reasoning"),
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "failed to list OpenRouter models");
            Vec::new()
        }
    })
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    name: String,
    architecture: Architecture,
    supported_parameters: Vec<String>,
    reasoning: Option<ReasoningCapabilities>,
}

#[derive(Deserialize)]
struct ReasoningCapabilities {}

#[derive(Deserialize)]
struct Architecture {
    input_modalities: Vec<String>,
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
    reasoning: Option<ReasoningConfig>,
    response_format: ResponseFormat,
    provider: ProviderPreferences,
}

#[derive(Serialize)]
struct ReasoningConfig {
    enabled: bool,
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
struct ProviderPreferences {
    require_parameters: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_reasoning_capability_from_model_list() {
        let response: ModelsResponse = serde_json::from_value(serde_json::json!({
            "data": [
                {
                    "id": "provider/reasoning-model",
                    "name": "Reasoning Model",
                    "architecture": { "input_modalities": ["text"] },
                    "supported_parameters": ["reasoning"],
                    "reasoning": {
                        "supported_efforts": ["high", "medium", "low"]
                    }
                },
                {
                    "id": "provider/chat-model",
                    "name": "Chat Model",
                    "architecture": { "input_modalities": ["text"] },
                    "supported_parameters": []
                }
            ]
        }))
        .unwrap();
        assert!(response.data[0].reasoning.is_some());
        assert!(response.data[1].reasoning.is_none());
    }

    #[test]
    fn serializes_openrouter_request_contract() {
        let body = ChatRequest {
            model: "provider/model",
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
            reasoning: Some(ReasoningConfig { enabled: true }),
            response_format: ResponseFormat {
                kind: "json_schema",
                json_schema: JsonSchema {
                    name: "manga_translation",
                    strict: true,
                    schema: prompt::output_schema(2),
                },
            },
            provider: ProviderPreferences {
                require_parameters: true,
            },
        };
        let value = serde_json::to_value(body).unwrap();

        assert_eq!(value["reasoning"]["enabled"], true);
        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
        assert_eq!(value["provider"]["require_parameters"], true);
    }
}
