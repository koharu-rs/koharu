//! Configuration for the ViT/BERT vision-encoder-decoder checkpoint.
//!
//! Transformers source:
//! https://github.com/huggingface/transformers/blob/05fa1a7ac17bb7aa07b9e0c1e138ecb31a28bbfe/src/transformers/models/vision_encoder_decoder/configuration_vision_encoder_decoder.py

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct MangaOcrConfig {
    pub(super) encoder: ViTConfig,
    pub(super) decoder: BertConfig,
    pub(super) decoder_start_token_id: i64,
    pub(super) eos_token_id: i64,
    pub(super) pad_token_id: i64,
    pub(super) max_length: usize,
    pub(super) num_beams: usize,
    pub(super) length_penalty: f64,
    pub(super) early_stopping: bool,
    pub(super) no_repeat_ngram_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ViTConfig {
    pub(super) hidden_size: i64,
    pub(super) intermediate_size: i64,
    pub(super) num_hidden_layers: usize,
    pub(super) num_attention_heads: i64,
    pub(super) image_size: i64,
    pub(super) patch_size: i64,
    pub(super) num_channels: i64,
    pub(super) qkv_bias: bool,
    pub(super) layer_norm_eps: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct BertConfig {
    pub(super) hidden_size: i64,
    pub(super) intermediate_size: i64,
    pub(super) num_hidden_layers: usize,
    pub(super) num_attention_heads: i64,
    pub(super) max_position_embeddings: i64,
    pub(super) type_vocab_size: i64,
    pub(super) vocab_size: i64,
    pub(super) layer_norm_eps: f64,
    pub(super) pad_token_id: i64,
}
