mod model;
mod processor;
mod types;

use std::{collections::VecDeque, ops::Range, path::Path};

use anyhow::{Context, Result, ensure};
use koharu_torch::Device;
use serde::Deserialize;

use crate::backend::TryIntoDevice;

use self::{
    model::{MAX_ENCODER_TOKENS, Model},
    processor::{Processor, decode_scores, resolve_overlaps, split_text, token_windows},
};

pub use types::{GlossaryEntity, GlossaryEntityKind};

pub struct Cancellation<'a> {
    cancelled: &'a (dyn Fn() -> bool + Sync),
}

impl<'a> Cancellation<'a> {
    #[must_use]
    pub fn new(cancelled: &'a (dyn Fn() -> bool + Sync)) -> Self {
        Self { cancelled }
    }

    #[must_use]
    pub fn never() -> Cancellation<'static> {
        Cancellation {
            cancelled: &never_cancelled,
        }
    }

    #[must_use]
    pub fn cancelled(&self) -> bool {
        (self.cancelled)()
    }
}

fn never_cancelled() -> bool {
    false
}

enum WindowEncoding<T> {
    Ready(T),
    Split(Range<usize>, Range<usize>),
}

fn run_windowed_extraction<T, O>(
    mut windows: VecDeque<Range<usize>>,
    cancellation: &Cancellation<'_>,
    mut encode: impl FnMut(Range<usize>) -> Result<WindowEncoding<T>>,
    mut infer: impl FnMut(Range<usize>, T) -> Result<Vec<O>>,
) -> Result<Option<Vec<O>>> {
    let mut output = Vec::new();
    while let Some(window) = windows.pop_front() {
        if cancellation.cancelled() {
            return Ok(None);
        }
        match encode(window.clone())? {
            WindowEncoding::Ready(encoded) => {
                if cancellation.cancelled() {
                    return Ok(None);
                }
                output.extend(infer(window, encoded)?);
            }
            WindowEncoding::Split(left, right) => {
                windows.push_front(right);
                windows.push_front(left);
            }
        }
    }
    Ok(Some(output))
}

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
    pub fn extract(
        &self,
        text: &str,
        confidence_threshold: f32,
        cancellation: &Cancellation<'_>,
    ) -> Result<Option<Vec<GlossaryEntity>>> {
        koharu_torch::no_grad(|| self.extract_inner(text, confidence_threshold, cancellation))
    }

    fn extract_inner(
        &self,
        text: &str,
        confidence_threshold: f32,
        cancellation: &Cancellation<'_>,
    ) -> Result<Option<Vec<GlossaryEntity>>> {
        ensure!(
            confidence_threshold.is_finite() && (0.0..=1.0).contains(&confidence_threshold),
            "glossary NER confidence threshold must be between 0 and 1"
        );
        let tokens = split_text(text);
        if tokens.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let Some(tokens) = self.processor.split_oversized_tokens(
            text,
            &tokens,
            MAX_ENCODER_TOKENS,
            cancellation,
        )?
        else {
            return Ok(None);
        };

        let windows = VecDeque::from(token_windows(tokens.len(), WINDOW_WORDS, WINDOW_OVERLAP)?);
        let Some(mut entities) = run_windowed_extraction(
            windows,
            cancellation,
            |window| {
                let encoded = self.processor.encode(text, &tokens[window.clone()])?;
                if !encoded.fits(MAX_ENCODER_TOKENS) && window.len() > 1 {
                    let midpoint = window.start + window.len() / 2;
                    let overlap = WINDOW_OVERLAP.min((window.len() / 2).saturating_sub(1));
                    return Ok(WindowEncoding::Split(
                        window.start..midpoint,
                        midpoint - overlap..window.end,
                    ));
                }
                ensure!(
                    encoded.fits(MAX_ENCODER_TOKENS),
                    "one glossary NER token exceeds the encoder capacity"
                );
                Ok(WindowEncoding::Ready(encoded))
            },
            |window, encoded| {
                let first_subtokens = encoded
                    .word_subtokens
                    .iter()
                    .map(|subtokens| subtokens.start)
                    .collect::<Vec<_>>();
                let probabilities = self.model.score(
                    &encoded.input_ids,
                    &first_subtokens,
                    encoded.prompt_words,
                    window.len(),
                )?;
                decode_scores(text, &tokens[window], &probabilities, confidence_threshold)
            },
        )?
        else {
            return Ok(None);
        };
        resolve_overlaps(&mut entities);
        Ok(Some(entities))
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

    #[test]
    fn cancellation_stops_multi_window_extraction_before_later_windows() -> Result<()> {
        use std::{
            collections::VecDeque,
            sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        };

        let cancelled = AtomicBool::new(false);
        let is_cancelled = || cancelled.load(Ordering::Relaxed);
        let cancellation = Cancellation::new(&is_cancelled);
        let encoded = AtomicUsize::new(0);
        let inferred = AtomicUsize::new(0);
        let windows = VecDeque::from(token_windows(300, WINDOW_WORDS, WINDOW_OVERLAP)?);

        let result = run_windowed_extraction(
            windows,
            &cancellation,
            |window| {
                encoded.fetch_add(1, Ordering::Relaxed);
                Ok(WindowEncoding::Ready(window))
            },
            |_window, encoded| {
                inferred.fetch_add(1, Ordering::Relaxed);
                cancelled.store(true, Ordering::Relaxed);
                Ok(vec![encoded])
            },
        )?;

        assert!(result.is_none());
        assert_eq!(encoded.load(Ordering::Relaxed), 1);
        assert_eq!(inferred.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "downloads a 1.1 GB checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_extracts_multilingual_entities() -> Result<()> {
        let runtime = koharu_runtime::Runtime::discover([koharu_runtime::Feature::Torch])?;
        runtime.initialize().await?;
        let ner = GlossaryNer::load(crate::Device::cpu()).await?;
        let cancellation = Cancellation::never();
        assert!(ner.extract("", 0.3, &cancellation)?.unwrap().is_empty());
        let entities = ner
            .extract("蒼井レンは東京へ向かった。", 0.3, &cancellation)?
            .unwrap();
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
