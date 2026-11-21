//! Baberu OCR and DINOv2 configuration.
//!
//! Upstream configuration:
//! https://huggingface.co/genshiai-daichi/baberu-ocr/blob/a0e3d4b1f98a8c5d0aba66ded831fe7b52bbf2fa/configuration_baberu.py
//! DINOv2 configuration used by `AutoModel.from_config`:
//! https://github.com/huggingface/transformers/blob/c6c8503869367af938666810e01a71866ca4fe93/src/transformers/models/dinov2/configuration_dinov2.py

use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct BaberuOcrConfig {
    pub(super) vision_model_name: Option<String>,
    pub(super) vision_hidden_size: i64,
    pub(super) vision_image_size: i64,
    pub(super) vision_patch_size: i64,
    pub(super) vision_num_tokens: i64,
    pub(super) projector_act: String,
    pub(super) vocab_size: i64,
    pub(super) hidden_size: i64,
    pub(super) intermediate_size: i64,
    pub(super) num_hidden_layers: usize,
    pub(super) num_attention_heads: i64,
    pub(super) num_key_value_heads: i64,
    pub(super) head_dim: Option<i64>,
    pub(super) max_position_embeddings: i64,
    pub(super) hidden_act: String,
    pub(super) rms_norm_eps: f64,
    pub(super) sandwich_norm: bool,
    pub(super) rope_theta: f64,
    pub(super) final_logit_softcap: Option<f64>,
    pub(super) attn_logit_softcap: Option<f64>,
    pub(super) tie_word_embeddings: bool,
    pub(super) pad_token_id: i64,
    pub(super) bos_token_id: i64,
    pub(super) eos_token_id: i64,
    pub(super) unk_token_id: i64,
}

impl BaberuOcrConfig {
    pub(super) fn head_dim(&self) -> i64 {
        self.head_dim
            .unwrap_or(self.hidden_size / self.num_attention_heads)
    }

    pub(super) fn validate(&self, vision: &Dinov2Config) -> Result<()> {
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
pub(super) struct BaberuGenerationConfig {
    pub(super) bos_token_id: i64,
    pub(super) eos_token_id: i64,
    pub(super) pad_token_id: i64,
    pub(super) max_new_tokens: usize,
    pub(super) repetition_penalty: f64,
}

impl BaberuGenerationConfig {
    pub(super) fn validate(&self, model: &BaberuOcrConfig) -> Result<()> {
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
pub(super) struct Dinov2Config {
    pub(super) hidden_size: i64,
    pub(super) num_hidden_layers: usize,
    pub(super) num_attention_heads: i64,
    pub(super) mlp_ratio: i64,
    pub(super) hidden_act: String,
    pub(super) layer_norm_eps: f64,
    pub(super) image_size: i64,
    pub(super) patch_size: i64,
    pub(super) num_channels: i64,
    pub(super) qkv_bias: bool,
    pub(super) use_swiglu_ffn: bool,
    pub(super) use_mask_token: bool,
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
