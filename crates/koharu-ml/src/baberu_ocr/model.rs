//! Inference-only Baberu OCR network.
//!
//! Baberu architecture and generation implementation:
//! https://huggingface.co/genshiai-daichi/baberu-ocr/blob/d9cc13153e9a1cd8fdfa3b7b1cc329da2020aeae/modeling_baberu.py
//! DINOv2 implementation instantiated by the upstream `AutoModel` call:
//! https://github.com/huggingface/transformers/blob/c6c8503869367af938666810e01a71866ca4fe93/src/transformers/models/dinov2/modeling_dinov2.py

use std::path::Path;

use anyhow::{Result, ensure};
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module},
};

use super::{
    config::{BaberuOcrConfig, Dinov2Config},
    processor::Tokenizer,
};

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    vision_encoder: Dinov2Model,
    projector: BaberuVisionProjector,
    model: BaberuModel,
    bos_token_id: i64,
    eos_token_id: i64,
    vision_image_size: i64,
    vision_num_tokens: i64,
    final_logit_softcap: Option<f64>,
}

impl Model {
    pub(super) fn new(
        config: &BaberuOcrConfig,
        vision_config: &Dinov2Config,
        device: Device,
        kind: Kind,
    ) -> Result<Self> {
        config.validate(vision_config)?;
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();
        let vision_encoder = Dinov2Model::new(&(&root / "vision_encoder"), vision_config)?;
        let projector = BaberuVisionProjector::new(&(&root / "projector"), config)?;
        let model = BaberuModel::new(&(&root / "model"), config, device, kind)?;
        Ok(Self {
            vs,
            vision_encoder,
            projector,
            model,
            bos_token_id: config.bos_token_id,
            eos_token_id: config.eos_token_id,
            vision_image_size: config.vision_image_size,
            vision_num_tokens: config.vision_num_tokens,
            final_logit_softcap: config.final_logit_softcap,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>, image_size: i64) -> Result<()> {
        self.vs.load(path)?;
        // The processor always emits one fixed 224x224 crop, so cache the exact
        // DINOv2 bicubic position interpolation instead of repeating it per crop.
        self.vision_encoder
            .cache_position_embeddings(image_size, image_size)?;
        Ok(())
    }

    /// Greedy decoding for each crop of `pixel_values`, `[batch, 3, size,
    /// size]`, the crops decoded together. Their prefixes, the vision tokens
    /// and BOS, are the same length, so no row is padded.
    pub(super) fn forward(
        &self,
        pixel_values: &Tensor,
        tokenizer: &Tokenizer,
        max_new_tokens: usize,
        repetition_penalty: f64,
        max_content_run: usize,
    ) -> Result<Vec<Vec<i64>>> {
        let size = pixel_values.size();
        ensure!(
            size.len() == 4
                && size[0] > 0
                && size[1..] == [3, self.vision_image_size, self.vision_image_size],
            "Baberu OCR expects RGB {}x{} crops, got {:?}",
            self.vision_image_size,
            self.vision_image_size,
            size
        );
        let rows = size[0] as usize;
        let device = pixel_values.device();

        let pixel_values = pixel_values.to_kind(self.vs.kind())?;
        let vision_hidden_states = self.vision_encoder.forward(&pixel_values)?;
        let vision_embeds = self.projector.forward(&vision_hidden_states.narrow(
            1,
            1,
            self.vision_num_tokens,
        )?)?;
        let bos = Tensor::full([rows as i64, 1], self.bos_token_id, (Kind::Int64, device))?;
        let text_embeds = self.model.embed_tokens.forward(&bos)?;
        let inputs_embeds = Tensor::cat(&[vision_embeds, text_embeds], 1)?;
        let (hidden_states, mut cache) = self.model.forward(inputs_embeds, None)?;
        let mut logits = self.logits(&hidden_states)?;

        let mut tokens = vec![Vec::with_capacity(max_new_tokens); rows];
        let mut finished = vec![false; rows];
        // Each row's ids in the order first seen, BOS first.
        let mut seen_ids = vec![vec![self.bos_token_id]; rows];

        for _ in 0..max_new_tokens {
            if repetition_penalty != 1.0 {
                let ids = crate::penalty_ids(&seen_ids, device)?;
                logits = crate::repetition_penalty(&logits, &ids, repetition_penalty)?;
            }
            if max_content_run != 0 {
                // A content token repeated max_content_run times can't come
                // again.
                let banned = tokens
                    .iter()
                    .map(|tokens| {
                        tokens.last().is_some_and(|&token_id| {
                            tokenizer.is_content(token_id)
                                && tokens
                                    .iter()
                                    .rev()
                                    .take_while(|candidate| **candidate == token_id)
                                    .count()
                                    >= max_content_run
                        })
                    })
                    .collect::<Vec<_>>();
                if banned.iter().any(|banned| *banned) {
                    let last_ids = tokens
                        .iter()
                        .map(|tokens| tokens.last().copied().unwrap_or(self.bos_token_id))
                        .collect::<Vec<_>>();
                    let last_ids = Tensor::from_slice(&last_ids)?
                        .view([rows as i64, 1])?
                        .to_device(device)?;
                    let banned = Tensor::from_slice(&banned)?
                        .view([rows as i64, 1])?
                        .to_device(device)?;
                    logits = logits
                        .scatter_value(1, &last_ids, f64::NEG_INFINITY)?
                        .where_self(&banned, &logits)?;
                }
            }

            let next_tokens = Vec::<i64>::try_from(logits.argmax(-1, false)?)?;
            for (row, &next_token) in next_tokens.iter().enumerate() {
                if finished[row] {
                    continue;
                }
                if next_token == self.eos_token_id {
                    finished[row] = true;
                    continue;
                }
                tokens[row].push(next_token);
                if tokens[row].len() == max_new_tokens {
                    finished[row] = true;
                } else if !seen_ids[row].contains(&next_token) {
                    seen_ids[row].push(next_token);
                }
            }
            if finished.iter().all(|finished| *finished) {
                break;
            }

            let next_ids = Tensor::from_slice(&next_tokens)?
                .view([rows as i64, 1])?
                .to_device(device)?;
            let input_embeds = self.model.embed_tokens.forward(&next_ids)?;
            let (hidden_states, next_cache) = self.model.forward(input_embeds, Some(&cache))?;
            cache = next_cache;
            logits = self.logits(&hidden_states)?;
        }
        Ok(tokens)
    }

    fn logits(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        // Generation only consumes the final position. Applying the tied head to
        // that row avoids materializing 256 unused vision-prefix logit rows.
        let last_hidden_state = hidden_states.select(1, hidden_states.size()[1] - 1)?;
        let logits = last_hidden_state.linear(&self.model.embed_tokens.ws, None::<&Tensor>)?;
        if let Some(cap) = self.final_logit_softcap {
            (logits / cap)?.tanh()? * cap
        } else {
            Ok(logits)
        }
    }
}

#[derive(Debug)]
struct Dinov2Model {
    embeddings: Dinov2Embeddings,
    encoder: Dinov2Encoder,
    layernorm: nn::LayerNorm,
}

impl Dinov2Model {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            embeddings: Dinov2Embeddings::new(&(path / "embeddings"), config)?,
            encoder: Dinov2Encoder::new(&(path / "encoder"), config)?,
            layernorm: layer_norm(
                &(path / "layernorm"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
        })
    }

    fn cache_position_embeddings(&mut self, height: i64, width: i64) -> Result<()> {
        self.embeddings.cache_position_embeddings(height, width)
    }

    fn forward(&self, pixel_values: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = self.embeddings.forward(pixel_values)?;
        self.layernorm
            .forward(&self.encoder.forward(hidden_states)?)
    }
}

#[derive(Debug)]
struct Dinov2Embeddings {
    cls_token: Tensor,
    _mask_token: Tensor,
    patch_embeddings: Dinov2PatchEmbeddings,
    position_embeddings: Tensor,
    interpolated_position_embeddings: Option<Tensor>,
    patch_size: i64,
}

impl Dinov2Embeddings {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        let patches_per_side = config.image_size / config.patch_size;
        Ok(Self {
            cls_token: path.var("cls_token", &[1, 1, config.hidden_size])?,
            _mask_token: path.var("mask_token", &[1, config.hidden_size])?,
            patch_embeddings: Dinov2PatchEmbeddings::new(&(path / "patch_embeddings"), config)?,
            position_embeddings: path.var(
                "position_embeddings",
                &[
                    1,
                    patches_per_side * patches_per_side + 1,
                    config.hidden_size,
                ],
            )?,
            interpolated_position_embeddings: None,
            patch_size: config.patch_size,
        })
    }

    fn cache_position_embeddings(&mut self, height: i64, width: i64) -> Result<()> {
        let num_positions = self.position_embeddings.size()[1] - 1;
        let new_height = height / self.patch_size;
        let new_width = width / self.patch_size;
        let position_embeddings = if new_height * new_width == num_positions && height == width {
            self.position_embeddings.alias()?
        } else {
            let positions_per_side = (num_positions as f64).sqrt() as i64;
            let class_position = self.position_embeddings.narrow(1, 0, 1)?;
            let target_kind = self.position_embeddings.kind();
            let patch_positions = self
                .position_embeddings
                .narrow(1, 1, num_positions)?
                .reshape([1, positions_per_side, positions_per_side, -1])?
                .permute([0, 3, 1, 2])?
                .to_kind(Kind::Float)?
                .upsample_bicubic2d([new_height, new_width], false, None::<f64>, None::<f64>)?
                .to_kind(target_kind)?
                .permute([0, 2, 3, 1])?
                .reshape([1, -1, self.position_embeddings.size()[2]])?;
            Tensor::cat(&[class_position, patch_positions], 1)?
        };
        self.interpolated_position_embeddings = Some(position_embeddings);
        Ok(())
    }

    fn forward(&self, pixel_values: &Tensor) -> koharu_torch::Result<Tensor> {
        let batch_size = pixel_values.size()[0];
        let patch_tokens = self
            .patch_embeddings
            .forward(&pixel_values.to_kind(self.patch_embeddings.projection.ws.kind())?)?;
        let cls_tokens = self.cls_token.expand([batch_size, -1, -1], false)?;
        Tensor::cat(&[cls_tokens, patch_tokens], 1)?
            + self
                .interpolated_position_embeddings
                .as_ref()
                .expect("DINOv2 positions are cached after loading")
    }
}

#[derive(Debug)]
struct Dinov2PatchEmbeddings {
    projection: nn::Conv2D,
}

impl Dinov2PatchEmbeddings {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            projection: nn::conv2d(
                path / "projection",
                config.num_channels,
                config.hidden_size,
                config.patch_size,
                nn::ConvConfig {
                    stride: config.patch_size,
                    bias: true,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> koharu_torch::Result<Tensor> {
        self.projection
            .forward(pixel_values)?
            .flatten(2, -1)?
            .transpose(1, 2)
    }
}

#[derive(Debug)]
struct Dinov2Encoder {
    layer: Vec<Dinov2Layer>,
}

impl Dinov2Encoder {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            layer: (0..config.num_hidden_layers)
                .map(|index| Dinov2Layer::new(&(path / "layer" / index), config))
                .collect::<Result<_>>()?,
        })
    }

    fn forward(&self, mut hidden_states: Tensor) -> koharu_torch::Result<Tensor> {
        for layer in &self.layer {
            hidden_states = layer.forward(&hidden_states)?;
        }
        Ok(hidden_states)
    }
}

#[derive(Debug)]
struct Dinov2Layer {
    norm1: nn::LayerNorm,
    attention: Dinov2Attention,
    layer_scale1: Dinov2LayerScale,
    norm2: nn::LayerNorm,
    mlp: Dinov2Mlp,
    layer_scale2: Dinov2LayerScale,
}

impl Dinov2Layer {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            norm1: layer_norm(&(path / "norm1"), config.hidden_size, config.layer_norm_eps)?,
            attention: Dinov2Attention::new(&(path / "attention"), config)?,
            layer_scale1: Dinov2LayerScale::new(&(path / "layer_scale1"), config.hidden_size)?,
            norm2: layer_norm(&(path / "norm2"), config.hidden_size, config.layer_norm_eps)?,
            mlp: Dinov2Mlp::new(&(path / "mlp"), config)?,
            layer_scale2: Dinov2LayerScale::new(&(path / "layer_scale2"), config.hidden_size)?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = (hidden_states
            + self.layer_scale1.forward(
                &self
                    .attention
                    .forward(&self.norm1.forward(hidden_states)?)?,
            )?)?;
        &hidden_states
            + self
                .layer_scale2
                .forward(&self.mlp.forward(&self.norm2.forward(&hidden_states)?)?)?
    }
}

#[derive(Debug)]
struct Dinov2Attention {
    attention: Dinov2SelfAttention,
    output: Dinov2SelfOutput,
}

impl Dinov2Attention {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            attention: Dinov2SelfAttention::new(&(path / "attention"), config)?,
            output: Dinov2SelfOutput::new(&(path / "output"), config)?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.output.forward(&self.attention.forward(hidden_states)?)
    }
}

#[derive(Debug)]
struct Dinov2SelfAttention {
    query: nn::Linear,
    key: nn::Linear,
    value: nn::Linear,
    num_heads: i64,
    head_dim: i64,
}

impl Dinov2SelfAttention {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        let linear_config = nn::LinearConfig {
            bias: config.qkv_bias,
        };
        Ok(Self {
            query: nn::linear(
                path / "query",
                config.hidden_size,
                config.hidden_size,
                linear_config,
            )?,
            key: nn::linear(
                path / "key",
                config.hidden_size,
                config.hidden_size,
                linear_config,
            )?,
            value: nn::linear(
                path / "value",
                config.hidden_size,
                config.hidden_size,
                linear_config,
            )?,
            num_heads: config.num_attention_heads,
            head_dim: config.hidden_size / config.num_attention_heads,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let size = hidden_states.size();
        let batch_size = size[0];
        let sequence_length = size[1];
        let shape = [batch_size, sequence_length, self.num_heads, self.head_dim];
        let query = self
            .query
            .forward(hidden_states)?
            .view(shape)?
            .transpose(1, 2)?;
        let key = self
            .key
            .forward(hidden_states)?
            .view(shape)?
            .transpose(1, 2)?;
        let value = self
            .value
            .forward(hidden_states)?
            .view(shape)?
            .transpose(1, 2)?;
        Tensor::scaled_dot_product_attention(
            &query,
            &key,
            &value,
            None::<&Tensor>,
            0.0,
            false,
            (self.head_dim as f64).powf(-0.5),
            false,
        )?
        .transpose(1, 2)?
        .contiguous()?
        .reshape([batch_size, sequence_length, self.num_heads * self.head_dim])
    }
}

#[derive(Debug)]
struct Dinov2SelfOutput {
    dense: nn::Linear,
}

impl Dinov2SelfOutput {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            dense: nn::linear(
                path / "dense",
                config.hidden_size,
                config.hidden_size,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.dense.forward(hidden_states)
    }
}

#[derive(Debug)]
struct Dinov2LayerScale {
    lambda1: Tensor,
}

impl Dinov2LayerScale {
    fn new(path: &nn::Path<'_>, hidden_size: i64) -> Result<Self> {
        Ok(Self {
            lambda1: path.var("lambda1", &[hidden_size])?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        hidden_states * &self.lambda1
    }
}

#[derive(Debug)]
struct Dinov2Mlp {
    fc1: nn::Linear,
    fc2: nn::Linear,
}

impl Dinov2Mlp {
    fn new(path: &nn::Path<'_>, config: &Dinov2Config) -> Result<Self> {
        Ok(Self {
            fc1: nn::linear(
                path / "fc1",
                config.hidden_size,
                config.hidden_size * config.mlp_ratio,
                Default::default(),
            )?,
            fc2: nn::linear(
                path / "fc2",
                config.hidden_size * config.mlp_ratio,
                config.hidden_size,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.fc2
            .forward(&self.fc1.forward(hidden_states)?.gelu("none")?)
    }
}

#[derive(Debug)]
struct BaberuVisionProjector {
    linear1: nn::Linear,
    linear2: nn::Linear,
}

impl BaberuVisionProjector {
    fn new(path: &nn::Path<'_>, config: &BaberuOcrConfig) -> Result<Self> {
        Ok(Self {
            linear1: nn::linear(
                path / "linear1",
                config.vision_hidden_size,
                config.hidden_size,
                Default::default(),
            )?,
            linear2: nn::linear(
                path / "linear2",
                config.hidden_size,
                config.hidden_size,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.linear2
            .forward(&self.linear1.forward(hidden_states)?.gelu("none")?)
    }
}

#[derive(Debug)]
struct BaberuModel {
    embed_tokens: nn::Embedding,
    layers: Vec<BaberuDecoderLayer>,
    norm: BaberuRmsNorm,
    rotary_emb: BaberuRotaryEmbedding,
    attn_logit_softcap: Option<f64>,
}

impl BaberuModel {
    fn new(
        path: &nn::Path<'_>,
        config: &BaberuOcrConfig,
        device: Device,
        kind: Kind,
    ) -> Result<Self> {
        Ok(Self {
            embed_tokens: nn::embedding(
                path / "embed_tokens",
                config.vocab_size,
                config.hidden_size,
                nn::EmbeddingConfig {
                    padding_idx: config.pad_token_id,
                    ..Default::default()
                },
            )?,
            layers: (0..config.num_hidden_layers)
                .map(|index| BaberuDecoderLayer::new(&(path / "layers" / index), config))
                .collect::<Result<_>>()?,
            norm: BaberuRmsNorm::new(&(path / "norm"), config.hidden_size, config.rms_norm_eps)?,
            rotary_emb: BaberuRotaryEmbedding::new(
                config.head_dim(),
                config.max_position_embeddings,
                config.rope_theta,
                device,
                kind,
            )?,
            attn_logit_softcap: config.attn_logit_softcap,
        })
    }

    fn forward(
        &self,
        mut hidden_states: Tensor,
        past_key_values: Option<&[LayerCache]>,
    ) -> koharu_torch::Result<(Tensor, Vec<LayerCache>)> {
        let q_len = hidden_states.size()[1];
        let past_len = past_key_values
            .and_then(|cache| cache.first())
            .map_or(0, |cache| cache.key.size()[2]);
        let (cos, sin) = self.rotary_emb.forward(past_len, q_len)?;
        let attention_mask = (self.attn_logit_softcap.is_some() || (past_len != 0 && q_len != 1))
            .then(|| {
                causal_mask(
                    q_len,
                    past_len + q_len,
                    hidden_states.kind(),
                    hidden_states.device(),
                )
            })
            .transpose()?;
        let mut next_cache = Vec::with_capacity(self.layers.len());
        for (index, layer) in self.layers.iter().enumerate() {
            let (next_hidden_states, present) = layer.forward(
                &hidden_states,
                &cos,
                &sin,
                attention_mask.as_ref(),
                past_key_values.map(|cache| &cache[index]),
                past_len,
            )?;
            hidden_states = next_hidden_states;
            next_cache.push(present);
        }
        Ok((self.norm.forward(&hidden_states)?, next_cache))
    }
}

#[derive(Debug)]
struct BaberuDecoderLayer {
    input_norm: BaberuRmsNorm,
    attn: BaberuAttention,
    post_attn_norm: BaberuRmsNorm,
    pre_ffn_norm: BaberuRmsNorm,
    mlp: BaberuMlp,
    post_ffn_norm: BaberuRmsNorm,
    sandwich: bool,
}

impl BaberuDecoderLayer {
    fn new(path: &nn::Path<'_>, config: &BaberuOcrConfig) -> Result<Self> {
        Ok(Self {
            input_norm: BaberuRmsNorm::new(
                &(path / "input_norm"),
                config.hidden_size,
                config.rms_norm_eps,
            )?,
            attn: BaberuAttention::new(&(path / "attn"), config)?,
            post_attn_norm: BaberuRmsNorm::new(
                &(path / "post_attn_norm"),
                config.hidden_size,
                config.rms_norm_eps,
            )?,
            pre_ffn_norm: BaberuRmsNorm::new(
                &(path / "pre_ffn_norm"),
                config.hidden_size,
                config.rms_norm_eps,
            )?,
            mlp: BaberuMlp::new(&(path / "mlp"), config)?,
            post_ffn_norm: BaberuRmsNorm::new(
                &(path / "post_ffn_norm"),
                config.hidden_size,
                config.rms_norm_eps,
            )?,
            sandwich: config.sandwich_norm,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        cos: &Tensor,
        sin: &Tensor,
        attention_mask: Option<&Tensor>,
        past_key_value: Option<&LayerCache>,
        past_len: i64,
    ) -> koharu_torch::Result<(Tensor, LayerCache)> {
        let (attention, present) = self.attn.forward(
            &self.input_norm.forward(hidden_states)?,
            cos,
            sin,
            attention_mask,
            past_key_value,
            past_len,
        )?;
        let attention = if self.sandwich {
            self.post_attn_norm.forward(&attention)?
        } else {
            attention
        };
        let hidden_states = (hidden_states + attention)?;
        let mlp = self
            .mlp
            .forward(&self.pre_ffn_norm.forward(&hidden_states)?)?;
        let mlp = if self.sandwich {
            self.post_ffn_norm.forward(&mlp)?
        } else {
            mlp
        };
        Ok(((hidden_states + mlp)?, present))
    }
}

#[derive(Debug)]
struct BaberuAttention {
    q_proj: nn::Linear,
    k_proj: nn::Linear,
    v_proj: nn::Linear,
    o_proj: nn::Linear,
    num_heads: i64,
    num_kv_heads: i64,
    head_dim: i64,
    attn_softcap: Option<f64>,
}

impl BaberuAttention {
    fn new(path: &nn::Path<'_>, config: &BaberuOcrConfig) -> Result<Self> {
        let no_bias = nn::LinearConfig { bias: false };
        Ok(Self {
            q_proj: nn::linear(
                path / "q_proj",
                config.hidden_size,
                config.num_attention_heads * config.head_dim(),
                no_bias,
            )?,
            k_proj: nn::linear(
                path / "k_proj",
                config.hidden_size,
                config.num_key_value_heads * config.head_dim(),
                no_bias,
            )?,
            v_proj: nn::linear(
                path / "v_proj",
                config.hidden_size,
                config.num_key_value_heads * config.head_dim(),
                no_bias,
            )?,
            o_proj: nn::linear(
                path / "o_proj",
                config.num_attention_heads * config.head_dim(),
                config.hidden_size,
                no_bias,
            )?,
            num_heads: config.num_attention_heads,
            num_kv_heads: config.num_key_value_heads,
            head_dim: config.head_dim(),
            attn_softcap: config.attn_logit_softcap,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        cos: &Tensor,
        sin: &Tensor,
        attention_mask: Option<&Tensor>,
        past_key_value: Option<&LayerCache>,
        past_len: i64,
    ) -> koharu_torch::Result<(Tensor, LayerCache)> {
        let size = hidden_states.size();
        let batch_size = size[0];
        let q_len = size[1];
        let mut query = self
            .q_proj
            .forward(hidden_states)?
            .view([batch_size, q_len, self.num_heads, self.head_dim])?
            .transpose(1, 2)?;
        let mut key = self
            .k_proj
            .forward(hidden_states)?
            .view([batch_size, q_len, self.num_kv_heads, self.head_dim])?
            .transpose(1, 2)?;
        let mut value = self
            .v_proj
            .forward(hidden_states)?
            .view([batch_size, q_len, self.num_kv_heads, self.head_dim])?
            .transpose(1, 2)?;
        (query, key) = apply_rotary_pos_emb(&query, &key, cos, sin)?;
        if let Some(cache) = past_key_value {
            key = Tensor::cat(&[cache.key.alias()?, key], 2)?;
            value = Tensor::cat(&[cache.value.alias()?, value], 2)?;
        }
        let present = LayerCache {
            key: key.alias()?,
            value: value.alias()?,
        };

        let attention = if let Some(cap) = self.attn_softcap {
            let repeats = self.num_heads / self.num_kv_heads;
            let expanded_key = key.repeat_interleave_self_int(repeats, 1, None::<i64>)?;
            let expanded_value = value.repeat_interleave_self_int(repeats, 1, None::<i64>)?;
            let mut scores =
                (query.matmul(&expanded_key.transpose(-1, -2)?)? / (self.head_dim as f64).sqrt())?;
            scores = ((scores / cap)?.tanh()? * cap)?;
            if let Some(mask) = attention_mask {
                scores = (scores + mask)?;
            }
            scores
                .softmax(-1, Some(Kind::Float))?
                .to_kind(query.kind())?
                .matmul(&expanded_value)?
        } else {
            // Native GQA avoids physically repeating K/V heads and lets CUDA
            // select flash or memory-efficient SDPA kernels.
            Tensor::scaled_dot_product_attention(
                &query,
                &key,
                &value,
                attention_mask,
                0.0,
                attention_mask.is_none() && past_len == 0 && q_len > 1,
                (self.head_dim as f64).powf(-0.5),
                true,
            )?
        };
        let attention = attention.transpose(1, 2)?.contiguous()?.view([
            batch_size,
            q_len,
            self.num_heads * self.head_dim,
        ])?;
        Ok((self.o_proj.forward(&attention)?, present))
    }
}

#[derive(Debug)]
struct LayerCache {
    key: Tensor,
    value: Tensor,
}

#[derive(Debug)]
struct BaberuMlp {
    gate_proj: nn::Linear,
    up_proj: nn::Linear,
    down_proj: nn::Linear,
}

impl BaberuMlp {
    fn new(path: &nn::Path<'_>, config: &BaberuOcrConfig) -> Result<Self> {
        let no_bias = nn::LinearConfig { bias: false };
        Ok(Self {
            gate_proj: nn::linear(
                path / "gate_proj",
                config.hidden_size,
                config.intermediate_size,
                no_bias,
            )?,
            up_proj: nn::linear(
                path / "up_proj",
                config.hidden_size,
                config.intermediate_size,
                no_bias,
            )?,
            down_proj: nn::linear(
                path / "down_proj",
                config.intermediate_size,
                config.hidden_size,
                no_bias,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.down_proj.forward(
            &(self.gate_proj.forward(hidden_states)?.silu()?
                * self.up_proj.forward(hidden_states)?)?,
        )
    }
}

#[derive(Debug)]
struct BaberuRmsNorm {
    weight: Tensor,
    eps: f64,
}

impl BaberuRmsNorm {
    fn new(path: &nn::Path<'_>, hidden_size: i64, eps: f64) -> Result<Self> {
        Ok(Self {
            weight: path.var("weight", &[hidden_size])?,
            eps,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let input_kind = hidden_states.kind();
        let hidden_states = hidden_states.to_kind(Kind::Float)?;
        let variance = hidden_states.pow_tensor_scalar(2.0)?.mean_dim(
            &[-1i64][..],
            true,
            Some(Kind::Float),
        )?;
        ((&self.weight * hidden_states)? * (variance + self.eps)?.rsqrt()?)?.to_kind(input_kind)
    }
}

#[derive(Debug)]
struct BaberuRotaryEmbedding {
    cos: Tensor,
    sin: Tensor,
}

impl BaberuRotaryEmbedding {
    fn new(dim: i64, max_seq_len: i64, theta: f64, device: Device, kind: Kind) -> Result<Self> {
        let positions = Tensor::arange(max_seq_len, (Kind::Float, device))?.unsqueeze(1)?;
        let dimensions = Tensor::arange_start_step(0, dim, 2, (Kind::Float, device))?;
        let inv_freq = ((dimensions / dim as f64)? * theta.ln())?
            .exp()?
            .reciprocal()?;
        let frequencies = positions.matmul(&inv_freq.unsqueeze(0)?)?;
        let embeddings = Tensor::cat(&[frequencies.alias()?, frequencies], 1)?;
        Ok(Self {
            cos: embeddings.cos()?.to_kind(kind)?,
            sin: embeddings.sin()?.to_kind(kind)?,
        })
    }

    fn forward(
        &self,
        past_len: i64,
        sequence_length: i64,
    ) -> koharu_torch::Result<(Tensor, Tensor)> {
        Ok((
            self.cos
                .narrow(0, past_len, sequence_length)?
                .unsqueeze(0)?,
            self.sin
                .narrow(0, past_len, sequence_length)?
                .unsqueeze(0)?,
        ))
    }
}

fn apply_rotary_pos_emb(
    query: &Tensor,
    key: &Tensor,
    cos: &Tensor,
    sin: &Tensor,
) -> koharu_torch::Result<(Tensor, Tensor)> {
    let cos = cos.unsqueeze(1)?;
    let sin = sin.unsqueeze(1)?;
    Ok((
        ((query * &cos)? + (rotate_half(query)? * &sin)?)?,
        ((key * cos)? + (rotate_half(key)? * sin)?)?,
    ))
}

fn rotate_half(hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
    let half = hidden_states.size()[hidden_states.dim() - 1] / 2;
    Tensor::cat(
        &[
            (-hidden_states.narrow(-1, half, half)?)?,
            hidden_states.narrow(-1, 0, half)?,
        ],
        -1,
    )
}

fn causal_mask(
    q_len: i64,
    kv_len: i64,
    kind: Kind,
    device: Device,
) -> koharu_torch::Result<Tensor> {
    Tensor::full([q_len, kv_len], f64::NEG_INFINITY, (kind, device))?
        .triu(kv_len - q_len + 1)?
        .unsqueeze(0)?
        .unsqueeze(0)
}

fn layer_norm(path: &nn::Path<'_>, hidden_size: i64, eps: f64) -> Result<nn::LayerNorm> {
    Ok(nn::layer_norm(
        path,
        vec![hidden_size],
        nn::LayerNormConfig {
            eps,
            ..Default::default()
        },
    )?)
}
