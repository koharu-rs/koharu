//! Inference-only Hayai OCR network.
//!
//! Hayai architecture and generation implementation:
//! https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2.5-nova/blob/2803d0dc005fa4396ab1e7c5a472d2c450a247f7/modeling_hayai.py
//! Upstream generates through `_FastEngine`, an inference copy of the
//! modules' math with fused weights; this port follows the modules and
//! `_reference_generate`, which compute the same result.
//! SigLIP2 NaFlex vision encoder instantiated by the upstream `HayaiModel`:
//! https://github.com/huggingface/transformers/blob/598d8ba8baaec7fec5a22da0e2844c7bf4ea20e1/src/transformers/models/siglip2/modeling_siglip2.py

use std::path::Path;

use crate::nn::{self, Module};
use anyhow::{Result, ensure};
use koharu_torch::{Device, Kind, Tensor};

use super::{
    config::{HayaiConfig, decoder, siglip2},
    processor::{ImageInput, Tokenizer},
};

/// Upstream `generate`'s default; it decodes greedily and ignores its
/// `num_beams` and `repetition_penalty` arguments.
const MAX_NEW_TOKENS: i64 = 128;
const RMS_NORM_EPS: f64 = 1e-6;

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    vision_encoder: Siglip2VisionModel,
    decoder: Decoder,
    text_rope_cos: Tensor,
    text_rope_sin: Tensor,
    bos_token_id: i64,
    eos_token_id: i64,
}

impl Model {
    pub(super) fn new(
        config: &HayaiConfig,
        tokenizer: &Tokenizer,
        device: Device,
        kind: Kind,
    ) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();
        let vision_encoder = Siglip2VisionModel::new(&(&root / "vision_encoder" / "vision_model"))?;
        let decoder = Decoder::new(&(&root / "decoder"), config)?;

        // Text tokens use plain 1D RoPE; positions 0..=MAX_NEW_TOKENS cover
        // every decoding step. Visual tokens receive their own 2D frequencies
        // per spatial shape, computed during generation.
        let inv_freq = inverse_frequencies(decoder::HEAD_DIM / 2, decoder::ROPE_THETA, device)?;
        let positions = Tensor::f_arange(MAX_NEW_TOKENS + 1, (Kind::Float, device))?
            .f_unsqueeze(1)?
            .f_matmul(&inv_freq.f_unsqueeze(0)?)?;
        let angles = Tensor::f_cat(&[positions.f_alias()?, positions], -1)?;
        let text_rope_cos = angles.f_cos()?;
        let text_rope_sin = angles.f_sin()?;

        Ok(Self {
            vs,
            vision_encoder,
            decoder,
            text_rope_cos,
            text_rope_sin,
            bos_token_id: tokenizer.bos_token_id(),
            eos_token_id: tokenizer.eos_token_id(),
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    /// Greedy decoding for each crop, the crops decoded together. Every
    /// crop is padded to the same patch budget, so their prefixes are the
    /// same length.
    pub(super) fn generate(
        &self,
        inputs: &[ImageInput],
        tokenizer: &Tokenizer,
        device: Device,
    ) -> Result<Vec<Vec<i64>>> {
        let kind = self.vs.kind();
        let rows = inputs.len();
        ensure!(rows > 0, "Hayai OCR needs at least one crop");
        for input in inputs {
            ensure!(
                input.pixel_values.size()[1] == siglip2::MAX_NUM_PATCHES as i64,
                "Hayai OCR expects {} patch tokens",
                siglip2::MAX_NUM_PATCHES
            );
        }
        let pixel_values = Tensor::f_cat(
            &inputs
                .iter()
                .map(|input| input.pixel_values.f_alias())
                .collect::<koharu_torch::Result<Vec<_>>>()?,
            0,
        )?
        .f_to_kind(kind)?;
        let attention_mask = Tensor::f_cat(
            &inputs
                .iter()
                .map(|input| input.attention_mask.f_alias())
                .collect::<koharu_torch::Result<Vec<_>>>()?,
            0,
        )?;
        let spatial_shapes = inputs
            .iter()
            .map(|input| input.spatial_shape)
            .collect::<Vec<_>>();

        // The encoder masks padded patches as attention keys only; their query
        // rows still produce outputs that remain part of the visual prefix.
        let key_mask = additive_mask(&attention_mask, kind)?;
        let visual_features =
            self.vision_encoder
                .forward(&pixel_values, &spatial_shapes, Some(&key_mask))?;
        let (vision_embeddings, merged_shapes) = self
            .decoder
            .projector
            .forward(&visual_features, &spatial_shapes)?;

        let bos = Tensor::f_full([rows as i64, 1], self.bos_token_id, (Kind::Int64, device))?;
        let mut hidden = Tensor::f_cat(
            &[
                vision_embeddings,
                self.decoder.token_embeddings.forward(&bos)?,
            ],
            1,
        )?;

        let m_vision = hidden.size()[1] - 1;
        let total_length = m_vision + 1;
        // Block-causal prefill mask: bidirectional across visual tokens, causal
        // onto the text token (upstream `generate_block_causal_mask`).
        let text_future = Tensor::f_cat(
            &[
                Tensor::f_full([m_vision, 1], f64::NEG_INFINITY, (kind, device))?,
                Tensor::f_zeros([1, 1], (kind, device))?,
            ],
            0,
        )?;
        let prefill_mask = Tensor::f_cat(
            &[
                Tensor::f_zeros([total_length, m_vision], (kind, device))?,
                text_future,
            ],
            1,
        )?
        .f_unsqueeze(0)?
        .f_unsqueeze(0)?;

        // Tokens past a crop's own merged grid only pad the batch; upstream
        // masks them as keys in prefill and in every decoding step.
        let vision_keys = merged_shapes
            .iter()
            .flat_map(|&[height, width]| {
                (0..m_vision).map(move |token| i64::from(token < height * width))
            })
            .collect::<Vec<_>>();
        let vision_key_mask = if vision_keys.contains(&0) {
            Some(additive_mask(
                &Tensor::f_from_slice(&vision_keys)?
                    .f_view([rows as i64, m_vision])?
                    .f_to_device(device)?,
                kind,
            )?)
        } else {
            None
        };
        let key_mask = |text_keys: i64| -> koharu_torch::Result<Option<Tensor>> {
            vision_key_mask
                .as_ref()
                .map(|vision| {
                    Tensor::f_cat(
                        &[
                            vision.f_alias()?,
                            Tensor::f_zeros([rows as i64, 1, 1, text_keys], (kind, device))?,
                        ],
                        3,
                    )
                })
                .transpose()
        };
        let prefill_mask = match key_mask(1)? {
            Some(keys) => prefill_mask.f_add(&keys)?,
            None => prefill_mask,
        };

        // Each crop's angles: its merged grid's, then its padding tokens',
        // which upstream leaves at cos=1 / sin=0 (no rotation), then the
        // first text position's.
        let mut cos = Vec::with_capacity(rows);
        let mut sin = Vec::with_capacity(rows);
        for &[patch_height, patch_width] in &merged_shapes {
            let (visual_cos, visual_sin) = self.visual_rope(patch_height, patch_width)?;
            let padding_rows = m_vision - patch_height * patch_width;
            let rope_device = self.text_rope_cos.device();
            let padding_cos = Tensor::f_ones(
                [padding_rows, decoder::HEAD_DIM / 2],
                (Kind::Float, rope_device),
            )?;
            let padding_sin = Tensor::f_zeros(
                [padding_rows, decoder::HEAD_DIM / 2],
                (Kind::Float, rope_device),
            )?;
            cos.push(Tensor::f_cat(
                &[
                    visual_cos,
                    padding_cos,
                    self.text_rope_cos.f_narrow(0, 0, 1)?,
                ],
                0,
            )?);
            sin.push(Tensor::f_cat(
                &[
                    visual_sin,
                    padding_sin,
                    self.text_rope_sin.f_narrow(0, 0, 1)?,
                ],
                0,
            )?);
        }
        let cos = Tensor::f_stack(&cos, 0)?;
        let sin = Tensor::f_stack(&sin, 0)?;

        let mut caches: Vec<LayerCache> = Vec::with_capacity(self.decoder.layers.len());
        for index in 0..self.decoder.layers.len() {
            let (next_hidden_states, present) = self.decoder.layers[index].forward(
                &hidden,
                Some(&prefill_mask),
                (&cos, &sin),
                caches.get(index),
            )?;
            hidden = next_hidden_states;
            caches.push(present);
        }
        let logits = self.decoder.logits(&hidden)?;

        let stops = |token: i64| token == self.eos_token_id || token == tokenizer.pad_token_id();
        let first_tokens = Vec::<i64>::try_from(logits.f_argmax(-1, false)?)?;
        let mut finished = first_tokens
            .iter()
            .map(|&token| stops(token))
            .collect::<Vec<_>>();
        let mut tokens = first_tokens
            .iter()
            .zip(&finished)
            .map(|(&token, &finished)| if finished { Vec::new() } else { vec![token] })
            .collect::<Vec<_>>();

        for step in 1..MAX_NEW_TOKENS {
            if finished.iter().all(|finished| *finished) {
                break;
            }
            // A finished crop is fed its stop token, and its answer is kept.
            let current = tokens
                .iter()
                .map(|tokens| tokens.last().copied().unwrap_or(self.eos_token_id))
                .collect::<Vec<_>>();
            let current_input = Tensor::f_from_slice(&current)?
                .f_view([rows as i64, 1])?
                .f_to_device(device)?;
            let embedding = self.decoder.token_embeddings.forward(&current_input)?;
            let cos_step = self.text_rope_cos.f_narrow(0, step, 1)?.f_unsqueeze(0)?;
            let sin_step = self.text_rope_sin.f_narrow(0, step, 1)?.f_unsqueeze(0)?;
            hidden = embedding;
            let step_mask = key_mask(step + 1)?;
            for (index, layer) in self.decoder.layers.iter().enumerate() {
                let (next_hidden_states, present) = layer.forward(
                    &hidden,
                    step_mask.as_ref(),
                    (&cos_step, &sin_step),
                    caches.get(index),
                )?;
                caches[index] = present;
                hidden = next_hidden_states;
            }
            let step_logits = self.decoder.logits(&hidden)?;
            let next_tokens = Vec::<i64>::try_from(step_logits.f_argmax(-1, false)?)?;
            for (row, &next_token) in next_tokens.iter().enumerate() {
                if finished[row] {
                    continue;
                }
                if stops(next_token) {
                    finished[row] = true;
                } else {
                    tokens[row].push(next_token);
                }
            }
        }
        Ok(tokens)
    }

    /// Interleaved 2D rotary angles over this image's patch grid
    /// (upstream `_get_2d_visual_freqs`), returning `(cos, sin)` of shape
    /// `(height * width, head_dim / 2)`.
    fn visual_rope(&self, height: i64, width: i64) -> koharu_torch::Result<(Tensor, Tensor)> {
        let device = self.text_rope_cos.device();
        let inv_freq = inverse_frequencies(decoder::HEAD_DIM / 2, decoder::ROPE_THETA, device)?;
        let grid_y = Tensor::f_arange(height, (Kind::Float, device))?
            .f_unsqueeze(1)?
            .f_matmul(&inv_freq.f_unsqueeze(0)?)?;
        let grid_x = Tensor::f_arange(width, (Kind::Float, device))?
            .f_unsqueeze(1)?
            .f_matmul(&inv_freq.f_unsqueeze(0)?)?;
        let grid_y = grid_y
            .f_unsqueeze(1)?
            .f_expand([height, width, -1], false)?;
        let grid_x = grid_x
            .f_unsqueeze(0)?
            .f_expand([height, width, -1], false)?;
        let angles = Tensor::f_cat(&[grid_y, grid_x], -1)?.f_flatten(0, 1)?;
        Ok((angles.f_cos()?, angles.f_sin()?))
    }
}

fn inverse_frequencies(d_axis: i64, theta: f64, device: Device) -> koharu_torch::Result<Tensor> {
    let dimensions = Tensor::f_arange_start_step(0, d_axis, 2, (Kind::Float, device))?;
    (dimensions.f_div_scalar(d_axis as f64)?)
        .f_mul_scalar(theta.ln())?
        .f_exp()?
        .f_reciprocal()
}

/// Applies an interleaved (even/odd pairs) rotary embedding over the last
/// dimension (upstream `apply_rotary_emb_2d`). Angles have shape
/// `(batch, sequence, head_dim / 2)` and broadcast across heads.
fn apply_rotary_emb(
    x: &Tensor,
    angles_cos: &Tensor,
    angles_sin: &Tensor,
) -> koharu_torch::Result<Tensor> {
    let original_kind = x.kind();
    let original_size = x.size();
    let mut pairs_shape = original_size.clone();
    let d_head = original_size[original_size.len() - 1];
    let d_axis = d_head / 2;
    let last = pairs_shape.len() - 1;
    pairs_shape[last] = d_axis;
    pairs_shape.push(2);
    let cos = angles_cos.f_unsqueeze(2)?;
    let sin = angles_sin.f_unsqueeze(2)?;
    let x = x.f_to_kind(Kind::Float)?;
    let pairs = x.f_reshape(&pairs_shape)?;
    let even = pairs.f_select(-1, 0)?;
    let odd = pairs.f_select(-1, 1)?;
    let rotated_even = (even.f_mul(&cos)?).f_sub(&odd.f_mul(&sin)?)?;
    let rotated_odd = (even.f_mul(&sin)?).f_add(&odd.f_mul(&cos)?)?;
    Tensor::f_stack(&[rotated_even, rotated_odd], -1)?
        .f_reshape(&original_size)?
        .f_to_kind(original_kind)
}

/// Converts a `[batch, keys]` padding mask into an additive SDPA bias,
/// `[batch, 1, 1, keys]`, where `0` entries become forbidden keys.
fn additive_mask(mask: &Tensor, kind: Kind) -> koharu_torch::Result<Tensor> {
    mask.f_lt(0.5)?
        .f_to_kind(Kind::Float)?
        .f_mul_scalar(f32::MIN as f64)?
        .f_to_kind(kind)?
        .f_unsqueeze(1)?
        .f_unsqueeze(1)
}

#[derive(Debug)]
struct Siglip2VisionModel {
    embeddings: VisionEmbeddings,
    layers: Vec<VisionEncoderLayer>,
    post_layernorm: nn::LayerNorm,
    _head: PoolingHead,
}

impl Siglip2VisionModel {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            embeddings: VisionEmbeddings::new(&(path / "embeddings"))?,
            layers: (0..siglip2::NUM_HIDDEN_LAYERS)
                .map(|index| VisionEncoderLayer::new(&(path / "encoder" / "layers" / index)))
                .collect::<Result<_>>()?,
            post_layernorm: nn::layer_norm(
                path / "post_layernorm",
                vec![siglip2::HIDDEN_SIZE],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            )?,
            _head: PoolingHead::new(&(path / "head"))?,
        })
    }

    fn forward(
        &self,
        pixel_values: &Tensor,
        spatial_shapes: &[[i64; 2]],
        attention_mask: Option<&Tensor>,
    ) -> koharu_torch::Result<Tensor> {
        let mut hidden_states = self.embeddings.forward(pixel_values, spatial_shapes)?;
        for layer in &self.layers {
            hidden_states = layer.forward(&hidden_states, attention_mask)?;
        }
        self.post_layernorm.forward(&hidden_states)
    }
}

/// Upstream's `Siglip2MultiheadAttentionPoolingHead`, which the encoder
/// builds as `vision_use_head` is on. Hayai reads the last hidden state, so
/// the head's weights load but never run.
/// https://github.com/huggingface/transformers/blob/598d8ba8baaec7fec5a22da0e2844c7bf4ea20e1/src/transformers/models/siglip2/modeling_siglip2.py#L664-L676
#[derive(Debug)]
struct PoolingHead {
    _probe: Tensor,
    /// `torch.nn.MultiheadAttention`'s packed query, key, and value
    /// projection, then its output projection.
    _in_proj_weight: Tensor,
    _in_proj_bias: Tensor,
    _out_proj: nn::Linear,
    _layernorm: nn::LayerNorm,
    _mlp: VisionMlp,
}

impl PoolingHead {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        let hidden = siglip2::HIDDEN_SIZE;
        let attention = path / "attention";
        Ok(Self {
            _probe: path.var("probe", &[1, 1, hidden])?,
            _in_proj_weight: attention.var("in_proj_weight", &[3 * hidden, hidden])?,
            _in_proj_bias: attention.var("in_proj_bias", &[3 * hidden])?,
            _out_proj: nn::linear(&attention / "out_proj", hidden, hidden, Default::default())?,
            _layernorm: nn::layer_norm(
                path / "layernorm",
                vec![hidden],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            )?,
            _mlp: VisionMlp::new(&(path / "mlp"))?,
        })
    }
}

#[derive(Debug)]
struct VisionEmbeddings {
    /// NaFlex patchifies on the host, so the upstream convolution collapses to
    /// a linear layer over flattened patches.
    patch_embedding: nn::Linear,
    position_embedding: nn::Embedding,
}

impl VisionEmbeddings {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            patch_embedding: nn::linear(
                path / "patch_embedding",
                siglip2::NUM_CHANNELS * siglip2::PATCH_SIZE * siglip2::PATCH_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
            position_embedding: nn::embedding(
                path / "position_embedding",
                siglip2::POSITION_EMBEDDING_SIZE * siglip2::POSITION_EMBEDDING_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
        })
    }

    /// Patch embeddings plus each image's position embeddings, resized to its
    /// patch grid (upstream `resize_positional_embeddings`, which loops over
    /// the batch).
    fn forward(
        &self,
        pixel_values: &Tensor,
        spatial_shapes: &[[i64; 2]],
    ) -> koharu_torch::Result<Tensor> {
        let patch_embeds = self.patch_embedding.forward(pixel_values)?;
        let position_embeddings = spatial_shapes
            .iter()
            .map(|&[height, width]| self.position_embeddings(height, width))
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        patch_embeds.f_add(&Tensor::f_stack(&position_embeddings, 0)?)
    }

    fn position_embeddings(&self, height: i64, width: i64) -> koharu_torch::Result<Tensor> {
        // Antialiased bilinear interpolation of the square base grid to this
        // image's patch shape, as upstream: a grid side under 16 shrinks.
        let size = siglip2::POSITION_EMBEDDING_SIZE;
        let position_embeddings = self
            .position_embedding
            .ws
            .f_reshape([size, size, siglip2::HIDDEN_SIZE])?
            .f_permute([2, 0, 1])?
            .f_unsqueeze(0)?
            .f_to_kind(Kind::Float)?
            .f_internal_upsample_bilinear2d_aa([height, width], false, None::<f64>, None::<f64>)?
            .f_to_kind(self.position_embedding.ws.kind())?
            .f_reshape([siglip2::HIDDEN_SIZE, height * width])?
            .f_transpose(0, 1)?;
        // The upstream resize pads its output buffer to the full patch budget
        // by repeating the first row, matching the zero-padded pixel values.
        let num_patches = height * width;
        let max_patches = siglip2::MAX_NUM_PATCHES as i64;
        if num_patches < max_patches {
            let first_row = position_embeddings.f_select(0, 0)?.f_unsqueeze(0)?;
            Tensor::f_cat(
                &[
                    position_embeddings,
                    first_row.f_expand([max_patches - num_patches, -1], false)?,
                ],
                0,
            )
        } else {
            Ok(position_embeddings)
        }
    }
}

#[derive(Debug)]
struct VisionEncoderLayer {
    layer_norm1: nn::LayerNorm,
    attention: VisionAttention,
    layer_norm2: nn::LayerNorm,
    mlp: VisionMlp,
}

impl VisionEncoderLayer {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            layer_norm1: nn::layer_norm(
                path / "layer_norm1",
                vec![siglip2::HIDDEN_SIZE],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            )?,
            attention: VisionAttention::new(&(path / "self_attn"))?,
            layer_norm2: nn::layer_norm(
                path / "layer_norm2",
                vec![siglip2::HIDDEN_SIZE],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            )?,
            mlp: VisionMlp::new(&(path / "mlp"))?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
    ) -> koharu_torch::Result<Tensor> {
        let attended = self
            .attention
            .forward(&self.layer_norm1.forward(hidden_states)?, attention_mask)?;
        let hidden_states = hidden_states.f_add(&attended)?;
        hidden_states.f_add(
            &self
                .mlp
                .forward(&self.layer_norm2.forward(&hidden_states)?)?,
        )
    }
}

#[derive(Debug)]
struct VisionAttention {
    query: nn::Linear,
    key: nn::Linear,
    value: nn::Linear,
    output: nn::Linear,
    num_heads: i64,
    head_dim: i64,
}

impl VisionAttention {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            query: nn::linear(
                path / "q_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
            key: nn::linear(
                path / "k_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
            value: nn::linear(
                path / "v_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
            output: nn::linear(
                path / "out_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
            num_heads: siglip2::NUM_ATTENTION_HEADS,
            head_dim: siglip2::HEAD_DIM,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
    ) -> koharu_torch::Result<Tensor> {
        let size = hidden_states.size();
        let [batch_size, sequence_length] = [size[0], size[1]];
        let shape = [batch_size, sequence_length, self.num_heads, self.head_dim];
        let query = self
            .query
            .forward(hidden_states)?
            .f_view(shape)?
            .f_transpose(1, 2)?;
        let key = self
            .key
            .forward(hidden_states)?
            .f_view(shape)?
            .f_transpose(1, 2)?;
        let value = self
            .value
            .forward(hidden_states)?
            .f_view(shape)?
            .f_transpose(1, 2)?;
        let attended = Tensor::f_scaled_dot_product_attention(
            &query,
            &key,
            &value,
            attention_mask,
            0.0,
            false,
            (self.head_dim as f64).powf(-0.5),
            false,
        )?
        .f_transpose(1, 2)?
        .f_contiguous()?
        .f_reshape([batch_size, sequence_length, self.num_heads * self.head_dim])?;
        self.output.forward(&attended)
    }
}

#[derive(Debug)]
struct VisionMlp {
    fc1: nn::Linear,
    fc2: nn::Linear,
}

impl VisionMlp {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            fc1: nn::linear(
                path / "fc1",
                siglip2::HIDDEN_SIZE,
                siglip2::INTERMEDIATE_SIZE,
                Default::default(),
            )?,
            fc2: nn::linear(
                path / "fc2",
                siglip2::INTERMEDIATE_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        // Upstream activation is `gelu_pytorch_tanh`.
        self.fc2
            .forward(&self.fc1.forward(hidden_states)?.f_gelu("tanh")?)
    }
}

#[derive(Debug)]
struct Decoder {
    projector: Projector,
    token_embeddings: nn::Embedding,
    layers: Vec<DecoderLayer>,
    final_norm: RmsNorm,
    output_head: nn::Linear,
    /// The IDS classification head, trained alongside the decoder and shipped
    /// in the checkpoint; the model card says it never runs at inference.
    _ids_norm: RmsNorm,
    _ids_head: nn::Linear,
}

impl Decoder {
    fn new(path: &nn::Path<'_>, config: &HayaiConfig) -> Result<Self> {
        Ok(Self {
            _ids_norm: RmsNorm::new(&(path / "ids_norm"), config.d_model)?,
            _ids_head: nn::linear(
                path / "ids_head",
                config.d_model,
                config.num_ids_classes,
                nn::LinearConfig { bias: false },
            )?,
            projector: Projector::new(&(path / "projector"), config.d_vision, config.d_model)?,
            token_embeddings: nn::embedding(
                path / "token_embeddings",
                config.vocab_size,
                config.d_model,
                Default::default(),
            )?,
            layers: (0..config.n_layers)
                .map(|index| {
                    DecoderLayer::new(&(path / "layers" / index), config.d_model, config.d_ffn)
                })
                .collect::<Result<_>>()?,
            final_norm: RmsNorm::new(&(path / "final_norm"), config.d_model)?,
            output_head: nn::linear(
                path / "output_head",
                config.d_model,
                config.vocab_size,
                nn::LinearConfig { bias: false },
            )?,
        })
    }

    /// Logits of the final sequence position; generation consumes nothing else.
    fn logits(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let last_hidden_state = hidden_states.f_select(1, hidden_states.size()[1] - 1)?;
        self.output_head
            .forward(&self.final_norm.forward(&last_hidden_state)?)
    }
}

/// Upstream `DSCProjector`: each 2x2 block of patches becomes one token.
#[derive(Debug)]
struct Projector {
    norm: nn::LayerNorm,
    linear1: nn::Linear,
    linear2: nn::Linear,
    out_norm: RmsNorm,
}

impl Projector {
    fn new(path: &nn::Path<'_>, d_vision: i64, d_model: i64) -> Result<Self> {
        let unshuffled = d_vision * decoder::DOWNSCALE * decoder::DOWNSCALE;
        let no_bias = nn::LinearConfig { bias: false };
        Ok(Self {
            norm: nn::layer_norm(path / "norm", vec![unshuffled], Default::default())?,
            linear1: nn::linear(path / "mlp" / 0, unshuffled, d_model, no_bias)?,
            linear2: nn::linear(path / "mlp" / 2, d_model, d_model, no_bias)?,
            out_norm: RmsNorm::new(&(path / "out_norm"), d_model)?,
        })
    }

    /// The tokens of `(batch, patches, d_vision)` patch features laid out
    /// row-major on `spatial_shapes`, padded to the batch's longest merged
    /// grid, and each crop's merged grid.
    fn forward(
        &self,
        visual_features: &Tensor,
        spatial_shapes: &[[i64; 2]],
    ) -> koharu_torch::Result<(Tensor, Vec<[i64; 2]>)> {
        let r = decoder::DOWNSCALE;
        let size = visual_features.size();
        let [rows, patches, d_vision] = [size[0], size[1], size[2]];
        let merged = spatial_shapes
            .iter()
            .map(|&[height, width]| [(height + r - 1) / r, (width + r - 1) / r])
            .collect::<Vec<_>>();
        let length = merged
            .iter()
            .map(|&[height, width]| height * width)
            .max()
            .unwrap_or(0)
            .max(1);
        // Upstream `gather_patches`: the r x r neighbours of every merged
        // token, clamped into the grid, which replicate-pads its bottom and
        // right edges. Tokens past a crop's own grid gather clamped garbage
        // that the decoder masks.
        let mut index = Vec::with_capacity((rows * length * r * r) as usize);
        for (row, (&[height, width], &[_, merged_width])) in
            spatial_shapes.iter().zip(&merged).enumerate()
        {
            let merged_width = merged_width.max(1);
            for token in 0..length {
                let (y, x) = (token / merged_width, token % merged_width);
                for dy in 0..r {
                    for dx in 0..r {
                        let source_y = (y * r + dy).min(height - 1).max(0);
                        let source_x = (x * r + dx).min(width - 1).max(0);
                        index.push(row as i64 * patches + source_y * width + source_x);
                    }
                }
            }
        }
        let index = Tensor::f_from_slice(&index)?.f_to_device(visual_features.device())?;
        let tokens = visual_features
            .f_reshape([rows * patches, d_vision])?
            .f_index_select(0, &index)?
            .f_view([rows, length, r * r, d_vision])?
            // Channel c * r * r + k, as `F.pixel_unshuffle` orders them.
            .f_transpose(2, 3)?
            .f_reshape([rows, length, d_vision * r * r])?;
        let tokens = self.linear2.forward(
            &self
                .linear1
                .forward(&self.norm.forward(&tokens)?)?
                .f_gelu("none")?,
        )?;
        Ok((self.out_norm.forward(&tokens)?, merged))
    }
}

#[derive(Debug)]
struct DecoderLayer {
    attn_norm: RmsNorm,
    attn: GroupedQueryAttention,
    ffn_norm: RmsNorm,
    ffn: SwiGlu,
    attn_res_scale: Tensor,
    ffn_res_scale: Tensor,
}

impl DecoderLayer {
    fn new(path: &nn::Path<'_>, d_model: i64, d_ffn: i64) -> Result<Self> {
        Ok(Self {
            attn_norm: RmsNorm::new(&(path / "attn_norm"), d_model)?,
            attn: GroupedQueryAttention::new(&(path / "attn"), d_model)?,
            ffn_norm: RmsNorm::new(&(path / "ffn_norm"), d_model)?,
            ffn: SwiGlu::new(&(path / "ffn"), d_model, d_ffn)?,
            attn_res_scale: path.var("attn_res_scale", &[d_model])?,
            ffn_res_scale: path.var("ffn_res_scale", &[d_model])?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
        rope: (&Tensor, &Tensor),
        past_key_value: Option<&LayerCache>,
    ) -> koharu_torch::Result<(Tensor, LayerCache)> {
        let normalized = self.attn_norm.forward(hidden_states)?;
        let (attention, present) =
            self.attn
                .forward(&normalized, attention_mask, rope, past_key_value)?;
        let hidden_states = hidden_states.f_add(&self.attn_res_scale.f_mul(&attention)?)?;
        let ffn = self.ffn.forward(&self.ffn_norm.forward(&hidden_states)?)?;
        Ok((
            hidden_states.f_add(&self.ffn_res_scale.f_mul(&ffn)?)?,
            present,
        ))
    }
}

#[derive(Debug)]
struct GroupedQueryAttention {
    w_q: nn::Linear,
    w_k: nn::Linear,
    w_v: nn::Linear,
    w_o: nn::Linear,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    query_heads: i64,
    key_value_heads: i64,
    head_dim: i64,
}

impl GroupedQueryAttention {
    fn new(path: &nn::Path<'_>, d_model: i64) -> Result<Self> {
        let no_bias = nn::LinearConfig { bias: false };
        Ok(Self {
            w_q: nn::linear(
                path / "w_q",
                d_model,
                decoder::QUERY_HEADS * decoder::HEAD_DIM,
                no_bias,
            )?,
            w_k: nn::linear(
                path / "w_k",
                d_model,
                decoder::KEY_VALUE_HEADS * decoder::HEAD_DIM,
                no_bias,
            )?,
            w_v: nn::linear(
                path / "w_v",
                d_model,
                decoder::KEY_VALUE_HEADS * decoder::HEAD_DIM,
                no_bias,
            )?,
            w_o: nn::linear(
                path / "w_o",
                decoder::QUERY_HEADS * decoder::HEAD_DIM,
                d_model,
                no_bias,
            )?,
            q_norm: RmsNorm::new(&(path / "q_norm"), decoder::HEAD_DIM)?,
            k_norm: RmsNorm::new(&(path / "k_norm"), decoder::HEAD_DIM)?,
            query_heads: decoder::QUERY_HEADS,
            key_value_heads: decoder::KEY_VALUE_HEADS,
            head_dim: decoder::HEAD_DIM,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
        (cos, sin): (&Tensor, &Tensor),
        past_key_value: Option<&LayerCache>,
    ) -> koharu_torch::Result<(Tensor, LayerCache)> {
        let size = hidden_states.size();
        let [batch_size, sequence_length] = [size[0], size[1]];
        let query_shape = [batch_size, sequence_length, self.query_heads, self.head_dim];
        let kv_shape = [
            batch_size,
            sequence_length,
            self.key_value_heads,
            self.head_dim,
        ];
        let query = apply_rotary_emb(
            &self
                .q_norm
                .forward(&self.w_q.forward(hidden_states)?.f_view(query_shape)?)?,
            cos,
            sin,
        )?
        .f_transpose(1, 2)?;
        let key = apply_rotary_emb(
            &self
                .k_norm
                .forward(&self.w_k.forward(hidden_states)?.f_view(kv_shape)?)?,
            cos,
            sin,
        )?
        .f_transpose(1, 2)?;
        let value = self
            .w_v
            .forward(hidden_states)?
            .f_view(kv_shape)?
            .f_transpose(1, 2)?;

        let (key, value) = match past_key_value {
            Some(cache) => (
                Tensor::f_cat(&[cache.key.f_alias()?, key], 2)?,
                Tensor::f_cat(&[cache.value.f_alias()?, value], 2)?,
            ),
            None => (key, value),
        };
        let present = LayerCache {
            key: key.f_alias()?,
            value: value.f_alias()?,
        };

        // Native GQA lets CUDA select flash or memory-efficient SDPA kernels
        // instead of physically repeating K/V heads like the reference code.
        let attention = Tensor::f_scaled_dot_product_attention(
            &query,
            &key,
            &value,
            attention_mask,
            0.0,
            false,
            (self.head_dim as f64).powf(-0.5),
            true,
        )?
        .f_transpose(1, 2)?
        .f_contiguous()?
        .f_reshape([
            batch_size,
            sequence_length,
            self.query_heads * self.head_dim,
        ])?;
        Ok((self.w_o.forward(&attention)?, present))
    }
}

#[derive(Debug)]
struct LayerCache {
    key: Tensor,
    value: Tensor,
}

#[derive(Debug)]
struct SwiGlu {
    w_gate: nn::Linear,
    w_up: nn::Linear,
    w_down: nn::Linear,
}

impl SwiGlu {
    fn new(path: &nn::Path<'_>, d_model: i64, d_ffn: i64) -> Result<Self> {
        let no_bias = nn::LinearConfig { bias: false };
        Ok(Self {
            w_gate: nn::linear(path / "w_gate", d_model, d_ffn, no_bias)?,
            w_up: nn::linear(path / "w_up", d_model, d_ffn, no_bias)?,
            w_down: nn::linear(path / "w_down", d_ffn, d_model, no_bias)?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.w_down.forward(
            &self
                .w_gate
                .forward(hidden_states)?
                .f_silu()?
                .f_mul(&self.w_up.forward(hidden_states)?)?,
        )
    }
}

#[derive(Debug)]
struct RmsNorm {
    weight: Tensor,
}

impl RmsNorm {
    fn new(path: &nn::Path<'_>, dim: i64) -> Result<Self> {
        Ok(Self {
            weight: path.var("weight", &[dim])?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let input_kind = hidden_states.kind();
        let hidden_states = hidden_states.f_to_kind(Kind::Float)?;
        let variance = hidden_states.f_pow_tensor_scalar(2.0)?.f_mean_dim(
            &[-1i64][..],
            true,
            Some(Kind::Float),
        )?;
        self.weight
            .f_mul(&hidden_states)?
            .f_mul(&variance.f_add_scalar(RMS_NORM_EPS)?.f_rsqrt()?)?
            .f_to_kind(input_kind)
    }
}
