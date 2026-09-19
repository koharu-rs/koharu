// https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create
// https://developers.openai.com/api/reference/resources/models/methods/list

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::send_json;
use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image,
    display_name, prompt,
};

const URL: &str = "https://api.openai.com/v1/chat/completions";
const MODELS_URL: &str = "https://api.openai.com/v1/models";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct OpenAiConfig {}

pub(super) async fn models(client: &Client) -> Result<Vec<Model>> {
    let Some(api_key) = koharu_secrets::get("openai")? else {
        return Ok(Vec::new());
    };
    let response: ModelsResponse = send_json(
        "openai",
        client.get(MODELS_URL).bearer_auth(api_key.expose_secret()),
    )
    .await?;
    Ok(response
        .data
        .into_iter()
        .filter(|model| supports_translation(&model.id))
        .map(|model| Model {
            provider: Provider::OpenAi,
            name: display_name(&model.id),
            model: Some(model.id),
            quantizations: Vec::new(),
            vision: true,
            reasoning: true,
        })
        .collect())
}

fn supports_translation(id: &str) -> bool {
    let base = id
        .strip_prefix("ft:")
        .and_then(|id| id.split(':').next())
        .unwrap_or(id)
        .to_ascii_lowercase();
    let supported_family = base.starts_with("gpt-5")
        || base.starts_with("gpt-4.1")
        || base.starts_with("gpt-4o")
        || base == "o3"
        || base.starts_with("o3-");
    let incompatible = [
        "-audio",
        "-codex",
        "-deep-research",
        "-image",
        "-pro",
        "-realtime",
        "-search",
        "-transcribe",
        "-tts",
    ]
    .iter()
    .any(|marker| base.contains(marker));
    supported_family && !incompatible
}

pub(super) async fn translate(
    client: &Client,
    _config: &OpenAiConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("openai")?.context("openai API key is not configured")?;
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
        max_completion_tokens: generation.max_tokens,
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
    let response: ChatResponse = send_json(
        "openai",
        client
            .post(URL)
            .bearer_auth(api_key.expose_secret())
            .json(&body),
    )
    .await?;
    let text = response
        .choices
        .into_iter()
        .next()
        .context("OpenAI returned no choices")?
        .message
        .content
        .context("OpenAI returned no message content")?;
    Ok(prompt::translations("openai", &text, &request.segments)?)
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
    max_completion_tokens: Option<u32>,
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
    fn filters_models_to_the_chat_translation_contract() {
        assert!(supports_translation("gpt-5.6-luna"));
        assert!(supports_translation("gpt-4o-mini"));
        assert!(supports_translation("ft:gpt-4o-mini:org:name:id"));
        assert!(!supports_translation("gpt-5.5-pro"));
        assert!(!supports_translation("gpt-5.3-codex"));
        assert!(!supports_translation("gpt-4o-audio-preview"));
        assert!(!supports_translation("text-embedding-3-large"));
    }

    #[test]
    fn serializes_openai_request_contract() {
        let body = ChatRequest {
            model: "gpt-5.6-luna",
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
            max_completion_tokens: Some(1024),
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

        assert_eq!(value["max_completion_tokens"], 1024);
        assert!(value.get("max_tokens").is_none());
        assert_eq!(value["reasoning_effort"], "none");
        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
    }
}
