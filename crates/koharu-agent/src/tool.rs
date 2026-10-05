use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Control;

#[derive(Clone, Debug)]
pub struct ToolImage {
    pub label: String,
    pub data_url: String,
    /// How the provider should read this image: `"high"` tiles it and bills per
    /// tile, `"low"` is one cheap fixed view, `"auto"` lets the provider decide
    /// from the encoded size.
    pub detail: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    kind: &'static str,
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}

impl Tool {
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            kind: "function",
            name: name.into(),
            description: description.into(),
            parameters,
            strict: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct Invocation {
    pub value: Value,
    /// What survives in conversation history after this turn. Defaults to
    /// `value`; a tool whose live result is too large to keep in every later
    /// request sets its own summary, because only the current turn needs the
    /// detail.
    pub history: Value,
    pub changed: bool,
    pub images: Vec<ToolImage>,
}

impl Invocation {
    pub fn read(value: impl Serialize) -> Result<Self> {
        let value = serde_json::to_value(value)?;
        Ok(Self {
            history: value.clone(),
            value,
            changed: false,
            images: Vec::new(),
        })
    }

    pub fn changed(value: impl Serialize) -> Result<Self> {
        let value = serde_json::to_value(value)?;
        Ok(Self {
            history: value.clone(),
            value,
            changed: true,
            images: Vec::new(),
        })
    }

    #[must_use]
    pub fn with_history(mut self, history: Value) -> Self {
        self.history = history;
        self
    }

    #[must_use]
    pub fn with_image(
        mut self,
        label: impl Into<String>,
        data_url: impl Into<String>,
        detail: &'static str,
    ) -> Self {
        self.images.push(ToolImage {
            label: label.into(),
            data_url: data_url.into(),
            detail,
        });
        self
    }
}

#[async_trait]
pub trait Host: Send + Sync + 'static {
    async fn context(&self) -> Result<Value>;

    fn tools(&self) -> Vec<Tool>;

    async fn invoke(&self, call: ToolCall, control: &Control) -> Result<Invocation>;

    /// Page text for the bulk reviewer, as `{ "pages": [...] }` with the
    /// element set and order the page view reports.
    async fn bulk_review_pages(&self, pages: &[String]) -> Result<Value>;

    /// Display name of the language translations are being revised into. The
    /// agent crate does not own a language model, and source language is never
    /// a job-wide fact — it stays per element in the reviewed payload.
    fn review_target_language(&self) -> Result<String>;
}
