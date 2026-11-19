//! `[pipeline]`: each stage's section, with its model and that model's
//! options, each defined by its stage. The app passes the section to
//! `Pipeline::new` and `set_config`, and a request may bring its own. An
//! invalid section fails to parse.

use koharu_scene::{GlossaryEntry, valid_glossary};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{DetectorConfig, InpainterConfig, RecognizerConfig, TranslatorConfig};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct PipelineConfig {
    pub detector: DetectorConfig,
    pub recognizer: RecognizerConfig,
    pub inpainter: InpainterConfig,
    pub translator: TranslatorConfig,
    pub glossary_analysis: crate::GlossaryAnalysisConfig,
    #[serde(deserialize_with = "glossary")]
    #[specta(type = Vec<GlossaryEntry>)]
    pub glossary: Vec<GlossaryEntry>,
}

impl PipelineConfig {
    pub(crate) fn glossary_model(&self) -> &str {
        self.glossary_analysis
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .unwrap_or(&self.translator.model)
    }
}

fn glossary<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<GlossaryEntry>, D::Error> {
    let entries = Vec::deserialize(deserializer)?;
    if !valid_glossary(&entries) {
        return Err(D::Error::custom(
            "invalid or duplicate global glossary entry",
        ));
    }
    Ok(entries)
}

impl koharu_config::Section for PipelineConfig {
    const NAME: &'static str = "pipeline";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_section_fails_to_parse() {
        let parse = |section: serde_json::Value| serde_json::from_value::<PipelineConfig>(section);
        assert!(parse(serde_json::json!({"detector": {"text": 0.3}})).is_ok());
        assert!(parse(serde_json::json!({"detector": {"text": 1.5}})).is_err());
        assert!(parse(serde_json::json!({"translator": {"target": "xx"}})).is_err());
    }
}
