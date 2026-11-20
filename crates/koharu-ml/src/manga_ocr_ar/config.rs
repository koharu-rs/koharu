//! manga-ocr-ar configuration:
//! https://huggingface.co/muscgab/manga-ocr-ar-experimental/blob/20208154a8d34dc6e5fc13ba81bc35f0a2f1a26e/config.json

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct Config {
    pub(super) patch_budget: i64,
    pub(super) max_length: i64,
    pub(super) encoder: EncoderConfig,
    pub(super) decoder: DecoderConfig,
}

#[derive(Debug, Deserialize)]
pub(super) struct EncoderConfig {
    pub(super) dim: i64,
    pub(super) heads: i64,
    pub(super) ffn: i64,
    pub(super) layers: i64,
    pub(super) rms_eps: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct DecoderConfig {
    pub(super) dim: i64,
    pub(super) heads: i64,
    pub(super) ffn: i64,
    pub(super) layers: i64,
}
