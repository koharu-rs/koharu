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
    "JustANormalTinkerer/hayai-ocr-v2",
    "4a4ce477c9a8841f208b94e1d9ed5c0938965e05",
);

/// Fast crop-level OCR for Japanese, Chinese, Korean, and English text.
///
/// Port of the Hayai OCR v2 vision-to-text model: a SigLIP2 NaFlex encoder
/// paired with a small GQA transformer decoder over a shared visual/textual
/// prefix with block-causal attention and 2D multimodal RoPE.
#[derive(Debug)]
pub struct HayaiOcr {
    model: Model,
    processor: ImageProcessor,
    tokenizer: Tokenizer,
    device: Device,
}

impl HayaiOcr {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device)?;
        let [config_path, weights_path, tokenizer_path] = REPOSITORY
            .resolve(["config.json", "model.safetensors", "tokenizer.json"])
            .await
            .context("failed to resolve Hayai OCR files")?;

        let config = HayaiConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path)?;
        ensure!(
            tokenizer.len() == config.vocab_size as usize,
            "Hayai OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.vocab_size
        );

        let mut model = Model::new(&config, &tokenizer, device, kind)?;
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;
        let processor = ImageProcessor::new(device)?;

        Ok(Self {
            model,
            processor,
            tokenizer,
            device,
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
