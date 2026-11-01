//! Hayai OCR configuration.
//!
//! Upstream configuration:
//! https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2.5-nova/blob/2803d0dc005fa4396ab1e7c5a472d2c450a247f7/configuration_hayai.py
//! The vision encoder is the `Siglip2VisionModel` instantiated by
//! `modeling_hayai.HayaiModel` from `google/siglip2-base-patch16-naflex`, whose
//! released config only overrides the model type, so every value below is the
//! transformers default:
//! https://github.com/huggingface/transformers/blob/598d8ba8baaec7fec5a22da0e2844c7bf4ea20e1/src/transformers/models/siglip2/configuration_siglip2.py

use anyhow::{Result, ensure};
use serde::Deserialize;

/// Vision-encoder constants of `google/siglip2-base-patch16-naflex`.
pub(super) mod siglip2 {
    pub const HIDDEN_SIZE: i64 = 768;
    pub const INTERMEDIATE_SIZE: i64 = 3072;
    pub const NUM_HIDDEN_LAYERS: usize = 12;
    pub const NUM_ATTENTION_HEADS: i64 = 12;
    pub const HEAD_DIM: i64 = HIDDEN_SIZE / NUM_ATTENTION_HEADS;
    pub const NUM_CHANNELS: i64 = 3;
    pub const PATCH_SIZE: i64 = 16;
    /// NaFlex keeps a square base grid of position embeddings that is
    /// interpolated per image instead of forcing a fixed input resolution.
    pub const POSITION_EMBEDDING_SIZE: i64 = 16;
    pub const LAYER_NORM_EPS: f64 = 1e-6;
    /// Maximum number of 16x16 patches emitted by the SigLIP2 image processor:
    /// the budget the Hayai v2.5 model card calls the default.
    pub const MAX_NUM_PATCHES: usize = 384;
}

/// Decoder constants of `VisualCausalOCRDecoder`; upstream hardcodes them in
/// its layer construction rather than exposing them through the config file.
pub(super) mod decoder {
    pub const QUERY_HEADS: i64 = 8;
    pub const KEY_VALUE_HEADS: i64 = 2;
    pub const HEAD_DIM: i64 = 64;
    pub const ROPE_THETA: f64 = 10_000.0;
    /// `DSCProjector`'s `downscale_factor`: each 2x2 block of patches becomes
    /// one decoder token.
    pub const DOWNSCALE: i64 = 2;
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct HayaiConfig {
    pub(super) vocab_size: i64,
    pub(super) d_model: i64,
    pub(super) d_vision: i64,
    #[allow(dead_code)]
    pub(super) d_ffn: i64,
    pub(super) n_layers: usize,
    pub(super) num_ids_classes: i64,
}

impl HayaiConfig {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.vocab_size > 0,
            "Hayai OCR vocabulary size must be positive"
        );
        ensure!(
            self.d_vision == siglip2::HIDDEN_SIZE,
            "Hayai OCR expects the SigLIP2 NaFlex {}-dimensional vision encoder",
            siglip2::HIDDEN_SIZE
        );
        ensure!(
            self.d_model == decoder::QUERY_HEADS * decoder::HEAD_DIM,
            "decoder model size does not match the hardcoded query head layout"
        );
        ensure!(
            decoder::QUERY_HEADS % decoder::KEY_VALUE_HEADS == 0,
            "decoder query heads must be divisible by key/value heads"
        );
        ensure!(self.n_layers > 0, "decoder must have at least one layer");
        Ok(())
    }
}
