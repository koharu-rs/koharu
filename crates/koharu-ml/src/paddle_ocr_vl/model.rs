//! Inference-only Torch port of Transformers' PaddleOCR-VL model.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/paddleocr_vl/modeling_paddleocr_vl.py

use std::{collections::HashSet, path::Path};

use anyhow::{Result, bail, ensure};
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module},
};

use super::{
    REPETITION_PENALTY,
    config::{PaddleOCRVLConfig, PaddleOCRVisionConfig},
};

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    visual: PaddleOCRVisionModel,
    projector: PaddleOCRProjector,
    language_model: PaddleOCRTextModel,
    lm_head: nn::Linear,
    config: PaddleOCRVLConfig,
}

impl Model {
    pub(super) fn new(config: PaddleOCRVLConfig, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();

        // Transformers renames the checkpoint's `visual`, `model`, and `mlp_AR`
        // prefixes while loading. Keeping those original paths here lets VarStore
        // load the published checkpoint directly without a parallel weight reader.
        let visual =
            PaddleOCRVisionModel::new(&(&root / "visual" / "vision_model"), &config.vision_config)?;
        let projector = PaddleOCRProjector::new(&(&root / "mlp_AR"), &config)?;
        let language_model = PaddleOCRTextModel::new(&(&root / "model"), &config)?;
        let lm_head = linear(
            &(&root / "lm_head"),
            config.hidden_size,
            config.vocab_size,
            false,
        )?;
        Ok(Self {
            vs,
            visual,
            projector,
            language_model,
            lm_head,
            config,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        // The checkpoint carries these, and upstream leaves them out:
        // https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/paddleocr_vl/modeling_paddleocr_vl.py#L1309
        self.vs
            .load_ignoring(path, &["packing_position_embedding", "vision_model.head"])?;
        Ok(())
    }

    pub(super) fn forward(
        &self,
        input_ids: &[i64],
        mm_token_type_ids: &[i64],
        pixel_values: &Tensor,
        image_grid_thw: [i64; 3],
        max_new_tokens: usize,
    ) -> Result<Vec<i64>> {
        ensure!(
            input_ids.len() == mm_token_type_ids.len(),
            "input_ids and mm_token_type_ids must have equal lengths"
        );
        let device = self.vs.device();
        let ids = Tensor::from_slice(input_ids)?
            .view([1, input_ids.len() as i64])?
            .to_device(device)?;
        let mut inputs_embeds = self.language_model.embed_tokens(&ids)?;

        let image_features =
            self.get_image_features(&pixel_values.to_kind(self.vs.kind())?, image_grid_thw)?;
        let image_mask = ids
            .eq(self.config.image_token_id)?
            .unsqueeze(-1)?
            .expand_as(&inputs_embeds)?;
        let expected = input_ids
            .iter()
            .filter(|&&id| id == self.config.image_token_id)
            .count() as i64;
        ensure!(
            image_features.size()[0] == expected,
            "image features and image tokens do not match: tokens={expected}, features={}",
            image_features.size()[0]
        );
        inputs_embeds = inputs_embeds.masked_scatter(&image_mask, &image_features)?;

        let (position_ids, rope_delta) =
            self.get_rope_index(input_ids, mm_token_type_ids, image_grid_thw, device)?;
        if self.config.use_cache {
            Ok(self.generate_cached(
                inputs_embeds,
                &position_ids,
                rope_delta,
                input_ids.len() as i64,
                max_new_tokens,
            )?)
        } else {
            Ok(self.generate_uncached(
                inputs_embeds,
                position_ids,
                rope_delta,
                input_ids.len() as i64,
                max_new_tokens,
            )?)
        }
    }

    fn generate_cached(
        &self,
        inputs_embeds: Tensor,
        position_ids: &Tensor,
        rope_delta: i64,
        input_length: i64,
        max_new_tokens: usize,
    ) -> Result<Vec<i64>> {
        let device = inputs_embeds.device();
        let mut cache = (0..self.config.num_hidden_layers)
            .map(|_| None)
            .collect::<Vec<Option<(Tensor, Tensor)>>>();
        let hidden_states =
            self.language_model
                .forward(inputs_embeds, position_ids, true, &mut cache)?;
        let last_index = hidden_states.size()[1] - 1;
        let mut logits = self
            .lm_head
            .forward(&hidden_states.select(1, last_index)?)?;

        let mut generated = Vec::with_capacity(max_new_tokens);
        let mut unique_tokens = HashSet::new();
        let mut seen_ids = None;
        for step in 0..max_new_tokens {
            let next_token = greedy_token(&logits, seen_ids.as_ref())?;
            if next_token == self.config.eos_token_id {
                break;
            }
            generated.push(next_token);
            if step + 1 == max_new_tokens {
                break;
            }

            let next_ids = Tensor::from_slice(&[next_token])?
                .view([1, 1])?
                .to_device(device)?;
            if unique_tokens.insert(next_token) {
                seen_ids = Some(match seen_ids {
                    Some(seen_ids) => Tensor::cat(&[seen_ids, next_ids.alias()?], 1)?,
                    None => next_ids.alias()?,
                });
            }
            let position = input_length + step as i64 + rope_delta;
            let position_ids = Tensor::full([3, 1, 1], position, (Kind::Int64, device))?;
            let hidden_states = self.language_model.forward(
                self.language_model.embed_tokens(&next_ids)?,
                &position_ids,
                false,
                &mut cache,
            )?;
            logits = self.lm_head.forward(&hidden_states.select(1, 0)?)?;
        }
        Ok(generated)
    }

    fn generate_uncached(
        &self,
        mut inputs_embeds: Tensor,
        mut position_ids: Tensor,
        rope_delta: i64,
        input_length: i64,
        max_new_tokens: usize,
    ) -> Result<Vec<i64>> {
        let device = inputs_embeds.device();
        let mut generated = Vec::with_capacity(max_new_tokens);
        let mut unique_tokens = HashSet::new();
        let mut seen_ids = None;
        for step in 0..max_new_tokens {
            let mut cache = (0..self.config.num_hidden_layers)
                .map(|_| None)
                .collect::<Vec<Option<(Tensor, Tensor)>>>();
            let hidden_states = self.language_model.forward(
                inputs_embeds.alias()?,
                &position_ids,
                true,
                &mut cache,
            )?;
            let last_index = hidden_states.size()[1] - 1;
            let logits = self
                .lm_head
                .forward(&hidden_states.select(1, last_index)?)?;
            let next_token = greedy_token(&logits, seen_ids.as_ref())?;
            if next_token == self.config.eos_token_id {
                break;
            }
            generated.push(next_token);
            if step + 1 == max_new_tokens {
                break;
            }

            let next_ids = Tensor::from_slice(&[next_token])?
                .view([1, 1])?
                .to_device(device)?;
            if unique_tokens.insert(next_token) {
                seen_ids = Some(match seen_ids {
                    Some(seen_ids) => Tensor::cat(&[seen_ids, next_ids.alias()?], 1)?,
                    None => next_ids.alias()?,
                });
            }
            inputs_embeds = Tensor::cat(
                &[inputs_embeds, self.language_model.embed_tokens(&next_ids)?],
                1,
            )?;
            let position = input_length + step as i64 + rope_delta;
            position_ids = Tensor::cat(
                &[
                    position_ids,
                    Tensor::full([3, 1, 1], position, (Kind::Int64, device))?,
                ],
                2,
            )?;
        }
        Ok(generated)
    }

    fn get_image_features(&self, pixel_values: &Tensor, grid_thw: [i64; 3]) -> Result<Tensor> {
        let hidden_states = self.visual.forward(pixel_values, grid_thw)?;
        self.projector.forward(&hidden_states, grid_thw)
    }

    fn get_rope_index(
        &self,
        input_ids: &[i64],
        mm_token_type_ids: &[i64],
        grid_thw: [i64; 3],
        device: Device,
    ) -> Result<(Tensor, i64)> {
        let mut positions = [Vec::new(), Vec::new(), Vec::new()];
        let mut current_position = 0i64;
        let mut offset = 0usize;
        let mut consumed_image = false;

        while offset < input_ids.len() {
            let modality = mm_token_type_ids[offset];
            let end = mm_token_type_ids[offset..]
                .iter()
                .position(|&value| value != modality)
                .map_or(input_ids.len(), |length| offset + length);
            if modality == 0 {
                for position in current_position..current_position + (end - offset) as i64 {
                    for axis in &mut positions {
                        axis.push(position);
                    }
                }
                current_position += (end - offset) as i64;
            } else if modality == 1 {
                ensure!(
                    !consumed_image,
                    "only one image is supported per inference call"
                );
                consumed_image = true;
                let height = grid_thw[1] / self.config.vision_config.spatial_merge_size;
                let width = grid_thw[2] / self.config.vision_config.spatial_merge_size;
                ensure!(
                    (end - offset) as i64 == grid_thw[0] * height * width,
                    "image token run does not match image grid"
                );
                for temporal in 0..grid_thw[0] {
                    for row in 0..height {
                        for column in 0..width {
                            positions[0].push(current_position + temporal);
                            positions[1].push(current_position + row);
                            positions[2].push(current_position + column);
                        }
                    }
                }
                current_position += height.max(width);
            } else {
                bail!("unsupported PaddleOCR-VL modality type {modality}");
            }
            offset = end;
        }

        ensure!(
            consumed_image,
            "PaddleOCR-VL prompt contains no image tokens"
        );
        let maximum = positions
            .iter()
            .flat_map(|axis| axis.iter())
            .copied()
            .max()
            .unwrap_or(0);
        let rope_delta = maximum + 1 - input_ids.len() as i64;
        let position_ids = Tensor::stack(
            &positions
                .iter()
                .map(|axis| Tensor::from_slice(axis)?.to_device(device))
                .collect::<koharu_torch::Result<Vec<_>>>()?,
            0,
        )?
        .unsqueeze(1)?;
        Ok((position_ids, rope_delta))
    }
}

fn greedy_token(logits: &Tensor, seen_ids: Option<&Tensor>) -> Result<i64> {
    let logits = seen_ids.map_or_else(
        || logits.alias(),
        |seen_ids| crate::repetition_penalty(logits, seen_ids, f64::from(REPETITION_PENALTY)),
    )?;
    Ok(logits.argmax(-1, false)?.int64_value(&[0])?)
}

#[derive(Debug)]
struct PaddleOCRProjector {
    pre_norm: nn::LayerNorm,
    linear_1: nn::Linear,
    linear_2: nn::Linear,
    spatial_merge_size: i64,
}

impl PaddleOCRProjector {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVLConfig) -> Result<Self> {
        let vision_hidden = config.vision_config.hidden_size;
        let merge = config.vision_config.spatial_merge_size;
        let merged_hidden = vision_hidden * merge * merge;
        Ok(Self {
            pre_norm: nn::layer_norm(
                path / "pre_norm",
                vec![vision_hidden],
                nn::LayerNormConfig {
                    eps: 1e-5,
                    ..Default::default()
                },
            )?,
            linear_1: linear(&(path / "linear_1"), merged_hidden, merged_hidden, true)?,
            linear_2: linear(
                &(path / "linear_2"),
                merged_hidden,
                config.hidden_size,
                true,
            )?,
            spatial_merge_size: merge,
        })
    }

    fn forward(&self, image_features: &Tensor, grid_thw: [i64; 3]) -> Result<Tensor> {
        let [temporal, height, width] = grid_thw;
        let merge = self.spatial_merge_size;
        let hidden = image_features.size()[1];
        let image_features = self.pre_norm.forward(image_features)?;
        let image_features = image_features
            .reshape([
                temporal,
                height / merge,
                merge,
                width / merge,
                merge,
                hidden,
            ])?
            .transpose(2, 3)?
            .reshape([
                temporal * (height / merge) * (width / merge),
                merge * merge * hidden,
            ])?;
        Ok(self
            .linear_2
            .forward(&self.linear_1.forward(&image_features)?.gelu("none")?)?)
    }
}

#[derive(Debug)]
struct PaddleOCRVisionModel {
    embeddings: PaddleOCRVisionEmbeddings,
    encoder: PaddleOCRVisionEncoder,
    post_layernorm: nn::LayerNorm,
}

impl PaddleOCRVisionModel {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVisionConfig) -> Result<Self> {
        Ok(Self {
            embeddings: PaddleOCRVisionEmbeddings::new(&(path / "embeddings"), config)?,
            encoder: PaddleOCRVisionEncoder::new(&(path / "encoder"), config)?,
            post_layernorm: nn::layer_norm(
                path / "post_layernorm",
                vec![config.hidden_size],
                nn::LayerNormConfig {
                    eps: config.layer_norm_eps,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, pixel_values: &Tensor, grid_thw: [i64; 3]) -> Result<Tensor> {
        let hidden_states = self.embeddings.forward(pixel_values, grid_thw)?;
        Ok(self
            .post_layernorm
            .forward(&self.encoder.forward(hidden_states, grid_thw)?)?)
    }
}

#[derive(Debug)]
struct PaddleOCRVisionEmbeddings {
    patch_embedding: nn::Conv2D,
    position_embedding: nn::Embedding,
    num_grid_per_side: i64,
}

impl PaddleOCRVisionEmbeddings {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVisionConfig) -> Result<Self> {
        let patch_embedding = nn::conv2d(
            path / "patch_embedding",
            config.num_channels,
            config.hidden_size,
            config.patch_size,
            nn::ConvConfig {
                stride: config.patch_size,
                ..Default::default()
            },
        )?;
        let num_grid_per_side = config.image_size / config.patch_size;
        let position_embedding = nn::embedding(
            path / "position_embedding",
            num_grid_per_side * num_grid_per_side,
            config.hidden_size,
            Default::default(),
        )?;
        Ok(Self {
            patch_embedding,
            position_embedding,
            num_grid_per_side,
        })
    }

    fn forward(&self, pixel_values: &Tensor, grid_thw: [i64; 3]) -> koharu_torch::Result<Tensor> {
        let embeddings = self
            .patch_embedding
            .forward(&pixel_values.to_kind(self.patch_embedding.ws.kind())?)?
            .flatten(2, 3)?
            .squeeze_dim(-1)?
            .reshape([-1, self.patch_embedding.ws.size()[0]])?;
        let position_embeddings = self.position_embeddings(grid_thw)?;
        embeddings + position_embeddings
    }

    // https://github.com/huggingface/transformers/blob/8c84144bfc7dd0c9c5e336a6d89c9dcee2efc2a8/src/transformers/models/paddleocr_vl/modeling_paddleocr_vl.py#L593-L621
    fn position_embeddings(&self, grid: [i64; 3]) -> koharu_torch::Result<Tensor> {
        let [temporal, height, width] = grid;
        let side = self.num_grid_per_side;
        let hidden = self.position_embedding.ws.size()[1];
        self.position_embedding
            .ws
            .reshape([1, side, side, hidden])?
            .permute([0, 3, 1, 2])?
            .upsample_bilinear2d([height, width], false, None::<f64>, None::<f64>)?
            .permute([0, 2, 3, 1])?
            .reshape([height * width, hidden])?
            .repeat([temporal, 1])
    }
}

#[derive(Debug)]
struct PaddleOCRVisionEncoder {
    layers: Vec<PaddleOCRVisionEncoderLayer>,
    head_dim: i64,
}

impl PaddleOCRVisionEncoder {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVisionConfig) -> Result<Self> {
        Ok(Self {
            layers: (0..config.num_hidden_layers)
                .map(|index| PaddleOCRVisionEncoderLayer::new(&(path / "layers" / index), config))
                .collect::<Result<Vec<_>>>()?,
            head_dim: config.hidden_size / config.num_attention_heads,
        })
    }

    fn forward(&self, mut hidden_states: Tensor, grid_thw: [i64; 3]) -> Result<Tensor> {
        let position_ids = vision_position_ids(grid_thw, hidden_states.device())?;
        let inv_freq = rotary_inv_freq(self.head_dim / 2, 10_000.0, hidden_states.device())?;
        let rotary = (position_ids.to_kind(Kind::Float)?.unsqueeze(-1)? * inv_freq)?
            .flatten(1, 2)?
            .repeat([1, 2])?;
        let cos = rotary.cos()?;
        let sin = rotary.sin()?;
        for layer in &self.layers {
            hidden_states = layer.forward(hidden_states, &cos, &sin)?;
        }
        Ok(hidden_states)
    }
}

#[derive(Debug)]
struct PaddleOCRVisionEncoderLayer {
    layer_norm1: nn::LayerNorm,
    self_attn: PaddleOCRVisionAttention,
    layer_norm2: nn::LayerNorm,
    mlp: PaddleOCRVisionMLP,
}

impl PaddleOCRVisionEncoderLayer {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVisionConfig) -> Result<Self> {
        let norm_config = nn::LayerNormConfig {
            eps: config.layer_norm_eps,
            ..Default::default()
        };
        Ok(Self {
            layer_norm1: nn::layer_norm(
                path / "layer_norm1",
                vec![config.hidden_size],
                norm_config,
            )?,
            self_attn: PaddleOCRVisionAttention::new(&(path / "self_attn"), config)?,
            layer_norm2: nn::layer_norm(
                path / "layer_norm2",
                vec![config.hidden_size],
                norm_config,
            )?,
            mlp: PaddleOCRVisionMLP::new(&(path / "mlp"), config)?,
        })
    }

    fn forward(&self, hidden_states: Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
        let residual = hidden_states.alias()?;
        let hidden_states = (residual
            + self
                .self_attn
                .forward(&self.layer_norm1.forward(&hidden_states)?, cos, sin)?)?;
        let residual = hidden_states.alias()?;
        Ok((residual
            + self
                .mlp
                .forward(&self.layer_norm2.forward(&hidden_states)?)?)?)
    }
}

#[derive(Debug)]
struct PaddleOCRVisionAttention {
    q_proj: nn::Linear,
    k_proj: nn::Linear,
    v_proj: nn::Linear,
    out_proj: nn::Linear,
    num_heads: i64,
    head_dim: i64,
}

impl PaddleOCRVisionAttention {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVisionConfig) -> Result<Self> {
        let hidden = config.hidden_size;
        Ok(Self {
            q_proj: linear(&(path / "q_proj"), hidden, hidden, true)?,
            k_proj: linear(&(path / "k_proj"), hidden, hidden, true)?,
            v_proj: linear(&(path / "v_proj"), hidden, hidden, true)?,
            out_proj: linear(&(path / "out_proj"), hidden, hidden, true)?,
            num_heads: config.num_attention_heads,
            head_dim: hidden / config.num_attention_heads,
        })
    }

    fn forward(&self, hidden_states: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
        let sequence_length = hidden_states.size()[0];
        let query = self.q_proj.forward(hidden_states)?.view([
            sequence_length,
            self.num_heads,
            self.head_dim,
        ])?;
        let key = self.k_proj.forward(hidden_states)?.view([
            sequence_length,
            self.num_heads,
            self.head_dim,
        ])?;
        let value = self.v_proj.forward(hidden_states)?.view([
            sequence_length,
            self.num_heads,
            self.head_dim,
        ])?;
        let (query, key) = apply_vision_rotary(query, key, cos, sin)?;
        let query = query.transpose(0, 1)?.unsqueeze(0)?;
        let key = key.transpose(0, 1)?.unsqueeze(0)?;
        let value = value.transpose(0, 1)?.unsqueeze(0)?;
        let attention = (query.matmul(&key.transpose(-2, -1)?)?
            * (self.head_dim as f64).powf(-0.5))?
        .softmax(-1, Kind::Float)?
        .to_kind(query.kind())?;
        let output = attention
            .matmul(&value)?
            .transpose(1, 2)?
            .reshape([sequence_length, self.num_heads * self.head_dim])?;
        Ok(self.out_proj.forward(&output)?)
    }
}

#[derive(Debug)]
struct PaddleOCRVisionMLP {
    fc1: nn::Linear,
    fc2: nn::Linear,
}

impl PaddleOCRVisionMLP {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVisionConfig) -> Result<Self> {
        Ok(Self {
            fc1: linear(
                &(path / "fc1"),
                config.hidden_size,
                config.intermediate_size,
                true,
            )?,
            fc2: linear(
                &(path / "fc2"),
                config.intermediate_size,
                config.hidden_size,
                true,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.fc2
            .forward(&self.fc1.forward(hidden_states)?.gelu("tanh")?)
    }
}

#[derive(Debug)]
struct PaddleOCRTextModel {
    embed_tokens: nn::Embedding,
    layers: Vec<PaddleOCRDecoderLayer>,
    norm: PaddleOCRRMSNorm,
    rope_theta: f64,
    head_dim: i64,
}

impl PaddleOCRTextModel {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVLConfig) -> Result<Self> {
        let embed_tokens = nn::embedding(
            path / "embed_tokens",
            config.vocab_size,
            config.hidden_size,
            nn::EmbeddingConfig {
                padding_idx: config.pad_token_id,
                ..Default::default()
            },
        )?;
        let layers = (0..config.num_hidden_layers)
            .map(|index| PaddleOCRDecoderLayer::new(&(path / "layers" / index), config))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            embed_tokens,
            layers,
            norm: PaddleOCRRMSNorm::new(&(path / "norm"), config.hidden_size, config.rms_norm_eps)?,
            rope_theta: config.rope_theta,
            head_dim: config.head_dim,
        })
    }

    fn embed_tokens(&self, input_ids: &Tensor) -> Result<Tensor> {
        Ok(self.embed_tokens.forward(input_ids)?)
    }

    fn forward(
        &self,
        mut hidden_states: Tensor,
        position_ids: &Tensor,
        causal: bool,
        cache: &mut [Option<(Tensor, Tensor)>],
    ) -> Result<Tensor> {
        let inv_freq = rotary_inv_freq(self.head_dim, self.rope_theta, hidden_states.device())?;
        let frequencies = (position_ids.to_kind(Kind::Float)?.unsqueeze(-1)? * inv_freq)?;
        let embeddings = Tensor::cat(&[frequencies.alias()?, frequencies], -1)?;
        let cos = embeddings.cos()?.to_kind(hidden_states.kind())?;
        let sin = embeddings.sin()?.to_kind(hidden_states.kind())?;
        let attention_mask = causal
            .then(|| {
                causal_mask(
                    hidden_states.size()[1],
                    hidden_states.device(),
                    hidden_states.kind(),
                )
            })
            .transpose()?;
        for (layer, layer_cache) in self.layers.iter().zip(cache.iter_mut()) {
            hidden_states = layer.forward(
                hidden_states,
                attention_mask.as_ref(),
                &cos,
                &sin,
                layer_cache,
            )?;
        }
        Ok(self.norm.forward(&hidden_states)?)
    }
}

#[derive(Debug)]
struct PaddleOCRDecoderLayer {
    self_attn: PaddleOCRAttention,
    mlp: PaddleOCRMLP,
    input_layernorm: PaddleOCRRMSNorm,
    post_attention_layernorm: PaddleOCRRMSNorm,
}

impl PaddleOCRDecoderLayer {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVLConfig) -> Result<Self> {
        Ok(Self {
            self_attn: PaddleOCRAttention::new(&(path / "self_attn"), config)?,
            mlp: PaddleOCRMLP::new(&(path / "mlp"), config)?,
            input_layernorm: PaddleOCRRMSNorm::new(
                &(path / "input_layernorm"),
                config.hidden_size,
                config.rms_norm_eps,
            )?,
            post_attention_layernorm: PaddleOCRRMSNorm::new(
                &(path / "post_attention_layernorm"),
                config.hidden_size,
                config.rms_norm_eps,
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: Tensor,
        attention_mask: Option<&Tensor>,
        cos: &Tensor,
        sin: &Tensor,
        cache: &mut Option<(Tensor, Tensor)>,
    ) -> Result<Tensor> {
        let residual = hidden_states.alias()?;
        let hidden_states = (residual
            + self.self_attn.forward(
                &self.input_layernorm.forward(&hidden_states)?,
                attention_mask,
                cos,
                sin,
                cache,
            )?)?;
        let residual = hidden_states.alias()?;
        Ok((residual
            + self
                .mlp
                .forward(&self.post_attention_layernorm.forward(&hidden_states)?)?)?)
    }
}

#[derive(Debug)]
struct PaddleOCRAttention {
    q_proj: nn::Linear,
    k_proj: nn::Linear,
    v_proj: nn::Linear,
    o_proj: nn::Linear,
    num_heads: i64,
    num_key_value_heads: i64,
    num_key_value_groups: i64,
    head_dim: i64,
    mrope_section: Vec<i64>,
}

impl PaddleOCRAttention {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVLConfig) -> Result<Self> {
        Ok(Self {
            q_proj: linear(
                &(path / "q_proj"),
                config.hidden_size,
                config.num_attention_heads * config.head_dim,
                config.use_bias,
            )?,
            k_proj: linear(
                &(path / "k_proj"),
                config.hidden_size,
                config.num_key_value_heads * config.head_dim,
                config.use_bias,
            )?,
            v_proj: linear(
                &(path / "v_proj"),
                config.hidden_size,
                config.num_key_value_heads * config.head_dim,
                config.use_bias,
            )?,
            o_proj: linear(
                &(path / "o_proj"),
                config.num_attention_heads * config.head_dim,
                config.hidden_size,
                config.use_bias,
            )?,
            num_heads: config.num_attention_heads,
            num_key_value_heads: config.num_key_value_heads,
            num_key_value_groups: config.num_attention_heads / config.num_key_value_heads,
            head_dim: config.head_dim,
            mrope_section: config.rope_scaling.mrope_section.clone(),
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
        cos: &Tensor,
        sin: &Tensor,
        cache: &mut Option<(Tensor, Tensor)>,
    ) -> Result<Tensor> {
        let [batch, sequence_length, _] = <[i64; 3]>::try_from(hidden_states.size()).unwrap();
        let query = self
            .q_proj
            .forward(hidden_states)?
            .view([batch, sequence_length, self.num_heads, self.head_dim])?
            .transpose(1, 2)?;
        let key = self
            .k_proj
            .forward(hidden_states)?
            .view([
                batch,
                sequence_length,
                self.num_key_value_heads,
                self.head_dim,
            ])?
            .transpose(1, 2)?;
        let value = self
            .v_proj
            .forward(hidden_states)?
            .view([
                batch,
                sequence_length,
                self.num_key_value_heads,
                self.head_dim,
            ])?
            .transpose(1, 2)?;
        let (query, mut key) = apply_multimodal_rotary(query, key, cos, sin, &self.mrope_section)?;
        let mut value = value;
        if let Some((cached_key, cached_value)) = cache.as_ref() {
            key = Tensor::cat(&[cached_key.alias()?, key], 2)?;
            value = Tensor::cat(&[cached_value.alias()?, value], 2)?;
        }
        *cache = Some((key.alias()?, value.alias()?));

        let key = repeat_kv(&key, self.num_key_value_groups)?;
        let value = repeat_kv(&value, self.num_key_value_groups)?;
        let mut weights =
            (query.matmul(&key.transpose(2, 3)?)? * (self.head_dim as f64).powf(-0.5))?;
        if let Some(mask) = attention_mask {
            weights = (weights + mask)?;
        }
        weights = weights.softmax(-1, Kind::Float)?.to_kind(query.kind())?;
        let output = weights.matmul(&value)?.transpose(1, 2)?.reshape([
            batch,
            sequence_length,
            self.num_heads * self.head_dim,
        ])?;
        Ok(self.o_proj.forward(&output)?)
    }
}

#[derive(Debug)]
struct PaddleOCRMLP {
    gate_proj: nn::Linear,
    up_proj: nn::Linear,
    down_proj: nn::Linear,
}

impl PaddleOCRMLP {
    fn new(path: &nn::Path<'_>, config: &PaddleOCRVLConfig) -> Result<Self> {
        Ok(Self {
            gate_proj: linear(
                &(path / "gate_proj"),
                config.hidden_size,
                config.intermediate_size,
                config.use_bias,
            )?,
            up_proj: linear(
                &(path / "up_proj"),
                config.hidden_size,
                config.intermediate_size,
                config.use_bias,
            )?,
            down_proj: linear(
                &(path / "down_proj"),
                config.intermediate_size,
                config.hidden_size,
                config.use_bias,
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
struct PaddleOCRRMSNorm {
    weight: Tensor,
    eps: f64,
}

impl PaddleOCRRMSNorm {
    fn new(path: &nn::Path<'_>, hidden_size: i64, eps: f64) -> Result<Self> {
        Ok(Self {
            weight: path.var("weight", &[hidden_size])?,
            eps,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let input_kind = hidden_states.kind();
        let hidden_states = hidden_states.to_kind(Kind::Float)?;
        let variance =
            hidden_states
                .pow_tensor_scalar(2.0)?
                .mean_dim(&[-1i64][..], true, None::<Kind>)?;
        &self.weight * (hidden_states * (variance + self.eps)?.rsqrt()?)?.to_kind(input_kind)?
    }
}

fn linear(path: &nn::Path<'_>, input: i64, output: i64, bias: bool) -> Result<nn::Linear> {
    Ok(nn::linear(path, input, output, nn::LinearConfig { bias })?)
}

fn rotary_inv_freq(dim: i64, theta: f64, device: Device) -> koharu_torch::Result<Tensor> {
    let exponents = (Tensor::arange_start_step(0, dim, 2, (Kind::Float, device))? / dim as f64)?;
    (-theta.ln() * exponents)?.exp()
}

fn rotate_half(tensor: &Tensor) -> Result<Tensor> {
    let half = tensor.size().last().copied().unwrap() / 2;
    Ok(Tensor::cat(
        &[
            (-tensor.narrow(-1, half, half)?)?,
            tensor.narrow(-1, 0, half)?,
        ],
        -1,
    )?)
}

fn apply_vision_rotary(
    query: Tensor,
    key: Tensor,
    cos: &Tensor,
    sin: &Tensor,
) -> Result<(Tensor, Tensor)> {
    let query_kind = query.kind();
    let key_kind = key.kind();
    let cos = cos.unsqueeze(-2)?.to_kind(Kind::Float)?;
    let sin = sin.unsqueeze(-2)?.to_kind(Kind::Float)?;
    let query = query.to_kind(Kind::Float)?;
    let key = key.to_kind(Kind::Float)?;
    let query_rotated = ((&query * &cos)? + (rotate_half(&query)? * &sin)?)?;
    let key_rotated = ((&key * &cos)? + (rotate_half(&key)? * &sin)?)?;
    Ok((
        query_rotated.to_kind(query_kind)?,
        key_rotated.to_kind(key_kind)?,
    ))
}

fn apply_multimodal_rotary(
    query: Tensor,
    key: Tensor,
    cos: &Tensor,
    sin: &Tensor,
    sections: &[i64],
) -> Result<(Tensor, Tensor)> {
    let doubled = sections.iter().map(|value| value * 2).collect::<Vec<_>>();
    let cos_chunks = cos.split_with_sizes(&doubled, -1)?;
    let sin_chunks = sin.split_with_sizes(&doubled, -1)?;
    let selected_cos = Tensor::cat(
        &cos_chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| chunk.select(0, index as i64 % 3))
            .collect::<koharu_torch::Result<Vec<_>>>()?,
        -1,
    )?
    .unsqueeze(1)?;
    let selected_sin = Tensor::cat(
        &sin_chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| chunk.select(0, index as i64 % 3))
            .collect::<koharu_torch::Result<Vec<_>>>()?,
        -1,
    )?
    .unsqueeze(1)?;
    let rotated_query = ((&query * &selected_cos)? + (rotate_half(&query)? * &selected_sin)?)?;
    let rotated_key = ((&key * &selected_cos)? + (rotate_half(&key)? * &selected_sin)?)?;
    Ok((rotated_query, rotated_key))
}

fn repeat_kv(hidden_states: &Tensor, repeats: i64) -> Result<Tensor> {
    if repeats == 1 {
        return Ok(hidden_states.alias()?);
    }
    let size = hidden_states.size();
    Ok(hidden_states
        .unsqueeze(2)?
        .expand([size[0], size[1], repeats, size[2], size[3]], false)?
        .reshape([size[0], size[1] * repeats, size[2], size[3]])?)
}

fn causal_mask(sequence_length: i64, device: Device, kind: Kind) -> koharu_torch::Result<Tensor> {
    let mask = Tensor::ones([sequence_length, sequence_length], (Kind::Bool, device))?.triu(1)?;
    Tensor::zeros([sequence_length, sequence_length], (kind, device))?
        .masked_fill(&mask, f64::NEG_INFINITY)?
        .view([1, 1, sequence_length, sequence_length])
}

fn vision_position_ids(grid: [i64; 3], device: Device) -> Result<Tensor> {
    let [temporal, height, width] = grid;
    let mut rows = Vec::with_capacity((temporal * height * width) as usize);
    let mut columns = Vec::with_capacity(rows.capacity());
    for _ in 0..temporal {
        for row in 0..height {
            for column in 0..width {
                rows.push(row);
                columns.push(column);
            }
        }
    }
    Ok(Tensor::stack(
        &[
            Tensor::from_slice(&rows)?.to_device(device)?,
            Tensor::from_slice(&columns)?.to_device(device)?,
        ],
        1,
    )?)
}
