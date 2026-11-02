//! Manga OCR: a ViT encoder and a BERT decoder reading one line or bubble of
//! Japanese manga text.
//!
//! Upstream:
//! https://github.com/kha-white/manga-ocr/blob/847e0939fcb391ad63584dcf50ba6c6533ec9ee8/manga_ocr/ocr.py

mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_runtime::Repository;
use koharu_torch::{Device, Tensor};

use self::{
    config::MangaOcrConfig,
    model::Model,
    processor::{Tokenizer, ViTImageProcessor},
};

const REPOSITORY: Repository = Repository::model(
    "mayocream/manga-ocr",
    "4380edba990b959c508752350955350c1c80c31c",
);

#[derive(Debug)]
pub struct MangaOcr {
    device: Device,
    model: Model,
    processor: ViTImageProcessor,
    tokenizer: Tokenizer,
}

impl MangaOcr {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device)?;
        let [config, weights, processor, vocabulary] = REPOSITORY
            .resolve([
                "config.json",
                "model.safetensors",
                "preprocessor_config.json",
                "vocab.txt",
            ])
            .await
            .context("failed to resolve Manga OCR's files")?;
        let config: MangaOcrConfig = serde_json::from_slice(&fs_err::read(&config)?)?;
        let processor: ViTImageProcessor = serde_json::from_slice(&fs_err::read(&processor)?)?;
        let tokenizer = Tokenizer::from_file(&vocabulary)?;
        ensure!(
            tokenizer.len() == config.decoder.vocab_size as usize,
            "Manga OCR vocabulary has {} entries but the decoder has {} outputs",
            tokenizer.len(),
            config.decoder.vocab_size
        );

        // The full ViT/BERT module tree must exist before VarStore loads checkpoint names.
        let mut model = Model::new(&config, device, kind)?;
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
            let pixel_values = images
                .iter()
                .map(|image| self.processor.preprocess(image, self.device))
                .collect::<Result<Vec<_>>>()?;
            let token_ids = self.model.forward(&Tensor::cat(&pixel_values, 0)?)?;
            token_ids
                .iter()
                .map(|token_ids| self.tokenizer.decode(token_ids))
                .collect()
        })
    }
}
