//! Inference-only Hayai OCR network.
//!
//! Hayai architecture and generation implementation:
//! https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2/blob/4a4ce477c9a8841f208b94e1d9ed5c0938965e05/modeling_hayai.py
//! SigLIP2 NaFlex vision encoder instantiated by the upstream `HayaiModel`:
//! https://github.com/huggingface/transformers/blob/main/src/transformers/models/siglip2/modeling_siglip2.py

use std::path::Path;

use anyhow::{Result, ensure};
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module},
};

use super::{
    config::{HayaiConfig, decoder, siglip2},
    processor::{ImageInput, Tokenizer},
};

/// Upstream `generate` defaults. Beam search (`num_beams > 1`) is intentionally
/// not ported: greedy decoding is the upstream default path and Koharu favors
/// throughput, at a small CER cost versus the 3-4 beams recommended upstream.
const MAX_NEW_TOKENS: i64 = 256;
const REPETITION_PENALTY: f64 = 1.0;
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
    pub(super) fn new(config: &HayaiConfig, tokenizer: &Tokenizer, device: Device) -> Result<Self> {
        config.validate()?;
        let mut vs = nn::VarStore::new(device);
        crate::backend::set_precision(&mut vs);
        let root = vs.root();
        let vision_encoder = Siglip2VisionModel::new(&(&root / "vision_encoder" / "vision_model"));
        let decoder = Decoder::new(&(&root / "decoder"), config);

        // Text tokens use plain 1D RoPE; positions 0..=MAX_NEW_TOKENS cover
        // every decoding step. Visual tokens receive their own 2D frequencies
        // per spatial shape, computed during generation.
        let inv_freq = inverse_frequencies(decoder::HEAD_DIM / 2, decoder::ROPE_THETA, device);
        let positions = Tensor::arange(MAX_NEW_TOKENS + 1, (Kind::Float, device))
            .unsqueeze(1)
            .matmul(&inv_freq.unsqueeze(0));
        let angles = Tensor::cat(&[positions.shallow_clone(), positions], -1);
        let text_rope_cos = angles.cos();
        let text_rope_sin = angles.sin();

        vs.freeze();
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
        crate::backend::set_precision(&mut self.vs);
        Ok(())
    }

    pub(super) fn generate(
        &self,
        input: &ImageInput,
        tokenizer: &Tokenizer,
        device: Device,
    ) -> Result<Vec<i64>> {
        let kind = self.vs.kind();
        ensure!(
            input.pixel_values.size()[1] == siglip2::MAX_NUM_PATCHES as i64,
            "Hayai OCR expects {} patch tokens",
            siglip2::MAX_NUM_PATCHES
        );
        let pixel_values = input.pixel_values.to_kind(kind);
        let [patch_height, patch_width] = input.spatial_shape;

        // The encoder masks padded patches as attention keys only; their query
        // rows still produce outputs that remain part of the visual prefix.
        let key_mask = additive_mask(&input.attention_mask, kind);
        let visual_features =
            self.vision_encoder
                .forward(&pixel_values, patch_height, patch_width, Some(&key_mask));
        let vision_embeddings = self.decoder.projector.forward(&visual_features);

        let bos = Tensor::from_slice(&[self.bos_token_id])
            .view([1, 1])
            .to_device(pixel_values.device());
        let mut hidden = Tensor::cat(
            &[
                vision_embeddings,
                self.decoder.token_embeddings.forward(&bos),
            ],
            1,
        );

        let m_vision = hidden.size()[1] - 1;
        let total_length = m_vision + 1;
        // Block-causal prefill mask: bidirectional across visual tokens, causal
        // onto the text token (upstream `generate_block_causal_mask`).
        let text_future = Tensor::cat(
            &[
                Tensor::full([m_vision, 1], f64::NEG_INFINITY, (kind, device)),
                Tensor::zeros([1, 1], (kind, device)),
            ],
            0,
        );
        let prefill_mask = Tensor::cat(
            &[
                Tensor::zeros([total_length, m_vision], (kind, device)),
                text_future,
            ],
            1,
        )
        .unsqueeze(0)
        .unsqueeze(0);

        let (visual_cos, visual_sin) = self.visual_rope(patch_height, patch_width);
        // Rows of the visual prefix beyond this image's patch grid are padded
        // patches; upstream leaves their angles at cos=1 / sin=0 (no rotation).
        let covered_patches = patch_height * patch_width;
        let padding_rows = m_vision - covered_patches;
        let build_angles = |angles: &Tensor, padding: Tensor, first_text: &Tensor| {
            Tensor::cat(
                &[
                    angles.unsqueeze(0),
                    padding.unsqueeze(0),
                    first_text.narrow(0, 0, 1).unsqueeze(0),
                ],
                1,
            )
        };
        let identity_padding = |rows: i64, device| {
            (
                Tensor::ones([rows, decoder::HEAD_DIM / 2], (Kind::Float, device)),
                Tensor::zeros([rows, decoder::HEAD_DIM / 2], (Kind::Float, device)),
            )
        };
        let (padding_cos, padding_sin) =
            identity_padding(padding_rows, self.text_rope_cos.device());
        let cos = build_angles(&visual_cos, padding_cos, &self.text_rope_cos);
        let sin = build_angles(&visual_sin, padding_sin, &self.text_rope_sin);

        let mut caches: Vec<LayerCache> = Vec::with_capacity(self.decoder.layers.len());
        for index in 0..self.decoder.layers.len() {
            let (next_hidden_states, present) = self.decoder.layers[index].forward(
                &hidden,
                Some(&prefill_mask),
                (&cos, &sin),
                caches.get(index),
            );
            hidden = next_hidden_states;
            caches.push(present);
        }
        let mut logits = self.decoder.logits(&hidden)?;

        let mut seen_ids = vec![self.bos_token_id];
        logits = apply_repetition_penalty(&logits, &seen_ids, REPETITION_PENALTY);
        let first_token = logits.argmax(-1, false).int64_value(&[0]);
        if first_token == self.eos_token_id || first_token == tokenizer.pad_token_id() {
            return Ok(Vec::new());
        }
        let mut tokens = vec![first_token];

        for step in 1..MAX_NEW_TOKENS {
            let current_input = *tokens.last().expect("a token was generated");
            seen_ids.push(current_input);
            let current_input = Tensor::from_slice(&[current_input])
                .view([1, 1])
                .to_device(pixel_values.device());
            let embedding = self.decoder.token_embeddings.forward(&current_input);
            let cos_step = self.text_rope_cos.narrow(0, step, 1).unsqueeze(0);
            let sin_step = self.text_rope_sin.narrow(0, step, 1).unsqueeze(0);
            hidden = embedding;
            for (index, layer) in self.decoder.layers.iter().enumerate() {
                let (next_hidden_states, present) =
                    layer.forward(&hidden, None, (&cos_step, &sin_step), caches.get(index));
                caches[index] = present;
                hidden = next_hidden_states;
            }
            let mut step_logits = self.decoder.logits(&hidden)?;
            step_logits = apply_repetition_penalty(&step_logits, &seen_ids, REPETITION_PENALTY);
            let next_token = step_logits.argmax(-1, false).int64_value(&[0]);
            if next_token == self.eos_token_id || next_token == tokenizer.pad_token_id() {
                break;
            }
            tokens.push(next_token);
        }
        Ok(tokens)
    }

    /// Interleaved 2D rotary angles over this image's patch grid
    /// (upstream `_get_2d_visual_freqs`), returning `(cos, sin)` of shape
    /// `(height * width, head_dim / 2)`.
    fn visual_rope(&self, height: i64, width: i64) -> (Tensor, Tensor) {
        let device = self.text_rope_cos.device();
        let inv_freq = inverse_frequencies(decoder::HEAD_DIM / 2, decoder::ROPE_THETA, device);
        let grid_y = Tensor::arange(height, (Kind::Float, device))
            .unsqueeze(1)
            .matmul(&inv_freq.unsqueeze(0));
        let grid_x = Tensor::arange(width, (Kind::Float, device))
            .unsqueeze(1)
            .matmul(&inv_freq.unsqueeze(0));
        let grid_y = grid_y.unsqueeze(1).expand([height, width, -1], false);
        let grid_x = grid_x.unsqueeze(0).expand([height, width, -1], false);
        let angles = Tensor::cat(&[grid_y, grid_x], -1).flatten(0, 1);
        (angles.cos(), angles.sin())
    }
}

fn inverse_frequencies(d_axis: i64, theta: f64, device: Device) -> Tensor {
    let dimensions = Tensor::arange_start_step(0, d_axis, 2, (Kind::Float, device));
    (dimensions / d_axis as f64 * theta.ln()).exp().reciprocal()
}

/// Applies an interleaved (even/odd pairs) rotary embedding over the last
/// dimension (upstream `apply_rotary_emb_2d`). Angles have shape
/// `(batch, sequence, head_dim / 2)` and broadcast across heads.
fn apply_rotary_emb(x: &Tensor, angles_cos: &Tensor, angles_sin: &Tensor) -> Tensor {
    let original_kind = x.kind();
    let original_size = x.size();
    let mut pairs_shape = original_size.clone();
    let d_head = original_size[original_size.len() - 1];
    let d_axis = d_head / 2;
    let last = pairs_shape.len() - 1;
    pairs_shape[last] = d_axis;
    pairs_shape.push(2);
    let cos = angles_cos.unsqueeze(2);
    let sin = angles_sin.unsqueeze(2);
    let x = x.to_kind(Kind::Float);
    let pairs = x.reshape(&pairs_shape);
    let even = pairs.select(-1, 0);
    let odd = pairs.select(-1, 1);
    let rotated_even = &even * &cos - &odd * &sin;
    let rotated_odd = &even * &sin + &odd * &cos;
    Tensor::stack(&[rotated_even, rotated_odd], -1)
        .reshape(&original_size)
        .to_kind(original_kind)
}

fn apply_repetition_penalty(logits: &Tensor, seen_ids: &[i64], penalty: f64) -> Tensor {
    if penalty == 1.0 || seen_ids.is_empty() {
        return logits.shallow_clone();
    }
    // Matches Transformers' RepetitionPenaltyLogitsProcessor: negative scores
    // are multiplied, non-negative scores are divided, then scattered back.
    let ids = Tensor::from_slice(seen_ids).view([1, seen_ids.len() as i64]);
    let selected = logits.gather(1, &ids, false);
    let adjusted = (&selected * penalty).where_self(&selected.lt(0.0), &(&selected / penalty));
    logits.scatter(1, &ids, &adjusted)
}

/// Converts a padding mask into an additive SDPA bias where `0` entries become
/// forbidden keys.
fn additive_mask(mask: &Tensor, kind: Kind) -> Tensor {
    (mask.lt(0.5).to_kind(Kind::Float) * f32::MIN as f64)
        .to_kind(kind)
        .unsqueeze(0)
        .unsqueeze(0)
}

#[derive(Debug)]
struct Siglip2VisionModel {
    embeddings: VisionEmbeddings,
    layers: Vec<VisionEncoderLayer>,
    post_layernorm: nn::LayerNorm,
}

impl Siglip2VisionModel {
    fn new(path: &nn::Path<'_>) -> Self {
        Self {
            embeddings: VisionEmbeddings::new(&(path / "embeddings")),
            layers: (0..siglip2::NUM_HIDDEN_LAYERS)
                .map(|index| VisionEncoderLayer::new(&(path / "encoder" / "layers" / index)))
                .collect(),
            post_layernorm: nn::layer_norm(
                path / "post_layernorm",
                vec![siglip2::HIDDEN_SIZE],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            ),
        }
    }

    fn forward(
        &self,
        pixel_values: &Tensor,
        patch_height: i64,
        patch_width: i64,
        attention_mask: Option<&Tensor>,
    ) -> Tensor {
        let mut hidden_states = self
            .embeddings
            .forward(pixel_values, patch_height, patch_width);
        for layer in &self.layers {
            hidden_states = layer.forward(&hidden_states, attention_mask);
        }
        self.post_layernorm.forward(&hidden_states)
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
    fn new(path: &nn::Path<'_>) -> Self {
        Self {
            patch_embedding: nn::linear(
                path / "patch_embedding",
                siglip2::NUM_CHANNELS * siglip2::PATCH_SIZE * siglip2::PATCH_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
            position_embedding: nn::embedding(
                path / "position_embedding",
                siglip2::POSITION_EMBEDDING_SIZE * siglip2::POSITION_EMBEDDING_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
        }
    }

    fn forward(&self, pixel_values: &Tensor, height: i64, width: i64) -> Tensor {
        let patch_embeds = self.patch_embedding.forward(pixel_values);
        // Bilinear interpolation of the square base grid to this image's patch
        // shape (upstream `resize_positional_embeddings`). Upstream enables
        // antialiasing, which the LibTorch binding does not expose; with a
        // 16x16 source grid the difference is negligible.
        let size = siglip2::POSITION_EMBEDDING_SIZE;
        let position_embeddings = self
            .position_embedding
            .ws
            .reshape([size, size, siglip2::HIDDEN_SIZE])
            .permute([2, 0, 1])
            .unsqueeze(0)
            .to_kind(Kind::Float)
            .upsample_bilinear2d([height, width], false, None::<f64>, None::<f64>)
            .to_kind(self.position_embedding.ws.kind())
            .reshape([siglip2::HIDDEN_SIZE, height * width])
            .transpose(0, 1);
        // The upstream resize pads its output buffer to the full patch budget
        // by repeating the first row, matching the zero-padded pixel values.
        let num_patches = height * width;
        let max_patches = siglip2::MAX_NUM_PATCHES as i64;
        let position_embeddings = if num_patches < max_patches {
            let first_row = position_embeddings.select(0, 0).unsqueeze(0);
            Tensor::cat(
                &[
                    position_embeddings,
                    first_row.expand([max_patches - num_patches, -1], false),
                ],
                0,
            )
        } else {
            position_embeddings
        };
        patch_embeds + position_embeddings.unsqueeze(0)
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
    fn new(path: &nn::Path<'_>) -> Self {
        Self {
            layer_norm1: nn::layer_norm(
                path / "layer_norm1",
                vec![siglip2::HIDDEN_SIZE],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            ),
            attention: VisionAttention::new(&(path / "self_attn")),
            layer_norm2: nn::layer_norm(
                path / "layer_norm2",
                vec![siglip2::HIDDEN_SIZE],
                nn::LayerNormConfig {
                    eps: siglip2::LAYER_NORM_EPS,
                    ..Default::default()
                },
            ),
            mlp: VisionMlp::new(&(path / "mlp")),
        }
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: Option<&Tensor>) -> Tensor {
        let attended = self
            .attention
            .forward(&self.layer_norm1.forward(hidden_states), attention_mask);
        let hidden_states = hidden_states + attended;
        &hidden_states + self.mlp.forward(&self.layer_norm2.forward(&hidden_states))
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
    fn new(path: &nn::Path<'_>) -> Self {
        Self {
            query: nn::linear(
                path / "q_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
            key: nn::linear(
                path / "k_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
            value: nn::linear(
                path / "v_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
            output: nn::linear(
                path / "out_proj",
                siglip2::HIDDEN_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
            num_heads: siglip2::NUM_ATTENTION_HEADS,
            head_dim: siglip2::HEAD_DIM,
        }
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: Option<&Tensor>) -> Tensor {
        let size = hidden_states.size();
        let [batch_size, sequence_length] = [size[0], size[1]];
        let shape = [batch_size, sequence_length, self.num_heads, self.head_dim];
        let query = self
            .query
            .forward(hidden_states)
            .view(shape)
            .transpose(1, 2);
        let key = self.key.forward(hidden_states).view(shape).transpose(1, 2);
        let value = self
            .value
            .forward(hidden_states)
            .view(shape)
            .transpose(1, 2);
        let attended = Tensor::scaled_dot_product_attention(
            &query,
            &key,
            &value,
            attention_mask,
            0.0,
            false,
            (self.head_dim as f64).powf(-0.5),
            false,
        )
        .transpose(1, 2)
        .contiguous()
        .reshape([batch_size, sequence_length, self.num_heads * self.head_dim]);
        self.output.forward(&attended)
    }
}

#[derive(Debug)]
struct VisionMlp {
    fc1: nn::Linear,
    fc2: nn::Linear,
}

impl VisionMlp {
    fn new(path: &nn::Path<'_>) -> Self {
        Self {
            fc1: nn::linear(
                path / "fc1",
                siglip2::HIDDEN_SIZE,
                siglip2::INTERMEDIATE_SIZE,
                Default::default(),
            ),
            fc2: nn::linear(
                path / "fc2",
                siglip2::INTERMEDIATE_SIZE,
                siglip2::HIDDEN_SIZE,
                Default::default(),
            ),
        }
    }

    fn forward(&self, hidden_states: &Tensor) -> Tensor {
        // Upstream activation is `gelu_pytorch_tanh`.
        self.fc2
            .forward(&self.fc1.forward(hidden_states).gelu("tanh"))
    }
}

#[derive(Debug)]
struct Decoder {
    projector: Projector,
    token_embeddings: nn::Embedding,
    layers: Vec<DecoderLayer>,
    final_norm: RmsNorm,
    output_head: nn::Linear,
}

impl Decoder {
    fn new(path: &nn::Path<'_>, config: &HayaiConfig) -> Self {
        Self {
            projector: Projector::new(&(path / "projector"), config.d_vision, config.d_model),
            token_embeddings: nn::embedding(
                path / "token_embeddings",
                config.vocab_size,
                config.d_model,
                Default::default(),
            ),
            layers: (0..config.n_layers)
                .map(|index| {
                    DecoderLayer::new(&(path / "layers" / index), config.d_model, config.d_ffn)
                })
                .collect(),
            final_norm: RmsNorm::new(&(path / "final_norm"), config.d_model),
            output_head: nn::linear(
                path / "output_head",
                config.d_model,
                config.vocab_size,
                nn::LinearConfig {
                    bias: false,
                    ..Default::default()
                },
            ),
        }
    }

    /// Logits of the final sequence position; generation consumes nothing else.
    fn logits(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let last_hidden_state = hidden_states.select(1, hidden_states.size()[1] - 1);
        Ok(self
            .output_head
            .forward(&self.final_norm.forward(&last_hidden_state)))
    }
}

#[derive(Debug)]
struct Projector {
    linear1: nn::Linear,
    linear2: nn::Linear,
}

impl Projector {
    fn new(path: &nn::Path<'_>, d_vision: i64, d_model: i64) -> Self {
        Self {
            linear1: nn::linear(path / "proj" / 0, d_vision, d_model, Default::default()),
            linear2: nn::linear(path / "proj" / 2, d_model, d_model, Default::default()),
        }
    }

    fn forward(&self, hidden_states: &Tensor) -> Tensor {
        self.linear2
            .forward(&self.linear1.forward(hidden_states).gelu("none"))
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
    fn new(path: &nn::Path<'_>, d_model: i64, d_ffn: i64) -> Self {
        Self {
            attn_norm: RmsNorm::new(&(path / "attn_norm"), d_model),
            attn: GroupedQueryAttention::new(&(path / "attn"), d_model),
            ffn_norm: RmsNorm::new(&(path / "ffn_norm"), d_model),
            ffn: SwiGlu::new(&(path / "ffn"), d_model, d_ffn),
            attn_res_scale: path.var("attn_res_scale", &[d_model], nn::Init::Const(1.0)),
            ffn_res_scale: path.var("ffn_res_scale", &[d_model], nn::Init::Const(1.0)),
        }
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
        rope: (&Tensor, &Tensor),
        past_key_value: Option<&LayerCache>,
    ) -> (Tensor, LayerCache) {
        let normalized = self.attn_norm.forward(hidden_states);
        let (attention, present) =
            self.attn
                .forward(&normalized, attention_mask, rope, past_key_value);
        let hidden_states = hidden_states + &self.attn_res_scale * attention;
        let ffn = self.ffn.forward(&self.ffn_norm.forward(&hidden_states));
        (&hidden_states + &self.ffn_res_scale * ffn, present)
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
    fn new(path: &nn::Path<'_>, d_model: i64) -> Self {
        let no_bias = nn::LinearConfig {
            bias: false,
            ..Default::default()
        };
        Self {
            w_q: nn::linear(
                path / "w_q",
                d_model,
                decoder::QUERY_HEADS * decoder::HEAD_DIM,
                no_bias,
            ),
            w_k: nn::linear(
                path / "w_k",
                d_model,
                decoder::KEY_VALUE_HEADS * decoder::HEAD_DIM,
                no_bias,
            ),
            w_v: nn::linear(
                path / "w_v",
                d_model,
                decoder::KEY_VALUE_HEADS * decoder::HEAD_DIM,
                no_bias,
            ),
            w_o: nn::linear(
                path / "w_o",
                decoder::QUERY_HEADS * decoder::HEAD_DIM,
                d_model,
                no_bias,
            ),
            q_norm: RmsNorm::new(&(path / "q_norm"), decoder::HEAD_DIM),
            k_norm: RmsNorm::new(&(path / "k_norm"), decoder::HEAD_DIM),
            query_heads: decoder::QUERY_HEADS,
            key_value_heads: decoder::KEY_VALUE_HEADS,
            head_dim: decoder::HEAD_DIM,
        }
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: Option<&Tensor>,
        (cos, sin): (&Tensor, &Tensor),
        past_key_value: Option<&LayerCache>,
    ) -> (Tensor, LayerCache) {
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
                .forward(&self.w_q.forward(hidden_states).view(query_shape)),
            cos,
            sin,
        )
        .transpose(1, 2);
        let key = apply_rotary_emb(
            &self
                .k_norm
                .forward(&self.w_k.forward(hidden_states).view(kv_shape)),
            cos,
            sin,
        )
        .transpose(1, 2);
        let value = self
            .w_v
            .forward(hidden_states)
            .view(kv_shape)
            .transpose(1, 2);

        let (key, value) = match past_key_value {
            Some(cache) => (
                Tensor::cat(&[cache.key.shallow_clone(), key], 2),
                Tensor::cat(&[cache.value.shallow_clone(), value], 2),
            ),
            None => (key, value),
        };
        let present = LayerCache {
            key: key.shallow_clone(),
            value: value.shallow_clone(),
        };

        // Native GQA lets CUDA select flash or memory-efficient SDPA kernels
        // instead of physically repeating K/V heads like the reference code.
        let attention = Tensor::scaled_dot_product_attention(
            &query,
            &key,
            &value,
            attention_mask,
            0.0,
            false,
            (self.head_dim as f64).powf(-0.5),
            true,
        )
        .transpose(1, 2)
        .contiguous()
        .reshape([
            batch_size,
            sequence_length,
            self.query_heads * self.head_dim,
        ]);
        (self.w_o.forward(&attention), present)
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
    fn new(path: &nn::Path<'_>, d_model: i64, d_ffn: i64) -> Self {
        let no_bias = nn::LinearConfig {
            bias: false,
            ..Default::default()
        };
        Self {
            w_gate: nn::linear(path / "w_gate", d_model, d_ffn, no_bias),
            w_up: nn::linear(path / "w_up", d_model, d_ffn, no_bias),
            w_down: nn::linear(path / "w_down", d_ffn, d_model, no_bias),
        }
    }

    fn forward(&self, hidden_states: &Tensor) -> Tensor {
        self.w_down.forward(
            &(self.w_gate.forward(hidden_states).silu() * self.w_up.forward(hidden_states)),
        )
    }
}

#[derive(Debug)]
struct RmsNorm {
    weight: Tensor,
}

impl RmsNorm {
    fn new(path: &nn::Path<'_>, dim: i64) -> Self {
        Self {
            weight: path.var("weight", &[dim], nn::Init::Const(1.0)),
        }
    }

    fn forward(&self, hidden_states: &Tensor) -> Tensor {
        let input_kind = hidden_states.kind();
        let hidden_states = hidden_states.to_kind(Kind::Float);
        let variance =
            hidden_states
                .pow_tensor_scalar(2.0)
                .mean_dim(&[-1i64][..], true, Some(Kind::Float));
        (&self.weight * hidden_states * (variance + RMS_NORM_EPS).rsqrt()).to_kind(input_kind)
    }
}
