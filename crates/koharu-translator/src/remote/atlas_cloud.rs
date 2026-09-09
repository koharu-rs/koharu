// https://www.atlascloud.ai/docs/llm-protocols
// https://www.atlascloud.ai/docs/createChatCompletion

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::send_json;
use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image,
    display_name, prompt,
};

const CHAT_URL: &str = "https://api.atlascloud.ai/v1/chat/completions";
const MODELS_URL: &str = "https://api.atlascloud.ai/v1/models";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct AtlasCloudConfig {}

pub(super) async fn translate(
    client: &Client,
    _config: &AtlasCloudConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key =
        koharu_secrets::get("atlas-cloud")?.context("atlas-cloud API key is not configured")?;
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
        max_tokens: generation.max_tokens,
        temperature: generation.temperature,
        top_p: generation.top_p,
        frequency_penalty: generation.frequency_penalty,
        presence_penalty: generation.presence_penalty,
        thinking: generation.reasoning.map(|enabled| ThinkingConfig {
            kind: if enabled { "enabled" } else { "disabled" },
        }),
        response_format: ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchema {
                name: "manga_translation",
                strict: true,
                schema: prompt::output_schema(request.segments.len()),
            },
        },
    };
    let response: ChatResponse = send_json(
        "atlas-cloud",
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
        .context("Atlas Cloud returned no choices")?
        .message
        .content
        .context("Atlas Cloud returned no message content")?;
    Ok(prompt::translations(
        "atlas-cloud",
        &text,
        &request.segments,
    )?)
}

pub(super) async fn models(client: &Client) -> Result<Vec<Model>> {
    let Some(api_key) = koharu_secrets::get("atlas-cloud")? else {
        return Ok(Vec::new());
    };
    let discovered: Result<ModelsResponse> = send_json(
        "atlas-cloud",
        client.get(MODELS_URL).bearer_auth(api_key.expose_secret()),
    )
    .await;
    Ok(match discovered {
        Ok(discovered) => discovered
            .data
            .into_iter()
            .map(|model| Model {
                provider: Provider::AtlasCloud,
                name: display_name(&model.id),
                model: Some(model.id),
                quantizations: Vec::new(),
                vision: false,
                reasoning: true,
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "failed to list Atlas Cloud models");
            Vec::new()
        }
    })
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    kind: &'static str,
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
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_include_the_required_v1_prefix() {
        assert_eq!(CHAT_URL, "https://api.atlascloud.ai/v1/chat/completions");
        assert_eq!(MODELS_URL, "https://api.atlascloud.ai/v1/models");
    }

    #[test]
    fn serializes_atlas_cloud_request_contract() {
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
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            thinking: Some(ThinkingConfig { kind: "disabled" }),
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
        assert_eq!(value["thinking"]["type"], "disabled");
        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
    }
}
