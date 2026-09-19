use koharu_ml::llm::GenerationOptions;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Provider;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct GenerationConfig {
    pub temperature: Option<f32>,
    pub top_k: Option<u32>,
    pub top_p: Option<f32>,
    pub min_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub repeat_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub reasoning: Option<bool>,
    pub vision: Option<bool>,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            temperature: None,
            top_k: None,
            top_p: None,
            min_p: None,
            max_tokens: None,
            repeat_penalty: None,
            frequency_penalty: None,
            presence_penalty: None,
            reasoning: Some(false),
            vision: Some(true),
        }
    }
}

impl GenerationConfig {
    pub(crate) fn for_model(self, model: &ModelSelection) -> Self {
        Self {
            reasoning: if model.reasoning {
                self.reasoning
            } else {
                None
            },
            vision: if model.vision { self.vision } else { None },
            ..self
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct ModelSelection {
    pub provider: Provider,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub quantization: Option<String>,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub reasoning: bool,
}

impl Default for ModelSelection {
    fn default() -> Self {
        Self {
            provider: Provider::Local,
            model: Some(crate::local::DEFAULT_MODEL.to_owned()),
            quantization: Some(crate::local::DEFAULT_QUANTIZATION.to_owned()),
            vision: true,
            reasoning: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct Quantization {
    pub id: String,
    pub name: String,
    pub downloaded: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct Model {
    pub provider: Provider,
    pub model: Option<String>,
    pub name: String,
    pub quantizations: Vec<Quantization>,
    pub vision: bool,
    pub reasoning: bool,
}

impl Model {
    pub(crate) fn service(provider: Provider, name: &str) -> Self {
        Self {
            provider,
            model: None,
            name: name.to_owned(),
            quantizations: Vec::new(),
            vision: false,
            reasoning: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QuantizationDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub filename: &'static str,
}

impl QuantizationDefinition {
    pub const fn new(id: &'static str, name: &'static str, filename: &'static str) -> Self {
        Self { id, name, filename }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ModelGeneration {
    pub temperature: Option<f32>,
    pub top_k: Option<u32>,
    pub top_p: Option<f32>,
    pub min_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub repeat_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
}

impl ModelGeneration {
    pub(crate) fn options(self, overrides: GenerationConfig) -> GenerationOptions {
        let defaults = GenerationOptions::default();
        GenerationOptions {
            max_tokens: overrides
                .max_tokens
                .or(self.max_tokens)
                .map_or(1000, |value| value as usize),
            temperature: overrides
                .temperature
                .or(self.temperature)
                .unwrap_or(defaults.temperature),
            top_k: overrides.top_k.or(self.top_k).map(|value| value as usize),
            top_p: overrides.top_p.or(self.top_p),
            min_p: overrides.min_p.or(self.min_p),
            repeat_penalty: overrides
                .repeat_penalty
                .or(self.repeat_penalty)
                .unwrap_or(defaults.repeat_penalty),
            frequency_penalty: overrides
                .frequency_penalty
                .or(self.frequency_penalty)
                .unwrap_or(defaults.frequency_penalty),
            presence_penalty: overrides
                .presence_penalty
                .or(self.presence_penalty)
                .unwrap_or(defaults.presence_penalty),
            ..defaults
        }
    }
}

#[must_use]
pub(crate) fn display_name(model: &str) -> String {
    model
        .rsplit_once('/')
        .map_or(model, |(_, model)| model)
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(vision: bool, reasoning: bool) -> ModelSelection {
        ModelSelection {
            provider: Provider::LmStudio,
            model: Some("publisher/model".to_owned()),
            quantization: None,
            vision,
            reasoning,
        }
    }

    #[test]
    fn generation_omits_unsupported_model_capabilities() {
        let generation = GenerationConfig {
            vision: Some(true),
            reasoning: Some(false),
            ..GenerationConfig::default()
        };
        let supported = generation.for_model(&selection(true, true));
        assert_eq!(supported.vision, Some(true));
        assert_eq!(supported.reasoning, Some(false));

        let unsupported = generation.for_model(&selection(false, false));
        assert_eq!(unsupported.vision, None);
        assert_eq!(unsupported.reasoning, None);
    }

    #[test]
    fn model_selection_defaults_missing_capabilities() {
        let selection: ModelSelection = serde_json::from_value(serde_json::json!({
            "provider": "lm-studio",
            "model": "publisher/model",
            "quantization": null
        }))
        .unwrap();
        assert!(!selection.vision);
        assert!(!selection.reasoning);
    }
}
