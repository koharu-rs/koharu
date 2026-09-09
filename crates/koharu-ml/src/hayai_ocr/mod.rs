mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

use self::{
    config::HayaiConfig,
    model::Model,
    processor::{ImageProcessor, Tokenizer},
};

crate::model_repository!("JustANormalTinkerer/hayai-ocr-v2" @ "4a4ce477c9a8841f208b94e1d9ed5c0938965e05" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
    TOKENIZER = "tokenizer.json",
});

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
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let (config_path, weights_path, tokenizer_path) = tokio::try_join!(
            async {
                CONFIG
                    .resolve()
                    .await
                    .context("failed to resolve Hayai OCR config")
            },
            async {
                WEIGHTS
                    .resolve()
                    .await
                    .context("failed to resolve Hayai OCR weights")
            },
            async {
                TOKENIZER
                    .resolve()
                    .await
                    .context("failed to resolve Hayai OCR tokenizer")
            },
        )?;

        let config = HayaiConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path)?;
        ensure!(
            tokenizer.len() == config.vocab_size as usize,
            "Hayai OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.vocab_size
        );

        let mut model = Model::new(&config, &tokenizer, device)?;
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

    pub fn inference(&self, image: &DynamicImage) -> Result<String> {
        koharu_torch::no_grad(|| {
            let input = self.processor.preprocess(image)?;
            let token_ids = self.model.generate(&input, &self.tokenizer, self.device)?;
            self.tokenizer.decode(&token_ids)
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
        crate::init().await?;
        let model = HayaiOcr::load(crate::Device::cpu()).await?;
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/ocr/title.png");
        let text = model.inference(&image::open(input)?)?;
        // Verified against the pinned upstream `HayaiModel.generate` in greedy
        // mode on the identical fixture crop.
        assert_eq!(text, "対策委員会です!");
        Ok(())
    }
}
