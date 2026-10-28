//! Transformers 4.15 ViT + BERT `VisionEncoderDecoderModel` used by Manga OCR.
//!
//! Original implementations:
//! - https://github.com/huggingface/transformers/blob/05fa1a7ac17bb7aa07b9e0c1e138ecb31a28bbfe/src/transformers/models/vit/modeling_vit.py
//! - https://github.com/huggingface/transformers/blob/05fa1a7ac17bb7aa07b9e0c1e138ecb31a28bbfe/src/transformers/models/bert/modeling_bert.py
//! - https://github.com/huggingface/transformers/blob/05fa1a7ac17bb7aa07b9e0c1e138ecb31a28bbfe/src/transformers/generation_utils.py#L1730
//! - https://github.com/huggingface/transformers/blob/05fa1a7ac17bb7aa07b9e0c1e138ecb31a28bbfe/src/transformers/generation_beam_search.py

use std::path::Path;

use crate::nn::{self, Module};
use anyhow::{Result, ensure};
use tch::{Device, Kind, Tensor};

use super::config::{BertConfig, MangaOcrConfig, ViTConfig};

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    encoder: ViTModel,
    decoder: BertLMHeadModel,
    generation: GenerationConfig,
}

impl Model {
    pub(super) fn new(config: &MangaOcrConfig, device: Device, kind: Kind) -> Result<Self> {
        ensure!(
            config.encoder.hidden_size == config.decoder.hidden_size,
            "Manga OCR encoder and decoder hidden sizes must match"
        );
        ensure!(config.num_beams > 1, "Manga OCR requires beam search");
        ensure!(
            config.max_length > 1,
            "Manga OCR max_length must exceed one"
        );
        ensure!(
            config.pad_token_id == config.decoder.pad_token_id,
            "Manga OCR model and decoder pad token IDs must match"
        );

        let vs = nn::VarStore::new(device, kind);
        let encoder = ViTModel::new(&(&vs.root() / "encoder"), &config.encoder)?;
        let decoder = BertLMHeadModel::new(&(&vs.root() / "decoder"), &config.decoder)?;
        Ok(Self {
            vs,
            encoder,
            decoder,
            generation: GenerationConfig {
                decoder_start_token_id: config.decoder_start_token_id,
                eos_token_id: config.eos_token_id,
                pad_token_id: config.pad_token_id,
                max_length: config.max_length,
                num_beams: config.num_beams,
                length_penalty: config.length_penalty,
                early_stopping: config.early_stopping,
                no_repeat_ngram_size: config.no_repeat_ngram_size,
                vocab_size: config.decoder.vocab_size,
            },
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    /// The token ids for each image of `pixel_values`, `[batch, 3, height,
    /// width]`.
    pub(super) fn forward(&self, pixel_values: &Tensor) -> Result<Vec<Vec<i64>>> {
        let size = pixel_values.size();
        ensure!(
            size.len() == 4 && size[0] > 0,
            "Manga OCR expects a batch of images, got {size:?}"
        );
        let encoder_hidden_states = self
            .encoder
            .forward(&pixel_values.f_to_kind(self.vs.kind())?)?;
        self.beam_search(&encoder_hidden_states)
    }

    /// `generation_utils.beam_search` with a `BeamSearchScorer` for every
    /// image of the batch: an image whose hypotheses are done has its beams
    /// padded while the others go on.
    fn beam_search(&self, encoder_hidden_states: &Tensor) -> Result<Vec<Vec<i64>>> {
        let config = &self.generation;
        let device = encoder_hidden_states.device();
        let num_beams = config.num_beams;
        let encoder_size = encoder_hidden_states.size();
        let batch_size = encoder_size[0] as usize;
        let rows = batch_size * num_beams;
        let encoder_hidden_states = encoder_hidden_states
            .f_unsqueeze(1)?
            .f_repeat([1, num_beams as i64, 1, 1])?
            .f_view([rows as i64, encoder_size[1], encoder_size[2]])?;

        let mut sequences = vec![vec![config.decoder_start_token_id]; rows];
        // Only each image's first beam is live at the start, so the first
        // step doesn't pick one token num_beams times.
        let mut beam_scores = (0..rows)
            .map(|row| if row % num_beams == 0 { 0.0 } else { -1.0e9f32 })
            .collect::<Vec<_>>();
        let mut input_ids = Tensor::f_full(
            [rows as i64, 1],
            config.decoder_start_token_id,
            (Kind::Int64, device),
        )?;
        let mut cache: Option<Vec<LayerCache>> = None;
        let mut hypotheses = (0..batch_size)
            .map(|_| BeamHypotheses::new(num_beams, config.length_penalty, config.early_stopping))
            .collect::<Vec<_>>();

        while sequences[0].len() < config.max_length {
            // BeamSearchScorer evaluates its completion bound against input_ids before
            // appending the token selected by this iteration.
            let current_length = sequences[0].len();
            let (logits, next_cache) =
                self.decoder
                    .forward(&input_ids, &encoder_hidden_states, cache.as_deref())?;
            let mut next_token_scores = logits
                .f_select(1, logits.size()[1] - 1)?
                .f_log_softmax(-1, Kind::Float)?;
            let banned = no_repeat_ngram_mask(
                &sequences,
                config.no_repeat_ngram_size,
                config.vocab_size as usize,
                device,
            )?;
            next_token_scores = next_token_scores.f_masked_fill(&banned, f64::NEG_INFINITY)?;
            next_token_scores = next_token_scores.f_add(
                &Tensor::f_from_slice(&beam_scores)?
                    .f_to_device(device)?
                    .f_unsqueeze(1)?,
            )?;

            let candidates = 2 * num_beams;
            let (candidate_scores, candidate_indices) = next_token_scores
                .f_view([batch_size as i64, num_beams as i64 * config.vocab_size])?
                .f_topk(candidates as i64, 1, true, true)?;
            let candidate_scores = Vec::<f32>::try_from(candidate_scores.f_reshape([-1])?)?;
            let candidate_indices = Vec::<i64>::try_from(candidate_indices.f_reshape([-1])?)?;

            let mut next_sequences = Vec::with_capacity(rows);
            let mut next_scores = Vec::with_capacity(rows);
            let mut next_tokens = Vec::with_capacity(rows);
            let mut beam_indices = Vec::with_capacity(rows);
            for (batch, hypotheses) in hypotheses.iter_mut().enumerate() {
                let first = batch * num_beams;
                if hypotheses.done {
                    for (beam, sequence) in sequences.iter().enumerate().skip(first).take(num_beams)
                    {
                        let mut sequence = sequence.clone();
                        sequence.push(config.pad_token_id);
                        next_sequences.push(sequence);
                        next_scores.push(0.0);
                        next_tokens.push(config.pad_token_id);
                        beam_indices.push(beam as i64);
                    }
                    continue;
                }
                let filled = next_sequences.len();
                let ranked = batch * candidates..(batch + 1) * candidates;
                for (rank, (&score, &flat_index)) in candidate_scores[ranked.clone()]
                    .iter()
                    .zip(&candidate_indices[ranked])
                    .enumerate()
                {
                    let beam_index = first as i64 + flat_index / config.vocab_size;
                    let token_id = flat_index % config.vocab_size;
                    if token_id == config.eos_token_id {
                        if rank < num_beams {
                            hypotheses.add(sequences[beam_index as usize].clone(), score as f64);
                        }
                        continue;
                    }

                    let mut sequence = sequences[beam_index as usize].clone();
                    sequence.push(token_id);
                    next_sequences.push(sequence);
                    next_scores.push(score);
                    next_tokens.push(token_id);
                    beam_indices.push(beam_index);
                    if next_sequences.len() - filled == num_beams {
                        break;
                    }
                }
                ensure!(
                    next_sequences.len() - filled == num_beams,
                    "Manga OCR beam search could not fill the next beam"
                );
                hypotheses.is_done(next_scores[filled] as f64, current_length);
            }

            sequences = next_sequences;
            beam_scores = next_scores;
            let beam_indices = Tensor::f_from_slice(&beam_indices)?.f_to_device(device)?;
            cache = Some(reorder_cache(next_cache, &beam_indices)?);
            input_ids = Tensor::f_from_slice(&next_tokens)?
                .f_to_device(device)?
                .f_view([rows as i64, 1])?;

            if hypotheses.iter().all(|hypotheses| hypotheses.done) {
                break;
            }
        }

        hypotheses
            .into_iter()
            .enumerate()
            .map(|(batch, mut hypotheses)| {
                if !hypotheses.done {
                    for row in batch * num_beams..(batch + 1) * num_beams {
                        hypotheses.add(sequences[row].clone(), beam_scores[row] as f64);
                    }
                }
                let mut best = hypotheses.best()?;
                if best.len() < config.max_length {
                    best.push(config.eos_token_id);
                }
                Ok(best)
            })
            .collect()
    }
}

#[derive(Debug)]
struct GenerationConfig {
    decoder_start_token_id: i64,
    eos_token_id: i64,
    pad_token_id: i64,
    max_length: usize,
    num_beams: usize,
    length_penalty: f64,
    early_stopping: bool,
    no_repeat_ngram_size: usize,
    vocab_size: i64,
}

#[derive(Debug)]
struct BeamHypotheses {
    num_beams: usize,
    length_penalty: f64,
    early_stopping: bool,
    beams: Vec<(f64, Vec<i64>)>,
    worst_score: f64,
    done: bool,
}

impl BeamHypotheses {
    fn new(num_beams: usize, length_penalty: f64, early_stopping: bool) -> Self {
        Self {
            num_beams,
            length_penalty,
            early_stopping,
            beams: Vec::with_capacity(num_beams),
            worst_score: f64::INFINITY,
            done: false,
        }
    }

    fn add(&mut self, tokens: Vec<i64>, sum_logprobs: f64) {
        let score = sum_logprobs / (tokens.len() as f64).powf(self.length_penalty);
        if self.beams.len() < self.num_beams || score > self.worst_score {
            self.beams.push((score, tokens));
            if self.beams.len() > self.num_beams {
                let worst = self
                    .beams
                    .iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| left.0.total_cmp(&right.0))
                    .map(|(index, _)| index)
                    .unwrap();
                self.beams.swap_remove(worst);
            }
            self.worst_score = self
                .beams
                .iter()
                .map(|(candidate, _)| *candidate)
                .fold(f64::INFINITY, f64::min);
        }
    }

    fn is_done(&mut self, best_sum_logprobs: f64, current_length: usize) -> bool {
        self.done = if self.beams.len() < self.num_beams {
            false
        } else if self.early_stopping {
            true
        } else {
            self.worst_score
                >= best_sum_logprobs / (current_length as f64).powf(self.length_penalty)
        };
        self.done
    }

    fn best(mut self) -> Result<Vec<i64>> {
        self.beams.sort_by(|left, right| left.0.total_cmp(&right.0));
        self.beams
            .pop()
            .map(|(_, tokens)| tokens)
            .ok_or_else(|| anyhow::anyhow!("Manga OCR beam search produced no hypothesis"))
    }
}

fn no_repeat_ngram_mask(
    sequences: &[Vec<i64>],
    ngram_size: usize,
    vocab_size: usize,
    device: Device,
) -> tch::Result<Tensor> {
    let mut mask = vec![0u8; sequences.len() * vocab_size];
    if ngram_size == 0 {
        return Tensor::f_from_slice(&mask)?
            .f_view([sequences.len() as i64, vocab_size as i64])?
            .f_to_device(device)?
            .f_to_kind(Kind::Bool);
    }
    for (row, sequence) in sequences.iter().enumerate() {
        for token in banned_ngram_tokens(sequence, ngram_size) {
            mask[row * vocab_size + token as usize] = 1;
        }
    }
    Tensor::f_from_slice(&mask)?
        .f_view([sequences.len() as i64, vocab_size as i64])?
        .f_to_device(device)?
        .f_to_kind(Kind::Bool)
}

fn banned_ngram_tokens(sequence: &[i64], ngram_size: usize) -> Vec<i64> {
    if ngram_size == 0 || sequence.len() + 1 < ngram_size {
        return Vec::new();
    }
    let prefix_length = ngram_size - 1;
    let prefix = &sequence[sequence.len() - prefix_length..];
    sequence
        .windows(ngram_size)
        .filter(|ngram| &ngram[..prefix_length] == prefix)
        .map(|ngram| ngram[prefix_length])
        .collect()
}

fn reorder_cache(cache: Vec<LayerCache>, beam_indices: &Tensor) -> tch::Result<Vec<LayerCache>> {
    cache
        .into_iter()
        .map(|layer| {
            Ok(LayerCache {
                self_key: layer.self_key.f_index_select(0, beam_indices)?,
                self_value: layer.self_value.f_index_select(0, beam_indices)?,
                cross_key: layer.cross_key.f_index_select(0, beam_indices)?,
                cross_value: layer.cross_value.f_index_select(0, beam_indices)?,
            })
        })
        .collect()
}

#[derive(Debug)]
struct ViTModel {
    embeddings: ViTEmbeddings,
    encoder: ViTEncoder,
    layernorm: nn::LayerNorm,
    pooler: ViTPooler,
}

impl ViTModel {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        Ok(Self {
            embeddings: ViTEmbeddings::new(&(path / "embeddings"), config)?,
            encoder: ViTEncoder::new(&(path / "encoder"), config)?,
            layernorm: layer_norm(
                &(path / "layernorm"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
            pooler: ViTPooler::new(&(path / "pooler"), config)?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = self.embeddings.forward(pixel_values)?;
        let hidden_states = self.encoder.forward(&hidden_states)?;
        let hidden_states = self.layernorm.forward(&hidden_states)?;
        let _pooled_output = self.pooler.forward(&hidden_states)?;
        Ok(hidden_states)
    }
}

#[derive(Debug)]
struct ViTEmbeddings {
    cls_token: Tensor,
    position_embeddings: Tensor,
    patch_embeddings: PatchEmbeddings,
}

impl ViTEmbeddings {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        let patches = (config.image_size / config.patch_size).pow(2);
        Ok(Self {
            cls_token: path.var("cls_token", &[1, 1, config.hidden_size])?,
            position_embeddings: path
                .var("position_embeddings", &[1, patches + 1, config.hidden_size])?,
            patch_embeddings: PatchEmbeddings::new(&(path / "patch_embeddings"), config)?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        let batch_size = pixel_values.size()[0];
        let embeddings = self.patch_embeddings.forward(pixel_values)?;
        let cls_tokens = self.cls_token.f_expand([batch_size, -1, -1], true)?;
        (Tensor::f_cat(&[cls_tokens, embeddings], 1)?).f_add(&self.position_embeddings)
    }
}

#[derive(Debug)]
struct PatchEmbeddings {
    projection: nn::Conv2D,
    image_size: i64,
}

impl PatchEmbeddings {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        Ok(Self {
            projection: nn::conv2d(
                path / "projection",
                config.num_channels,
                config.hidden_size,
                config.patch_size,
                nn::ConvConfig {
                    stride: config.patch_size,
                    ..Default::default()
                },
            )?,
            image_size: config.image_size,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        assert_eq!(
            &pixel_values.size()[2..],
            &[self.image_size, self.image_size]
        );
        self.projection
            .forward(pixel_values)?
            .f_flatten(2, 3)?
            .f_transpose(1, 2)
    }
}

#[derive(Debug)]
struct ViTEncoder {
    layer: Vec<ViTLayer>,
}

impl ViTEncoder {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        Ok(Self {
            layer: (0..config.num_hidden_layers)
                .map(|index| ViTLayer::new(&(path / "layer" / index), config))
                .collect::<Result<_>>()?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        self.layer
            .iter()
            .try_fold(hidden_states.f_alias()?, |hidden, layer| {
                layer.forward(&hidden)
            })
    }
}

#[derive(Debug)]
struct ViTLayer {
    attention: ViTAttention,
    intermediate: nn::Linear,
    output: nn::Linear,
    layernorm_before: nn::LayerNorm,
    layernorm_after: nn::LayerNorm,
}

impl ViTLayer {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        Ok(Self {
            attention: ViTAttention::new(&(path / "attention"), config)?,
            intermediate: linear(
                &(path / "intermediate" / "dense"),
                config.hidden_size,
                config.intermediate_size,
                true,
            )?,
            output: linear(
                &(path / "output" / "dense"),
                config.intermediate_size,
                config.hidden_size,
                true,
            )?,
            layernorm_before: layer_norm(
                &(path / "layernorm_before"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
            layernorm_after: layer_norm(
                &(path / "layernorm_after"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = hidden_states.f_add(
            &self
                .attention
                .forward(&self.layernorm_before.forward(hidden_states)?)?,
        )?;
        let intermediate = self
            .intermediate
            .forward(&self.layernorm_after.forward(&hidden_states)?)?
            .f_gelu("none")?;
        hidden_states.f_add(&self.output.forward(&intermediate)?)
    }
}

#[derive(Debug)]
struct ViTAttention {
    query: nn::Linear,
    key: nn::Linear,
    value: nn::Linear,
    output: nn::Linear,
    num_heads: i64,
    head_dim: i64,
}

impl ViTAttention {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        let head_dim = config.hidden_size / config.num_attention_heads;
        let attention = path / "attention";
        Ok(Self {
            query: linear(
                &(&attention / "query"),
                config.hidden_size,
                config.hidden_size,
                config.qkv_bias,
            )?,
            key: linear(
                &(&attention / "key"),
                config.hidden_size,
                config.hidden_size,
                config.qkv_bias,
            )?,
            value: linear(
                &(&attention / "value"),
                config.hidden_size,
                config.hidden_size,
                config.qkv_bias,
            )?,
            output: linear(
                &(path / "output" / "dense"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
            num_heads: config.num_attention_heads,
            head_dim,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let query = self.transpose_for_scores(&self.query.forward(hidden_states)?)?;
        let key = self.transpose_for_scores(&self.key.forward(hidden_states)?)?;
        let value = self.transpose_for_scores(&self.value.forward(hidden_states)?)?;
        let probabilities = (query.f_matmul(&key.f_transpose(-1, -2)?)?)
            .f_div_scalar((self.head_dim as f64).sqrt())?
            .f_softmax(-1, Kind::Float)?
            .f_to_kind(value.kind())?;
        let size = hidden_states.size();
        let context = probabilities
            .f_matmul(&value)?
            .f_transpose(1, 2)?
            .f_contiguous()?
            .f_view([size[0], size[1], self.num_heads * self.head_dim])?;
        self.output.forward(&context)
    }

    fn transpose_for_scores(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let size = hidden_states.size();
        hidden_states
            .f_view([size[0], size[1], self.num_heads, self.head_dim])?
            .f_permute([0, 2, 1, 3])
    }
}

#[derive(Debug)]
struct ViTPooler {
    dense: nn::Linear,
}

impl ViTPooler {
    fn new(path: &nn::Path<'_>, config: &ViTConfig) -> Result<Self> {
        Ok(Self {
            dense: linear(
                &(path / "dense"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        self.dense.forward(&hidden_states.f_select(1, 0)?)?.f_tanh()
    }
}

#[derive(Debug)]
struct BertLMHeadModel {
    bert: BertModel,
    cls: BertOnlyMLMHead,
}

impl BertLMHeadModel {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        Ok(Self {
            bert: BertModel::new(&(path / "bert"), config)?,
            cls: BertOnlyMLMHead::new(&(path / "cls"), config)?,
        })
    }

    fn forward(
        &self,
        input_ids: &Tensor,
        encoder_hidden_states: &Tensor,
        cache: Option<&[LayerCache]>,
    ) -> tch::Result<(Tensor, Vec<LayerCache>)> {
        let (sequence_output, cache) =
            self.bert.forward(input_ids, encoder_hidden_states, cache)?;
        Ok((self.cls.forward(&sequence_output)?, cache))
    }
}

#[derive(Debug)]
struct BertModel {
    embeddings: BertEmbeddings,
    encoder: BertEncoder,
}

impl BertModel {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        Ok(Self {
            embeddings: BertEmbeddings::new(&(path / "embeddings"), config)?,
            encoder: BertEncoder::new(&(path / "encoder"), config)?,
        })
    }

    fn forward(
        &self,
        input_ids: &Tensor,
        encoder_hidden_states: &Tensor,
        cache: Option<&[LayerCache]>,
    ) -> tch::Result<(Tensor, Vec<LayerCache>)> {
        let past_length = cache.map_or(0, |cache| cache[0].self_key.size()[2]);
        let embeddings = self.embeddings.forward(input_ids, past_length)?;
        self.encoder
            .forward(&embeddings, encoder_hidden_states, cache)
    }
}

#[derive(Debug)]
struct BertEmbeddings {
    word_embeddings: nn::Embedding,
    position_embeddings: nn::Embedding,
    token_type_embeddings: nn::Embedding,
    layer_norm: nn::LayerNorm,
    position_ids: Tensor,
}

impl BertEmbeddings {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        let position_ids = path.var_with_kind(
            "position_ids",
            &[1, config.max_position_embeddings],
            Kind::Int64,
        )?;
        Ok(Self {
            word_embeddings: nn::embedding(
                path / "word_embeddings",
                config.vocab_size,
                config.hidden_size,
                nn::EmbeddingConfig {
                    padding_idx: config.pad_token_id,
                    ..Default::default()
                },
            )?,
            position_embeddings: nn::embedding(
                path / "position_embeddings",
                config.max_position_embeddings,
                config.hidden_size,
                Default::default(),
            )?,
            token_type_embeddings: nn::embedding(
                path / "token_type_embeddings",
                config.type_vocab_size,
                config.hidden_size,
                Default::default(),
            )?,
            layer_norm: layer_norm(
                &(path / "LayerNorm"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
            position_ids,
        })
    }

    fn forward(&self, input_ids: &Tensor, past_length: i64) -> tch::Result<Tensor> {
        let sequence_length = input_ids.size()[1];
        let position_ids = self
            .position_ids
            .f_narrow(1, past_length, sequence_length)?;
        let token_type_ids = Tensor::f_zeros_like(input_ids)?;
        self.layer_norm.forward(
            &(self
                .word_embeddings
                .forward(input_ids)?
                .f_add(&self.token_type_embeddings.forward(&token_type_ids)?)?)
            .f_add(&self.position_embeddings.forward(&position_ids)?)?,
        )
    }
}

#[derive(Debug)]
struct BertEncoder {
    layer: Vec<BertLayer>,
}

impl BertEncoder {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        Ok(Self {
            layer: (0..config.num_hidden_layers)
                .map(|index| BertLayer::new(&(path / "layer" / index), config))
                .collect::<Result<_>>()?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        encoder_hidden_states: &Tensor,
        cache: Option<&[LayerCache]>,
    ) -> tch::Result<(Tensor, Vec<LayerCache>)> {
        let mut hidden_states = hidden_states.f_alias()?;
        let mut next_cache = Vec::with_capacity(self.layer.len());
        for (index, layer) in self.layer.iter().enumerate() {
            let output = layer.forward(
                &hidden_states,
                encoder_hidden_states,
                cache.map(|cache| &cache[index]),
            )?;
            hidden_states = output.0;
            next_cache.push(output.1);
        }
        Ok((hidden_states, next_cache))
    }
}

#[derive(Debug)]
struct LayerCache {
    self_key: Tensor,
    self_value: Tensor,
    cross_key: Tensor,
    cross_value: Tensor,
}

#[derive(Debug)]
struct BertLayer {
    attention: BertAttention,
    crossattention: BertAttention,
    intermediate: nn::Linear,
    output: nn::Linear,
    output_layer_norm: nn::LayerNorm,
}

impl BertLayer {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        Ok(Self {
            attention: BertAttention::new(&(path / "attention"), config)?,
            crossattention: BertAttention::new(&(path / "crossattention"), config)?,
            intermediate: linear(
                &(path / "intermediate" / "dense"),
                config.hidden_size,
                config.intermediate_size,
                true,
            )?,
            output: linear(
                &(path / "output" / "dense"),
                config.intermediate_size,
                config.hidden_size,
                true,
            )?,
            output_layer_norm: layer_norm(
                &(path / "output" / "LayerNorm"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        encoder_hidden_states: &Tensor,
        cache: Option<&LayerCache>,
    ) -> tch::Result<(Tensor, LayerCache)> {
        let (attention_output, self_key, self_value) = self.attention.forward(
            hidden_states,
            None,
            cache.map(|cache| (&cache.self_key, &cache.self_value)),
        )?;
        let (attention_output, cross_key, cross_value) = self.crossattention.forward(
            &attention_output,
            Some(encoder_hidden_states),
            cache.map(|cache| (&cache.cross_key, &cache.cross_value)),
        )?;
        let intermediate = self
            .intermediate
            .forward(&attention_output)?
            .f_gelu("none")?;
        let hidden_states = self
            .output_layer_norm
            .forward(&(self.output.forward(&intermediate)?).f_add(&attention_output)?)?;
        Ok((
            hidden_states,
            LayerCache {
                self_key,
                self_value,
                cross_key,
                cross_value,
            },
        ))
    }
}

#[derive(Debug)]
struct BertAttention {
    self_attention: BertSelfAttention,
    dense: nn::Linear,
    layer_norm: nn::LayerNorm,
}

impl BertAttention {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        Ok(Self {
            self_attention: BertSelfAttention::new(&(path / "self"), config)?,
            dense: linear(
                &(path / "output" / "dense"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
            layer_norm: layer_norm(
                &(path / "output" / "LayerNorm"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        key_value_states: Option<&Tensor>,
        past: Option<(&Tensor, &Tensor)>,
    ) -> tch::Result<(Tensor, Tensor, Tensor)> {
        let (context, key, value) =
            self.self_attention
                .forward(hidden_states, key_value_states, past)?;
        let output = self
            .layer_norm
            .forward(&(self.dense.forward(&context)?).f_add(hidden_states)?)?;
        Ok((output, key, value))
    }
}

#[derive(Debug)]
struct BertSelfAttention {
    query: nn::Linear,
    key: nn::Linear,
    value: nn::Linear,
    num_heads: i64,
    head_dim: i64,
}

impl BertSelfAttention {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        let head_dim = config.hidden_size / config.num_attention_heads;
        Ok(Self {
            query: linear(
                &(path / "query"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
            key: linear(
                &(path / "key"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
            value: linear(
                &(path / "value"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
            num_heads: config.num_attention_heads,
            head_dim,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        key_value_states: Option<&Tensor>,
        past: Option<(&Tensor, &Tensor)>,
    ) -> tch::Result<(Tensor, Tensor, Tensor)> {
        let query = self.transpose_for_scores(&self.query.forward(hidden_states)?)?;
        let (key, value) = match (key_value_states, past) {
            (Some(_), Some((key, value))) => (key.f_alias()?, value.f_alias()?),
            (states, past) => {
                let states = states.unwrap_or(hidden_states);
                let key = self.transpose_for_scores(&self.key.forward(states)?)?;
                let value = self.transpose_for_scores(&self.value.forward(states)?)?;
                if key_value_states.is_none()
                    && let Some((past_key, past_value)) = past
                {
                    (
                        Tensor::f_cat(&[past_key.f_alias()?, key], 2)?,
                        Tensor::f_cat(&[past_value.f_alias()?, value], 2)?,
                    )
                } else {
                    (key, value)
                }
            }
        };
        let probabilities = (query.f_matmul(&key.f_transpose(-1, -2)?)?)
            .f_div_scalar((self.head_dim as f64).sqrt())?
            .f_softmax(-1, Kind::Float)?
            .f_to_kind(value.kind())?;
        let size = hidden_states.size();
        let context = probabilities
            .f_matmul(&value)?
            .f_transpose(1, 2)?
            .f_contiguous()?
            .f_view([size[0], size[1], self.num_heads * self.head_dim])?;
        Ok((context, key, value))
    }

    fn transpose_for_scores(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let size = hidden_states.size();
        hidden_states
            .f_view([size[0], size[1], self.num_heads, self.head_dim])?
            .f_permute([0, 2, 1, 3])
    }
}

#[derive(Debug)]
struct BertOnlyMLMHead {
    transform: BertPredictionHeadTransform,
    decoder: nn::Linear,
    _bias: Tensor,
}

impl BertOnlyMLMHead {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        let predictions = path / "predictions";
        Ok(Self {
            transform: BertPredictionHeadTransform::new(
                &(predictions.clone() / "transform"),
                config,
            )?,
            decoder: linear(
                &(predictions.clone() / "decoder"),
                config.hidden_size,
                config.vocab_size,
                true,
            )?,
            // Transformers aliases this parameter to decoder.bias; both names are in this checkpoint.
            _bias: predictions.var("bias", &[config.vocab_size])?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        self.decoder
            .forward(&self.transform.forward(hidden_states)?)
    }
}

#[derive(Debug)]
struct BertPredictionHeadTransform {
    dense: nn::Linear,
    layer_norm: nn::LayerNorm,
}

impl BertPredictionHeadTransform {
    fn new(path: &nn::Path<'_>, config: &BertConfig) -> Result<Self> {
        Ok(Self {
            dense: linear(
                &(path / "dense"),
                config.hidden_size,
                config.hidden_size,
                true,
            )?,
            layer_norm: layer_norm(
                &(path / "LayerNorm"),
                config.hidden_size,
                config.layer_norm_eps,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        self.layer_norm
            .forward(&self.dense.forward(hidden_states)?.f_gelu("none")?)
    }
}

fn linear(path: &nn::Path<'_>, input: i64, output: i64, bias: bool) -> Result<nn::Linear> {
    Ok(nn::linear(path, input, output, nn::LinearConfig { bias })?)
}

fn layer_norm(path: &nn::Path<'_>, hidden_size: i64, epsilon: f64) -> Result<nn::LayerNorm> {
    Ok(nn::layer_norm(
        path,
        vec![hidden_size],
        nn::LayerNormConfig {
            eps: epsilon,
            ..Default::default()
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::banned_ngram_tokens;

    #[test]
    fn no_repeat_trigram_matches_transformers() {
        assert_eq!(banned_ngram_tokens(&[2, 10, 11, 10, 11], 3), [10]);
    }
}
