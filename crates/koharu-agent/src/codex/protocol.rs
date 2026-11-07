use serde::Serialize;
use serde_json::{Value, json};

use crate::{Reasoning, Tool};

#[derive(Debug, Serialize)]
pub(crate) struct Request {
    pub(crate) model: String,
    pub(crate) instructions: String,
    pub(crate) input: Vec<Value>,
    pub(crate) tools: Vec<Tool>,
    pub(crate) tool_choice: &'static str,
    pub(crate) parallel_tool_calls: bool,
    pub(crate) reasoning: ReasoningOptions,
    pub(crate) text: TextOptions,
    pub(crate) include: [&'static str; 1],
    pub(crate) stream: bool,
    pub(crate) store: bool,
    pub(crate) prompt_cache_key: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReasoningOptions {
    pub(crate) effort: &'static str,
    pub(crate) summary: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct TextOptions {
    pub(crate) verbosity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) format: Option<TextFormat>,
}

/// Structured output for the Responses API lives under `text.format`, not a
/// top-level `response_format` parameter.
#[derive(Debug, Serialize)]
pub(crate) struct TextFormat {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) name: &'static str,
    pub(crate) strict: bool,
    pub(crate) schema: Value,
}

impl Request {
    pub(crate) fn new(
        model: String,
        instructions: String,
        input: Vec<Value>,
        tools: Vec<Tool>,
        reasoning: Reasoning,
        session: String,
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            tool_choice: "auto",
            parallel_tool_calls: false,
            reasoning: ReasoningOptions {
                effort: reasoning.as_str(),
                summary: "auto",
            },
            text: TextOptions {
                verbosity: "low",
                format: None,
            },
            include: ["reasoning.encrypted_content"],
            stream: true,
            store: false,
            prompt_cache_key: session,
        }
    }

    /// Attach an OpenAI-compatible JSON schema structured output under
    /// `text.format`. Pass `None` for plain text output (the default).
    pub(crate) fn with_json_schema(mut self, schema: Option<Value>, name: &'static str) -> Self {
        self.text.format = schema.map(|schema| TextFormat {
            kind: "json_schema",
            name,
            strict: true,
            schema,
        });
        self
    }
}

pub(crate) fn message(role: &str, text: impl Into<String>) -> Value {
    json!({
        "type": "message",
        "role": role,
        "content": [{
            "type": "input_text",
            "text": text.into(),
        }],
    })
}

pub(crate) fn project_context(data: &Value) -> Result<Value, serde_json::Error> {
    let content = vec![json!({
        "type": "input_text",
        "text": format!(
            "<koharu_project_context>\n{}\n</koharu_project_context>",
            serde_json::to_string(data)?
        ),
    })];
    Ok(json!({
        "type": "message",
        "role": "user",
        "content": content,
    }))
}

pub(crate) fn function_output(
    call_id: &str,
    output: &Value,
    images: &[crate::ToolImage],
) -> Result<Value, serde_json::Error> {
    let output = if images.is_empty() {
        Value::String(serde_json::to_string(output)?)
    } else {
        let mut content = vec![json!({
            "type": "input_text",
            "text": serde_json::to_string(output)?,
        })];
        for image in images {
            content.push(json!({
                "type": "input_text",
                "text": image.label,
            }));
            content.push(json!({
                "type": "input_image",
                "image_url": image.data_url,
                "detail": "high",
            }));
        }
        Value::Array(content)
    };
    Ok(json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": output,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn review_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "changes": { "type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "element": { "type": "string" },
                        "translation": { "type": "string" },
                    },
                    "required": ["element", "translation"],
                    "additionalProperties": false,
                } },
                "memory_updates": { "type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "value": { "type": "string" },
                    },
                    "required": ["key", "value"],
                    "additionalProperties": false,
                } },
            },
            "required": ["changes", "memory_updates"],
            "additionalProperties": false,
        })
    }

    #[test]
    fn structured_output_lives_in_text_format() {
        let request = Request::new(
            "codex".into(),
            "instructions".into(),
            Vec::new(),
            Vec::new(),
            Reasoning::Low,
            "session".into(),
        )
        .with_json_schema(Some(review_schema()), "review_response");
        let value = serde_json::to_value(&request).unwrap();
        let body = value.to_string();
        // Structured output must be under text.format.
        assert_eq!(value["text"]["verbosity"], "low");
        assert_eq!(value["text"]["format"]["type"], "json_schema");
        assert_eq!(value["text"]["format"]["name"], "review_response");
        assert_eq!(value["text"]["format"]["strict"], true);
        assert_eq!(value["text"]["format"]["schema"]["type"], "object");
        // There must be no top-level response_format parameter.
        assert!(value.get("response_format").is_none());
        assert!(!body.contains("response_format"));
        // Instructions, tools and reasoning remain wired.
        assert_eq!(value["instructions"], "instructions");
        assert_eq!(value["reasoning"]["effort"], "low");
        assert_eq!(value["tools"], json!([]));
    }

    #[test]
    fn plain_request_has_no_text_format() {
        let request = Request::new(
            "codex".into(),
            "i".into(),
            Vec::new(),
            Vec::new(),
            Reasoning::High,
            "s".into(),
        );
        let value = serde_json::to_value(&request).unwrap();
        assert!(value["text"].get("format").is_none());
        assert!(value.get("response_format").is_none());
        assert_eq!(value["text"]["verbosity"], "low");
    }
}
