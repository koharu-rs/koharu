mod model;
mod processor;
mod types;

use std::{collections::VecDeque, path::Path};

use anyhow::{Context, Result, ensure};
use koharu_torch::Device;
use serde::Deserialize;

use crate::backend::TryIntoDevice;

use self::{
    model::{MAX_ENCODER_TOKENS, Model},
    processor::{Processor, decode_scores, resolve_overlaps, split_text, token_windows},
};

pub use types::{GlossaryEntity, GlossaryEntityKind};

crate::model_repository!("urchade/gliner_multi-v2.1" @ "443d26d654e0324125a96bebd8e796c14ff2efe6" {
    CONFIG = "gliner_config.json",
    WEIGHTS = "model.safetensors",
});
crate::model_repository!("MoritzLaurer/mDeBERTa-v3-base-mnli-xnli" @ "8adb042d524ecd5c26d3e3ba0e3fbcf7e2d0864c" {
    TOKENIZER = "tokenizer.json",
});

const WINDOW_WORDS: usize = 128;
const WINDOW_OVERLAP: usize = 32;

#[derive(Debug, Deserialize)]
struct CheckpointConfig {
    max_width: usize,
    max_len: usize,
    hidden_size: usize,
    model_name: String,
    span_mode: String,
    subtoken_pooling: String,
}

impl CheckpointConfig {
    fn from_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let config: Self = serde_json::from_slice(&bytes)?;
        ensure!(
            config.max_width == 12
                && config.max_len == 384
                && config.hidden_size == 512
                && config.model_name == "microsoft/mdeberta-v3-base"
                && config.span_mode == "markerV0"
                && config.subtoken_pooling == "first",
            "pinned GLiNER configuration does not match the local architecture"
        );
        Ok(config)
    }
}

/// Fully local multilingual glossary entity extraction.
///
/// The network is a Rust/Torch port of `urchade/GLiNER` commit
/// `96350f0bafcc9ccf8a78ecab393c21634d9a58ce`. The Apache-2.0
/// `urchade/gliner_multi-v2.1` checkpoint is pinned above. Its mDeBERTa
/// tokenizer is supplied by the commit-pinned MIT-licensed mirror above.
#[derive(Debug)]
pub struct GlossaryNer {
    model: Model,
    processor: Processor,
}

impl GlossaryNer {
    /// Resolves the pinned files and initializes all weights directly on the
    /// selected device. The device abstraction is converted exactly once.
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let (config_path, weights_path, tokenizer_path) = tokio::try_join!(
            async {
                CONFIG
                    .resolve()
                    .await
                    .context("failed to resolve GLiNER config")
            },
            async {
                WEIGHTS
                    .resolve()
                    .await
                    .context("failed to resolve GLiNER weights")
            },
            async {
                TOKENIZER
                    .resolve()
                    .await
                    .context("failed to resolve mDeBERTa tokenizer")
            },
        )?;
        Self::from_files(device, &config_path, &weights_path, &tokenizer_path)
    }

    fn from_files(
        device: Device,
        config_path: &Path,
        weights_path: &Path,
        tokenizer_path: &Path,
    ) -> Result<Self> {
        CheckpointConfig::from_file(config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = Processor::from_file(tokenizer_path)?;
        let mut model = Model::new(device)?;
        model
            .load(weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        Ok(Self { model, processor })
    }

    /// Extracts flat, non-overlapping glossary entities. Offsets are UTF-8
    /// byte offsets into `text`; `end` is exclusive.
    pub fn extract(&self, text: &str, confidence_threshold: f32) -> Result<Vec<GlossaryEntity>> {
        koharu_torch::no_grad(|| self.extract_inner(text, confidence_threshold))
    }

    fn extract_inner(&self, text: &str, confidence_threshold: f32) -> Result<Vec<GlossaryEntity>> {
        ensure!(
            confidence_threshold.is_finite() && (0.0..=1.0).contains(&confidence_threshold),
            "glossary NER confidence threshold must be between 0 and 1"
        );
        let tokens = split_text(text);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let mut entities = Vec::new();
        let mut windows =
            VecDeque::from(token_windows(tokens.len(), WINDOW_WORDS, WINDOW_OVERLAP)?);
        while let Some(window) = windows.pop_front() {
            let mut encoded = self.processor.encode(text, &tokens[window.clone()])?;
            if encoded.input_ids.len() > MAX_ENCODER_TOKENS && window.len() > 1 {
                let midpoint = window.start + window.len() / 2;
                let overlap = WINDOW_OVERLAP.min((window.len() / 2).saturating_sub(1));
                windows.push_front(midpoint - overlap..window.end);
                windows.push_front(window.start..midpoint);
                continue;
            }
            if encoded.input_ids.len() > MAX_ENCODER_TOKENS {
                encoded.input_ids.truncate(MAX_ENCODER_TOKENS);
                ensure!(
                    encoded
                        .first_subtokens
                        .iter()
                        .all(|&index| index < MAX_ENCODER_TOKENS),
                    "one glossary NER token exceeds the encoder capacity"
                );
            }
            let text_words = window.len();
            let probabilities = self.model.score(
                &encoded.input_ids,
                &encoded.first_subtokens,
                encoded.prompt_words,
                text_words,
            )?;
            entities.extend(decode_scores(
                text,
                &tokens[window],
                &probabilities,
                confidence_threshold,
            )?);
        }
        resolve_overlaps(&mut entities);
        Ok(entities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_checkpoint_configuration_is_accepted() {
        let config: CheckpointConfig = serde_json::from_str(
            r#"{
                "max_width": 12,
                "max_len": 384,
                "hidden_size": 512,
                "model_name": "microsoft/mdeberta-v3-base",
                "span_mode": "markerV0",
                "subtoken_pooling": "first"
            }"#,
        )
        .unwrap();
        assert_eq!(config.max_width, 12);
        assert_eq!(config.model_name, "microsoft/mdeberta-v3-base");
    }

    #[tokio::test]
    #[ignore = "downloads a 1.1 GB checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_extracts_multilingual_entities() -> Result<()> {
        let runtime = koharu_runtime::Runtime::discover([koharu_runtime::Feature::Torch])?;
        runtime.initialize().await?;
        let ner = GlossaryNer::load(crate::Device::cpu()).await?;
        assert!(ner.extract("", 0.3)?.is_empty());
        let entities = ner.extract("蒼井レンは東京へ向かった。", 0.3)?;
        assert!(!entities.is_empty());
        assert!(entities.iter().all(|entity| {
            entity.start < entity.end
                && text_is_boundary("蒼井レンは東京へ向かった。", entity.start, entity.end)
        }));
        Ok(())
    }

    fn text_is_boundary(text: &str, start: usize, end: usize) -> bool {
        text.is_char_boundary(start) && text.is_char_boundary(end)
    }
}
