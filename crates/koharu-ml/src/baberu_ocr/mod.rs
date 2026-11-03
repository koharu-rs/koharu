//! Baberu OCR: a DINOv2 encoder, a projector, and a small decoder reading a
//! speech bubble of Japanese manga text, character by character.
//!
//! Upstream, at its `v1.2` tag:
//! https://huggingface.co/genshiai-daichi/baberu-ocr/tree/a0e3d4b1f98a8c5d0aba66ded831fe7b52bbf2fa

mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_runtime::Repository;
use koharu_torch::Tensor;

use self::{
    config::{BaberuGenerationConfig, BaberuOcrConfig, Dinov2Config},
    model::Model,
    processor::{BaberuImageProcessor, BitImageProcessorConfig, Tokenizer},
};

/// The `v1.2` tag.
const BABERU: Repository = Repository::model(
    "genshiai-daichi/baberu-ocr",
    "a0e3d4b1f98a8c5d0aba66ded831fe7b52bbf2fa",
);
const DINOV2: Repository = Repository::model(
    "facebook/dinov2-base",
    "f9e44c814b77203eaa57a6bdbbd535f21ede1415",
);

#[derive(Debug)]
pub struct BaberuOcr {
    model: Model,
    processor: BaberuImageProcessor,
    tokenizer: Tokenizer,
    generation: BaberuGenerationConfig,
}

impl BaberuOcr {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device);
        let [config, generation, weights, vocabulary] = BABERU
            .resolve([
                "config.json",
                "generation_config.json",
                "model.safetensors",
                "tokenizer/vocab.json",
            ])
            .await
            .context("failed to resolve Baberu OCR's files")?;
        let [vision_config, vision_processor] = DINOV2
            .resolve(["config.json", "preprocessor_config.json"])
            .await
            .context("failed to resolve DINOv2's files")?;
        let config: BaberuOcrConfig = serde_json::from_slice(&fs_err::read(&config)?)?;
        let generation: BaberuGenerationConfig =
            serde_json::from_slice(&fs_err::read(&generation)?)?;
        let vision_config: Dinov2Config = serde_json::from_slice(&fs_err::read(&vision_config)?)?;
        let vision_processor: BitImageProcessorConfig =
            serde_json::from_slice(&fs_err::read(&vision_processor)?)?;
        config.validate(&vision_config)?;
        generation.validate(&config)?;
        let tokenizer = Tokenizer::from_file(&vocabulary)?;
        ensure!(
            tokenizer.len() == config.vocab_size as usize,
            "Baberu OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.vocab_size
        );
        let processor =
            BaberuImageProcessor::new(&vision_processor, config.vision_image_size, device)?;

        // The complete DINOv2/projector/decoder tree must exist before loading
        // checkpoint names, including the unused DINOv2 mask token.
        let mut model = Model::new(&config, &vision_config, device, kind)?;
        model.load(&weights, config.vision_image_size)?;
        Ok(Self {
            model,
            processor,
            tokenizer,
            generation,
        })
    }

    /// The text of each crop, the crops decoded together.
    pub fn inference(&self, images: &[DynamicImage]) -> Result<Vec<String>> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        koharu_torch::no_grad(|| {
            let pixel_values = images
                .iter()
                .map(|image| self.processor.preprocess(image))
                .collect::<Result<Vec<_>>>()?;
            let token_ids = self.model.forward(
                &Tensor::cat(&pixel_values, 0)?,
                &self.tokenizer,
                self.generation.max_new_tokens,
                self.generation.repetition_penalty,
                // `BaberuOCR.__call__`'s `max_content_run` and `max_symbol_run`.
                12,
                16,
            )?;
            token_ids
                .iter()
                .map(|token_ids| self.tokenizer.decode(token_ids))
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::BaberuOcr;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_matches_upstream_character_sequence() -> anyhow::Result<()> {
        let device = koharu_runtime::device().await?;
        let model = BaberuOcr::load(device).await?;
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/ocr/title.png");
        let text = model.inference(&[image::open(input)?])?;
        assert_eq!(text, ["対策委員会です！"]);
        Ok(())
    }
}
