//! manga-ocr-ar-experimental: crop-level OCR for Japanese manga. A 12-layer
//! MonkeyOCRv2-B vision tower's states at every depth are fused into one
//! memory, which a single-block autoregressive decoder reads, its output
//! weights tied to the token embedding. The checkpoint is licensed
//! CC BY-NC-SA 4.0.
//!
//! Upstream:
//! https://huggingface.co/muscgab/manga-ocr-ar-experimental/tree/7873d8492307a021d6087b2c2b5f576a30b4acae

mod config;
mod model;
mod processor;

use anyhow::{Context, Result};
use image::DynamicImage;
use koharu_runtime::Repository;

use self::{
    config::Config,
    model::Model,
    processor::{Vocabulary, preprocess},
};

const REPOSITORY: Repository = Repository::model(
    "muscgab/manga-ocr-ar-experimental",
    "7873d8492307a021d6087b2c2b5f576a30b4acae",
);

/// Upstream `MangaOCR.__call__`'s default.
const BATCH_SIZE: usize = 64;

#[derive(Debug)]
pub struct MangaOcrAr {
    model: Model,
    patch_budget: i64,
    vocabulary: Vocabulary,
}

impl MangaOcrAr {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device);
        let [config, weights, vocabulary] = REPOSITORY
            .resolve(["config.json", "model.safetensors", "vocab.json"])
            .await
            .context("failed to resolve manga-ocr-ar's files")?;
        let config: Config = serde_json::from_slice(&fs_err::read(&config)?)?;
        let vocabulary: Vocabulary = serde_json::from_slice(&fs_err::read(&vocabulary)?)?;

        let mut model = Model::new(&config, vocabulary.len(), device, kind)?;
        model.load(&weights)?;
        Ok(Self {
            model,
            patch_budget: config.patch_budget,
            vocabulary,
        })
    }

    /// The text of each crop.
    pub fn inference(&self, images: &[DynamicImage]) -> Result<Vec<String>> {
        koharu_torch::inference_mode(|| {
            let mut texts = Vec::with_capacity(images.len());
            for images in images.chunks(BATCH_SIZE) {
                let crops = images
                    .iter()
                    .map(|image| preprocess(image, self.patch_budget))
                    .collect::<Result<Vec<_>>>()?;
                let token_ids = self.model.generate(&crops)?;
                texts.extend(token_ids.iter().map(|ids| self.vocabulary.decode(ids)));
            }
            Ok(texts)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::MangaOcrAr;

    #[tokio::test]
    #[ignore = "downloads the checkpoint and requires the LibTorch runtime"]
    async fn checkpoint_matches_upstream_text() -> anyhow::Result<()> {
        let device = koharu_runtime::device().await?;
        let model = MangaOcrAr::load(device).await?;
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/ocr/title.png");
        let text = model.inference(&[image::open(input)?])?;
        // Verified against the pinned upstream `MangaOCR` on the identical
        // fixture crop.
        assert_eq!(text, ["対策委員会です!"]);
        Ok(())
    }
}
