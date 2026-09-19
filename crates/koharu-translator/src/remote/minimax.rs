// https://platform.minimax.io/docs/api-reference/text-post
// https://platform.minimax.io/docs/api-reference/models/openai/list-models

use anyhow::Context;
use koharu_secrets::ExposeSecret;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::send_json;
use crate::{GenerationConfig, Model, Provider, Result, TranslationRequest, display_name, prompt};

// MiniMax recommends compatibility APIs, but this provider intentionally uses its native route.
const URL: &str = "https://api.minimax.io/v1/text/chatcompletion_v2";
const MODELS_URL: &str = "https://api.minimax.io/v1/models";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct MiniMaxConfig {}

pub(super) async fn models(client: &Client) -> Result<Vec<Model>> {
    let Some(api_key) = koharu_secrets::get("minimax")? else {
        return Ok(Vec::new());
    };
    let response: ModelsResponse = send_json(
        "minimax",
        client.get(MODELS_URL).bearer_auth(api_key.expose_secret()),
    )
    .await?;
    Ok(response
        .data
        .into_iter()
        .map(|model| Model {
            provider: Provider::MiniMax,
            name: display_name(&model.id),
            model: Some(model.id),
            quantizations: Vec::new(),
            vision: false,
            reasoning: true,
        })
        .collect())
}

pub(super) async fn translate(
    client: &Client,
    _config: &MiniMaxConfig,
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> Result<Vec<String>> {
    let api_key = koharu_secrets::get("minimax")?.context("minimax API key is not configured")?;
    let (system, user) = prompt::prompts(request)?;
    let response: Response = send_json(
        "minimax",
        client
            .post(URL)
            .bearer_auth(api_key.expose_secret())
            .json(&Request {
                model,
                messages: [
                    Message {
                        role: "system",
                        content: system,
                    },
                    Message {
                        role: "user",
                        content: user,
                    },
                ],
                temperature: generation.temperature,
                top_p: generation.top_p,
                max_completion_tokens: generation.max_tokens,
                response_format: response_format(model, request.segments.len()),
            }),
    )
    .await?;
    if response.base_resp.status_code != 0 {
        return Err(anyhow::anyhow!(
            "MiniMax returned status {}: {}",
            response.base_resp.status_code,
            response.base_resp.status_msg
        )
        .into());
    }
    let text = response
        .choices
        .into_iter()
        .next()
        .context("MiniMax returned no choices")?
        .message
        .content;
    Ok(prompt::translations("minimax", &text, &request.segments)?)
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: [Message; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
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

fn response_format(model: &str, expected: usize) -> Option<ResponseFormat> {
    (model == "MiniMax-Text-01").then(|| ResponseFormat {
        kind: "json_schema",
        json_schema: JsonSchema {
            name: "manga_translation",
            strict: true,
            schema: prompt::output_schema(expected),
        },
    })
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    base_resp: BaseResponse,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
}

#[derive(Default, Deserialize)]
struct BaseResponse {
    status_code: i64,
    #[serde(default)]
    status_msg: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_native_completion_fields() {
        let body = Request {
            model: "MiniMax-M3",
            messages: [
                Message {
                    role: "system",
                    content: "system".to_owned(),
                },
                Message {
                    role: "user",
                    content: "user".to_owned(),
                },
            ],
            temperature: Some(0.7),
            top_p: Some(0.95),
            max_completion_tokens: Some(1024),
            response_format: response_format("MiniMax-M3", 2),
        };
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(value["model"], "MiniMax-M3");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][1]["content"], "user");
        assert_eq!(value["max_completion_tokens"], 1024);
        assert!(value.get("response_format").is_none());
    }

    #[test]
    fn serializes_schema_only_for_minimax_text_01() {
        let body = Request {
            model: "MiniMax-Text-01",
            messages: [
                Message {
                    role: "system",
                    content: "system".to_owned(),
                },
                Message {
                    role: "user",
                    content: "user".to_owned(),
                },
            ],
            temperature: None,
            top_p: None,
            max_completion_tokens: None,
            response_format: response_format("MiniMax-Text-01", 2),
        };
        let value = serde_json::to_value(body).unwrap();

        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(
            value["response_format"]["json_schema"]["schema"]["properties"]["translations"]["maxItems"],
            2
        );
    }

    #[test]
    fn decodes_native_error_without_choices() {
        let response: Response = serde_json::from_value(serde_json::json!({
            "base_resp": {
                "status_code": 1008,
                "status_msg": "insufficient balance"
            }
        }))
        .unwrap();
        assert!(response.choices.is_empty());
        assert_eq!(response.base_resp.status_code, 1008);
        assert_eq!(response.base_resp.status_msg, "insufficient balance");
    }
}
