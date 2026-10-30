mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_runtime::Repository;
use koharu_torch::Tensor;

pub use self::config::BaberuOcrConfig;

use self::{
    config::{BaberuGenerationConfig, Dinov2Config},
    model::Model,
    processor::{BaberuImageProcessor, Tokenizer},
};

const BABERU: Repository = Repository::model(
    "genshiai-daichi/baberu-ocr",
    "d9cc13153e9a1cd8fdfa3b7b1cc329da2020aeae",
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
        let (device, kind) = crate::backend::torch(device)?;
        let [config_path, generation_path, weights_path, vocabulary_path] = BABERU
            .resolve([
                "config.json",
                "generation_config.json",
                "model.safetensors",
                "tokenizer/vocab.json",
            ])
            .await
            .context("failed to resolve Baberu OCR files")?;
        let [vision_config_path, vision_processor_path] = DINOV2
            .resolve(["config.json", "preprocessor_config.json"])
            .await
            .context("failed to resolve DINOv2 files")?;

        let config = BaberuOcrConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let generation = BaberuGenerationConfig::from_file(&generation_path)
            .with_context(|| format!("failed to read {}", generation_path.display()))?;
        let vision_config = Dinov2Config::from_file(&vision_config_path)
            .with_context(|| format!("failed to read {}", vision_config_path.display()))?;
        config.validate(&vision_config)?;
        generation.validate(&config)?;

        let tokenizer = Tokenizer::from_file(&vocabulary_path)
            .with_context(|| format!("failed to read {}", vocabulary_path.display()))?;
        ensure!(
            tokenizer.len() == config.vocab_size as usize,
            "Baberu OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.vocab_size
        );
        let processor = BaberuImageProcessor::from_file(
            &vision_processor_path,
            config.vision_image_size,
            device,
        )
        .with_context(|| format!("failed to read {}", vision_processor_path.display()))?;

        // The complete DINOv2/projector/decoder tree must exist before loading
        // checkpoint names, including the unused DINOv2 mask token.
        let mut model = Model::new(&config, &vision_config, device, kind)?;
        model
            .load(&weights_path, config.vision_image_size)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;

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
                &Tensor::f_cat(&pixel_values, 0)?,
                &self.tokenizer,
                self.generation.max_new_tokens,
                self.generation.repetition_penalty,
                12,
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
