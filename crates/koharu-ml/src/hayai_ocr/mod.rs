//! Hayai OCR v2.5 Nova: crop-level OCR for Japanese, Chinese, Korean, and
//! English text. A SigLIP2 NaFlex encoder's 2x2 patch blocks are merged into
//! one token each by a projector, then a small GQA transformer decoder reads
//! them over a shared visual and textual prefix, with block-causal attention
//! and 2D multimodal RoPE.
//!
//! Upstream:
//! https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2.5-nova/tree/2803d0dc005fa4396ab1e7c5a472d2c450a247f7

mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_runtime::Repository;
use koharu_torch::Device;

use self::{
    config::HayaiConfig,
    model::Model,
    processor::{ImageProcessor, Tokenizer},
};

const REPOSITORY: Repository = Repository::model(
    "JustANormalTinkerer/hayai-ocr-v2.5-nova",
    "2803d0dc005fa4396ab1e7c5a472d2c450a247f7",
);

#[derive(Debug)]
pub struct HayaiOcr {
    device: Device,
    model: Model,
    processor: ImageProcessor,
    tokenizer: Tokenizer,
}

impl HayaiOcr {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device);
        let [config, weights, tokenizer] = REPOSITORY
            .resolve(["config.json", "model.safetensors", "tokenizer.json"])
            .await
            .context("failed to resolve Hayai OCR's files")?;
        let config: HayaiConfig = serde_json::from_slice(&fs_err::read(&config)?)?;
        config.validate()?;
        let tokenizer = Tokenizer::from_file(&tokenizer)?;
        ensure!(
            tokenizer.len() == config.vocab_size as usize,
            "Hayai OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.vocab_size
        );
        let processor = ImageProcessor::new(device)?;

        let mut model = Model::new(&config, &tokenizer, device, kind)?;
        model.load(&weights)?;
        Ok(Self {
            device,
            model,
            processor,
            tokenizer,
        })
    }

    /// The text of each crop, the crops decoded together.
    pub fn inference(&self, images: &[DynamicImage]) -> Result<Vec<String>> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        koharu_torch::no_grad(|| {
            let inputs = images
                .iter()
                .map(|image| self.processor.preprocess(image))
                .collect::<Result<Vec<_>>>()?;
            let token_ids = self.model.generate(&inputs, &self.tokenizer, self.device)?;
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

    use super::HayaiOcr;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_matches_upstream_character_sequence() -> anyhow::Result<()> {
        let device = koharu_runtime::device().await?;
        let model = HayaiOcr::load(device).await?;
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/ocr/title.png");
        let text = model.inference(&[image::open(input)?])?;
        // Verified against the pinned upstream `HayaiModel.generate` in greedy
        // mode on the identical fixture crop.
        assert_eq!(text, ["対策委員会です!"]);
        Ok(())
    }
}
