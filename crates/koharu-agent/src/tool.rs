use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Control;

#[derive(Clone, Debug)]
pub struct ToolImage {
    pub label: String,
    pub data_url: String,
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
    pub changed: bool,
    pub images: Vec<ToolImage>,
}

impl Invocation {
    pub fn read(value: impl Serialize) -> Result<Self> {
        Ok(Self {
            value: serde_json::to_value(value)?,
            changed: false,
            images: Vec::new(),
        })
    }

    pub fn changed(value: impl Serialize) -> Result<Self> {
        Ok(Self {
            value: serde_json::to_value(value)?,
            changed: true,
            images: Vec::new(),
        })
    }

    #[must_use]
    pub fn with_image(mut self, label: impl Into<String>, data_url: impl Into<String>) -> Self {
        self.images.push(ToolImage {
            label: label.into(),
            data_url: data_url.into(),
        });
        self
    }
}

#[async_trait]
pub trait Host: Send + Sync + 'static {
    async fn context(&self) -> Result<Value>;

    fn tools(&self) -> Vec<Tool>;

    async fn invoke(&self, call: ToolCall, control: &Control) -> Result<Invocation>;

    async fn bulk_review_pages(&self, _pages: &[String]) -> Result<Value> {
        bail!("bulk review is not supported by this host")
    }
}
