//! manga-ocr-ar configuration:
//! https://huggingface.co/muscgab/manga-ocr-ar-experimental/blob/7873d8492307a021d6087b2c2b5f576a30b4acae/config.json

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
