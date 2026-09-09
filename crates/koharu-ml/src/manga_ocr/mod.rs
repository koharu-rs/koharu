mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_torch::Device;

use crate::backend::TryIntoDevice;

pub use self::{config::MangaOcrConfig, processor::ViTImageProcessor};

use self::{model::Model, processor::Tokenizer};

crate::model_repository!("mayocream/manga-ocr" @ "4380edba990b959c508752350955350c1c80c31c" {
    CONFIG = "config.json",
    WEIGHTS = "model.safetensors",
    PROCESSOR = "preprocessor_config.json",
    VOCABULARY = "vocab.txt",
});

#[derive(Debug)]
pub struct MangaOcr {
    device: Device,
    model: Model,
    processor: ViTImageProcessor,
    tokenizer: Tokenizer,
}

impl MangaOcr {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let device: Device = device.try_into_device()?;
        let config_path = CONFIG
            .resolve()
            .await
            .context("failed to resolve Manga OCR config")?;
        let weights_path = WEIGHTS
            .resolve()
            .await
            .context("failed to resolve Manga OCR weights")?;
        let processor_path = PROCESSOR
            .resolve()
            .await
            .context("failed to resolve Manga OCR image processor")?;
        let vocabulary_path = VOCABULARY
            .resolve()
            .await
            .context("failed to resolve Manga OCR vocabulary")?;

        let config = MangaOcrConfig::from_file(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let processor = ViTImageProcessor::from_file(&processor_path)
            .with_context(|| format!("failed to read {}", processor_path.display()))?;
        let tokenizer = Tokenizer::from_file(&vocabulary_path)
            .with_context(|| format!("failed to read {}", vocabulary_path.display()))?;
        ensure!(
            tokenizer.len() == config.decoder.vocab_size as usize,
            "Manga OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.decoder.vocab_size
        );

        // The full ViT/BERT module tree must exist before VarStore loads checkpoint names.
        let mut model = Model::new(&config, device)?;
        model
            .load(&weights_path)
            .with_context(|| format!("failed to load {}", weights_path.display()))?;

        Ok(Self {
            device,
            model,
            processor,
            tokenizer,
        })
    }

    pub fn inference(&self, image: &DynamicImage) -> Result<String> {
        koharu_torch::no_grad(|| {
            let pixel_values = self.processor.preprocess(image, self.device)?;
            let token_ids = self.model.forward(&pixel_values)?;
            self.tokenizer.decode(&token_ids)
        })
    }
}
