//! Baberu OCR and DINOv2 configuration.
//!
//! Upstream configuration:
//! https://huggingface.co/genshiai-daichi/baberu-ocr/blob/d9cc13153e9a1cd8fdfa3b7b1cc329da2020aeae/configuration_baberu.py
//! DINOv2 configuration used by `AutoModel.from_config`:
//! https://github.com/huggingface/transformers/blob/c6c8503869367af938666810e01a71866ca4fe93/src/transformers/models/dinov2/configuration_dinov2.py

use std::{fs, path::Path};

use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct BaberuOcrConfig {
    pub(crate) vision_model_name: Option<String>,
    pub(crate) vision_hidden_size: i64,
    pub(crate) vision_image_size: i64,
    pub(crate) vision_patch_size: i64,
    pub(crate) vision_num_tokens: i64,
    pub(crate) projector_act: String,
    pub(crate) vocab_size: i64,
    pub(crate) hidden_size: i64,
    pub(crate) intermediate_size: i64,
    pub(crate) num_hidden_layers: usize,
    pub(crate) num_attention_heads: i64,
    pub(crate) num_key_value_heads: i64,
    pub(crate) head_dim: Option<i64>,
    pub(crate) max_position_embeddings: i64,
    pub(crate) hidden_act: String,
    pub(crate) rms_norm_eps: f64,
    pub(crate) sandwich_norm: bool,
    pub(crate) rope_theta: f64,
    pub(crate) final_logit_softcap: Option<f64>,
    pub(crate) attn_logit_softcap: Option<f64>,
    pub(crate) tie_word_embeddings: bool,
    pub(crate) pad_token_id: i64,
    pub(crate) bos_token_id: i64,
    pub(crate) eos_token_id: i64,
    pub(crate) unk_token_id: i64,
}

impl BaberuOcrConfig {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub(crate) fn head_dim(&self) -> i64 {
        self.head_dim
            .unwrap_or(self.hidden_size / self.num_attention_heads)
    }

    pub(crate) fn validate(&self, vision: &Dinov2Config) -> Result<()> {
        ensure!(
            self.vision_model_name.as_deref() == Some("facebook/dinov2-base"),
            "unsupported Baberu OCR vision model {:?}",
            self.vision_model_name
        );
        ensure!(
            self.projector_act == "gelu",
            "unsupported projector activation"
        );
        ensure!(self.hidden_act == "silu", "unsupported decoder activation");
        ensure!(
            self.tie_word_embeddings,
            "the released Baberu OCR checkpoint requires tied word embeddings"
        );
        ensure!(
            [
                self.pad_token_id,
                self.bos_token_id,
                self.eos_token_id,
                self.unk_token_id
            ] == [0, 1, 2, 3],
            "unsupported Baberu OCR special-token layout"
        );
        ensure!(
            self.num_attention_heads % self.num_key_value_heads == 0,
            "decoder query heads must be divisible by key/value heads"
        );
        ensure!(
            self.head_dim() * self.num_attention_heads == self.hidden_size,
            "decoder head dimensions do not equal its hidden size"
        );
        ensure!(
            self.vision_hidden_size == vision.hidden_size,
            "Baberu OCR and DINOv2 hidden sizes do not match"
        );
        ensure!(
            self.vision_patch_size == vision.patch_size,
            "Baberu OCR and DINOv2 patch sizes do not match"
        );
        ensure!(
            self.vision_image_size % self.vision_patch_size == 0,
            "Baberu OCR image size is not divisible by its patch size"
        );
        ensure!(
            self.vision_num_tokens == (self.vision_image_size / self.vision_patch_size).pow(2),
            "Baberu OCR vision token count is inconsistent"
        );
        ensure!(
            vision.hidden_size % vision.num_attention_heads == 0,
            "DINOv2 hidden size is not divisible by its attention heads"
        );
        ensure!(vision.hidden_act == "gelu", "unsupported DINOv2 activation");
        ensure!(!vision.use_swiglu_ffn, "DINOv2 SwiGLU is not supported");
        ensure!(
            vision.use_mask_token,
            "the DINOv2 checkpoint contains a mask token"
        );
        ensure!(
            self.max_position_embeddings > self.vision_num_tokens,
            "decoder position table cannot contain the vision prefix"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BaberuGenerationConfig {
    pub(crate) bos_token_id: i64,
    pub(crate) eos_token_id: i64,
    pub(crate) pad_token_id: i64,
    pub(crate) max_new_tokens: usize,
    pub(crate) repetition_penalty: f64,
}

impl BaberuGenerationConfig {
    pub(crate) fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub(crate) fn validate(&self, model: &BaberuOcrConfig) -> Result<()> {
        ensure!(
            self.bos_token_id == model.bos_token_id,
            "BOS token IDs differ"
        );
        ensure!(
            self.eos_token_id == model.eos_token_id,
            "EOS token IDs differ"
        );
        ensure!(
            self.pad_token_id == model.pad_token_id,
            "pad token IDs differ"
        );
        ensure!(
            self.repetition_penalty > 0.0,
            "repetition penalty must be positive"
        );
        ensure!(
            model.vision_num_tokens + 1 + self.max_new_tokens as i64
                <= model.max_position_embeddings,
            "generation can exceed the decoder position table"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct Dinov2Config {
    pub(crate) hidden_size: i64,
    pub(crate) num_hidden_layers: usize,
    pub(crate) num_attention_heads: i64,
    pub(crate) mlp_ratio: i64,
    pub(crate) hidden_act: String,
    pub(crate) layer_norm_eps: f64,
    pub(crate) image_size: i64,
    pub(crate) patch_size: i64,
    pub(crate) num_channels: i64,
    pub(crate) qkv_bias: bool,
    pub(crate) use_swiglu_ffn: bool,
    pub(crate) use_mask_token: bool,
}

impl Dinov2Config {
    pub(crate) fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }
}

impl Default for Dinov2Config {
    fn default() -> Self {
        Self {
            hidden_size: 768,
            num_hidden_layers: 12,
            num_attention_heads: 12,
            mlp_ratio: 4,
            hidden_act: "gelu".to_owned(),
            layer_norm_eps: 1e-6,
            image_size: 224,
            patch_size: 14,
            num_channels: 3,
            qkv_bias: true,
            use_swiglu_ffn: false,
            use_mask_token: true,
        }
    }
}
