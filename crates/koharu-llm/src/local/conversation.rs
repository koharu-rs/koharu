//! A rig conversation in the OpenAI-style shape chat templates read, with the
//! merges the catalog's templates need: most of Qwen raises an error on a second
//! leading system message. Two user messages in a row, which compaction's summary
//! can leave behind, merge the same way.

use std::sync::Arc;

use base64::Engine as _;
use rig_core::completion::{
    AssistantContent, Message,
    message::{
        DocumentData, DocumentSourceKind, Image, ToolResult, ToolResultContent, UserContent,
    },
};
use serde_json::{Value, json};

/// A conversation ready for a chat template.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Conversation {
    pub(super) messages: Vec<Value>,
    /// In the order their `marker`s appear in the messages.
    pub(super) images: Vec<Vec<u8>>,
}

pub(super) fn conversation(history: &[Arc<Message>], marker: &str) -> Result<Conversation, String> {
    let mut conversation = Conversation::default();
    for message in history {
        match &**message {
            Message::System { content } => conversation.push("system", content),
            Message::User { content } => conversation.user(content, marker)?,
            Message::Assistant(turn) => {
                conversation.messages.push(assistant(&turn.content));
            }
        }
    }
    Ok(conversation)
}

impl Conversation {
    /// `text` as a message, or at the end of the last one if it has the same
    /// role.
    fn push(&mut self, role: &str, text: &str) {
        if let Some(last) = self.messages.last_mut()
            && last["role"] == role
            && let Some(held) = last["content"].as_str()
        {
            last["content"] = json!(format!("{held}\n\n{text}"));
            return;
        }
        self.messages.push(json!({"role": role, "content": text}));
    }

    fn user(&mut self, content: &[UserContent], marker: &str) -> Result<(), String> {
        let mut text = String::new();
        for part in content {
            match part {
                UserContent::Text(part) => text.push_str(&part.text),
                UserContent::Image(image) => {
                    self.images.push(bytes(image)?);
                    text.push_str(marker);
                }
                // rig adds a request's documents to the history as these.
                UserContent::Document(document) => match &document.data {
                    DocumentData::Text(document) => {
                        text.push_str(document);
                        text.push_str("\n\n");
                    }
                    _ => return Err("a local model reads a document's text, not a file".to_owned()),
                },
                UserContent::ToolResult(result) => {
                    self.messages.push(tool(result)?);
                }
                _ => return Err("a local model reads only text and images".to_owned()),
            }
        }
        if !text.is_empty() {
            self.push("user", &text);
        }
        Ok(())
    }
}

fn assistant(content: &[AssistantContent]) -> Value {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls = Vec::new();
    for block in content {
        match block {
            AssistantContent::Text(block) => text.push_str(&block.text),
            AssistantContent::Reasoning(block) => reasoning.push_str(&block.text),
            AssistantContent::ToolCall(call) => calls.push(json!({
                "id": call.id.wire(),
                "type": "function",
                // A map, which the templates iterate.
                "function": {"name": call.function.name.as_str(), "arguments": call.function.arguments},
            })),
            _ => {}
        }
    }
    let mut message = json!({"role": "assistant", "content": text});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    message
}

fn tool(result: &ToolResult) -> Result<Value, String> {
    let mut text = String::new();
    for part in &result.content {
        match part {
            ToolResultContent::Text(part) => text.push_str(&part.text),
            ToolResultContent::Json { value } => text.push_str(&value.to_string()),
            _ => return Err("a local model reads only text from a tool".to_owned()),
        }
    }
    Ok(json!({
        "role": "tool",
        "tool_call_id": result.call.wire(),
        "name": result.name.as_str(),
        "content": text,
    }))
}

fn bytes(image: &Image) -> Result<Vec<u8>, String> {
    match &image.data {
        DocumentSourceKind::Raw(bytes) => Ok(bytes.clone()),
        DocumentSourceKind::Base64(data) => base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| error.to_string()),
        _ => Err("a local model reads an image's bytes, not a URL or file id".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(conversation: &Conversation) -> Vec<&str> {
        conversation
            .messages
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn merges_what_the_templates_refuse() {
        let history = [
            Message::system("One."),
            Message::system("Two."),
            Message::user("Summary."),
            Message::user("Page."),
            Message::assistant("Answer."),
            Message::user("Next."),
        ]
        .map(Arc::new);
        let conversation = conversation(&history, "<image>").unwrap();
        assert_eq!(
            roles(&conversation),
            ["system", "user", "assistant", "user"]
        );
        assert_eq!(conversation.messages[0]["content"], "One.\n\nTwo.");
        assert_eq!(conversation.messages[1]["content"], "Summary.\n\nPage.");
    }

    #[test]
    fn reads_a_user_message_in_order_without_separators() {
        let content = vec![
            UserContent::Image(Image {
                data: DocumentSourceKind::Raw(vec![1, 2]),
                media_type: None,
                detail: None,
                native: None,
            }),
            UserContent::text("OCR:"),
        ];
        let history = [Arc::new(Message::User { content })];
        let conversation = conversation(&history, "<image>").unwrap();
        assert_eq!(conversation.messages[0]["content"], "<image>OCR:");
        assert_eq!(conversation.images, [vec![1, 2]]);
    }
}
