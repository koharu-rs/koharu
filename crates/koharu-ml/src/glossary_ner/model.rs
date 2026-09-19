//! Torch network for the pinned GLiNER checkpoint.
//!
//! The GLiNER head mirrors `gliner/model.py`, `modules/layers.py`, and
//! `modules/span_rep.py` from urchade/GLiNER commit
//! `96350f0bafcc9ccf8a78ecab393c21634d9a58ce`. The encoder mirrors
//! `modeling_deberta_v2.py` from huggingface/transformers v4.35.2 commit
//! `514de24abfd4416aeba6a6455ad5920f57f3567d`.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use koharu_torch::nn::{Module, RNN};
use koharu_torch::{Device, Kind, Tensor, nn};

const ENCODER_WIDTH: i64 = 768;
const GLOSSARY_WIDTH: i64 = 512;
const INTERMEDIATE_WIDTH: i64 = 3072;
const HEADS: i64 = 12;
const HEAD_WIDTH: i64 = ENCODER_WIDTH / HEADS;
const LAYERS: usize = 12;
const VOCAB_SIZE: i64 = 250_105;
const RELATIVE_BUCKETS: i64 = 256;
const MAX_RELATIVE_POSITION: i64 = 512;
pub(super) const MAX_SPAN_WIDTH: usize = 12;
pub(super) const MAX_ENCODER_TOKENS: usize = 512;
pub(super) const EXPECTED_WEIGHT_COUNT: usize = 224;

#[derive(Debug)]
struct Projection {
    input: nn::Linear,
    output: nn::Linear,
}

impl Projection {
    fn new(path: &nn::Path<'_>, input_width: i64, output_width: i64) -> Self {
        Self {
            input: nn::linear(
                path / "0",
                input_width,
                output_width * 4,
                Default::default(),
            ),
            output: nn::linear(
                path / "3",
                output_width * 4,
                output_width,
                Default::default(),
            ),
        }
    }

    fn forward(&self, input: &Tensor) -> Tensor {
        // Dropout at index 2 is deliberately omitted for inference.
        self.output.forward(&self.input.forward(input).relu())
    }
}

#[derive(Debug)]
struct DebertaLayer {
    query: nn::Linear,
    key: nn::Linear,
    value: nn::Linear,
    attention_dense: nn::Linear,
    attention_norm: nn::LayerNorm,
    intermediate: nn::Linear,
    output_dense: nn::Linear,
    output_norm: nn::LayerNorm,
}

impl DebertaLayer {
    fn new(path: &nn::Path<'_>) -> Self {
        let attention = path / "attention";
        let self_attention = &attention / "self";
        let output = &attention / "output";
        let norm = nn::LayerNormConfig {
            eps: 1e-7,
            ..Default::default()
        };
        Self {
            query: nn::linear(
                &self_attention / "query_proj",
                ENCODER_WIDTH,
                ENCODER_WIDTH,
                Default::default(),
            ),
            key: nn::linear(
                &self_attention / "key_proj",
                ENCODER_WIDTH,
                ENCODER_WIDTH,
                Default::default(),
            ),
            value: nn::linear(
                &self_attention / "value_proj",
                ENCODER_WIDTH,
                ENCODER_WIDTH,
                Default::default(),
            ),
            attention_dense: nn::linear(
                &output / "dense",
                ENCODER_WIDTH,
                ENCODER_WIDTH,
                Default::default(),
            ),
            attention_norm: nn::layer_norm(&output / "LayerNorm", vec![ENCODER_WIDTH], norm),
            intermediate: nn::linear(
                path / "intermediate" / "dense",
                ENCODER_WIDTH,
                INTERMEDIATE_WIDTH,
                Default::default(),
            ),
            output_dense: nn::linear(
                path / "output" / "dense",
                INTERMEDIATE_WIDTH,
                ENCODER_WIDTH,
                Default::default(),
            ),
            output_norm: nn::layer_norm(path / "output" / "LayerNorm", vec![ENCODER_WIDTH], norm),
        }
    }

    // Structurally follows `DisentangledSelfAttention` in Transformers'
    // mDeBERTa-v2 implementation. The checkpoint uses shared attention keys,
    // so query/key projections also project the relative embeddings.
    fn forward(
        &self,
        hidden: &Tensor,
        relative_positions: &Tensor,
        relative_embeddings: &Tensor,
    ) -> Tensor {
        let [batch, length, _] = <[i64; 3]>::try_from(hidden.size()).unwrap();
        let to_heads = |tensor: Tensor| {
            tensor
                .view([batch, length, HEADS, HEAD_WIDTH])
                .permute([0, 2, 1, 3])
                .contiguous()
                .view([batch * HEADS, length, HEAD_WIDTH])
        };
        let query = to_heads(self.query.forward(hidden));
        let key = to_heads(self.key.forward(hidden));
        let value = to_heads(self.value.forward(hidden));

        let scale = (HEAD_WIDTH as f64 * 3.0).sqrt();
        let mut scores = query.bmm(&(key.transpose(-1, -2) / scale));
        let relative_length = relative_embeddings.size()[0];
        let to_relative_heads = |tensor: Tensor| {
            tensor
                .view([1, relative_length, HEADS, HEAD_WIDTH])
                .permute([0, 2, 1, 3])
                .contiguous()
                .view([HEADS, relative_length, HEAD_WIDTH])
                .repeat([batch, 1, 1])
        };
        let position_query = to_relative_heads(self.query.forward(relative_embeddings));
        let position_key = to_relative_heads(self.key.forward(relative_embeddings));

        let content_to_position = query.bmm(&position_key.transpose(-1, -2));
        let c2p_indices = (relative_positions + RELATIVE_BUCKETS)
            .clamp(0, RELATIVE_BUCKETS * 2 - 1)
            .expand([batch * HEADS, length, length], false);
        scores += content_to_position.gather(-1, &c2p_indices, false) / scale;

        let position_to_content = key.bmm(&position_query.transpose(-1, -2));
        let p2c_indices = (-relative_positions + RELATIVE_BUCKETS)
            .clamp(0, RELATIVE_BUCKETS * 2 - 1)
            .expand([batch * HEADS, length, length], false);
        scores += position_to_content
            .gather(-1, &p2c_indices, false)
            .transpose(-1, -2)
            / scale;

        // Windows are inferred independently without padding, so the upstream
        // attention mask is all true and can be omitted here.
        let context = scores
            .view([batch, HEADS, length, length])
            .softmax(-1, Some(Kind::Float))
            .view([batch * HEADS, length, length])
            .bmm(&value)
            .view([batch, HEADS, length, HEAD_WIDTH])
            .permute([0, 2, 1, 3])
            .contiguous()
            .view([batch, length, ENCODER_WIDTH]);
        let attention = self
            .attention_norm
            .forward(&(self.attention_dense.forward(&context) + hidden));
        self.output_norm.forward(
            &(self
                .output_dense
                .forward(&self.intermediate.forward(&attention).gelu("none"))
                + attention),
        )
    }
}

#[derive(Debug)]
pub(super) struct Model {
    var_store: nn::VarStore,
    word_embeddings: nn::Embedding,
    embedding_norm: nn::LayerNorm,
    relative_embeddings: nn::Embedding,
    relative_norm: nn::LayerNorm,
    encoder_layers: Vec<DebertaLayer>,
    token_projection: nn::Linear,
    rnn: nn::LSTM,
    prompt_projection: Projection,
    span_start_projection: Projection,
    span_end_projection: Projection,
    span_output_projection: Projection,
    relative_positions: Tensor,
    relative_ids: Tensor,
    device: Device,
}

impl Model {
    pub(super) fn new(device: Device) -> Result<Self> {
        let var_store = nn::VarStore::new(device);
        let root = var_store.root();
        let encoder = &root / "token_rep_layer" / "bert_layer" / "model";
        let norm = nn::LayerNormConfig {
            eps: 1e-7,
            ..Default::default()
        };
        let word_embeddings = nn::embedding(
            &encoder / "embeddings" / "word_embeddings",
            VOCAB_SIZE,
            ENCODER_WIDTH,
            nn::EmbeddingConfig {
                padding_idx: 0,
                ..Default::default()
            },
        );
        let embedding_norm = nn::layer_norm(
            &encoder / "embeddings" / "LayerNorm",
            vec![ENCODER_WIDTH],
            norm,
        );
        let relative_embeddings = nn::embedding(
            &encoder / "encoder" / "rel_embeddings",
            RELATIVE_BUCKETS * 2,
            ENCODER_WIDTH,
            Default::default(),
        );
        let relative_norm = nn::layer_norm(
            &encoder / "encoder" / "LayerNorm",
            vec![ENCODER_WIDTH],
            norm,
        );
        let encoder_layers = (0..LAYERS)
            .map(|index| DebertaLayer::new(&(&encoder / "encoder" / "layer" / index.to_string())))
            .collect();
        let token_projection = nn::linear(
            &root / "token_rep_layer" / "projection",
            ENCODER_WIDTH,
            GLOSSARY_WIDTH,
            Default::default(),
        );
        let rnn = nn::lstm(
            &root / "rnn" / "lstm",
            GLOSSARY_WIDTH,
            GLOSSARY_WIDTH / 2,
            nn::RNNConfig {
                train: false,
                bidirectional: true,
                ..Default::default()
            },
        );
        let prompt_projection = Projection::new(
            &(&root / "prompt_rep_layer"),
            GLOSSARY_WIDTH,
            GLOSSARY_WIDTH,
        );
        let span = &root / "span_rep_layer" / "span_rep_layer";
        let span_start_projection =
            Projection::new(&(&span / "project_start"), GLOSSARY_WIDTH, GLOSSARY_WIDTH);
        let span_end_projection =
            Projection::new(&(&span / "project_end"), GLOSSARY_WIDTH, GLOSSARY_WIDTH);
        let span_output_projection =
            Projection::new(&(&span / "out_project"), GLOSSARY_WIDTH * 2, GLOSSARY_WIDTH);
        let relative_positions = relative_position_tensor(MAX_ENCODER_TOKENS as i64, device);
        let relative_ids = Tensor::arange(RELATIVE_BUCKETS * 2, (Kind::Int64, device));
        ensure!(
            var_store.len() == EXPECTED_WEIGHT_COUNT,
            "GLiNER architecture registered {} weights; expected {EXPECTED_WEIGHT_COUNT}",
            var_store.len()
        );
        Ok(Self {
            var_store,
            word_embeddings,
            embedding_norm,
            relative_embeddings,
            relative_norm,
            encoder_layers,
            token_projection,
            rnn,
            prompt_projection,
            span_start_projection,
            span_end_projection,
            span_output_projection,
            relative_positions,
            relative_ids,
            device,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.var_store
            .load(path.as_ref())
            .context("failed to load GLiNER safetensors")?;
        self.var_store.freeze();
        Ok(())
    }

    pub(super) fn score(
        &self,
        input_ids: &[i64],
        first_subtokens: &[usize],
        prompt_words: usize,
        text_words: usize,
    ) -> Result<Vec<f32>> {
        ensure!(
            input_ids.len() <= MAX_ENCODER_TOKENS,
            "GLiNER encoder input has {} subtokens; maximum is {MAX_ENCODER_TOKENS}",
            input_ids.len()
        );
        ensure!(
            first_subtokens.len() == prompt_words + text_words,
            "GLiNER word/subtoken alignment is incomplete"
        );
        let input_ids = Tensor::from_slice(input_ids)
            .view([1, input_ids.len() as i64])
            .to_device(self.device);
        let mut hidden = self
            .embedding_norm
            .forward(&self.word_embeddings.forward(&input_ids));
        let length = input_ids.size()[1];
        let relative_positions = self
            .relative_positions
            .narrow(1, 0, length)
            .narrow(2, 0, length);
        let relative_embeddings = self
            .relative_norm
            .forward(&self.relative_embeddings.forward(&self.relative_ids));
        for layer in &self.encoder_layers {
            hidden = layer.forward(&hidden, &relative_positions, &relative_embeddings);
        }

        let first_subtokens = first_subtokens
            .iter()
            .map(|&index| i64::try_from(index))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let first_subtokens = Tensor::from_slice(&first_subtokens).to_device(self.device);
        let words = self
            .token_projection
            .forward(&hidden.index_select(1, &first_subtokens));
        let prompt = words.narrow(1, 0, prompt_words as i64);
        let text = words.narrow(1, prompt_words as i64, text_words as i64);
        let entity_indices =
            Tensor::arange_start_step(0, prompt_words as i64 - 1, 2, (Kind::Int64, self.device));
        let entities = self
            .prompt_projection
            .forward(&prompt.index_select(1, &entity_indices));
        // Upstream packs padded batches before the LSTM. Local windows are
        // unpadded single examples, making a direct sequence call equivalent.
        let (text, _) = self.rnn.seq(&text);
        let starts = self.span_start_projection.forward(&text);
        let ends = self.span_end_projection.forward(&text);

        // Upstream materializes [B,L,max_width,D]. Processing width slices
        // separately yields the same scores while avoiding padded span tensors.
        let mut scores = Vec::with_capacity(MAX_SPAN_WIDTH);
        for width in 0..MAX_SPAN_WIDTH.min(text_words) {
            let valid = text_words - width;
            let span = Tensor::cat(
                &[
                    starts.narrow(1, 0, valid as i64),
                    ends.narrow(1, width as i64, valid as i64),
                ],
                -1,
            )
            .relu();
            let span = self.span_output_projection.forward(&span);
            scores.push(span.matmul(&entities.transpose(-1, -2)).flatten(1, -1));
        }
        let probabilities = Tensor::cat(&scores, 1)
            .sigmoid()
            .view([-1])
            .to_device(Device::Cpu)
            .to_kind(Kind::Float);
        Ok(Vec::<f32>::try_from(&probabilities)?)
    }
}

fn relative_position_tensor(length: i64, device: Device) -> Tensor {
    let mut positions = Vec::with_capacity((length * length) as usize);
    for query in 0..length {
        for key in 0..length {
            positions.push(relative_position_bucket(query - key));
        }
    }
    Tensor::from_slice(&positions)
        .view([1, length, length])
        .to_device(device)
}

// Ported from Hugging Face Transformers' mDeBERTa-v2 implementation used by
// microsoft/mdeberta-v3-base. The scalar form builds one table per window.
fn relative_position_bucket(position: i64) -> i64 {
    let sign = position.signum();
    let mid = RELATIVE_BUCKETS / 2;
    let absolute = position.abs();
    if absolute <= mid {
        position
    } else {
        let logarithmic = ((absolute as f64 / mid as f64).ln()
            / ((MAX_RELATIVE_POSITION - 1) as f64 / mid as f64).ln()
            * (mid - 1) as f64)
            .ceil() as i64
            + mid;
        logarithmic * sign
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_positions_match_mdeberta_log_buckets() {
        assert_eq!(relative_position_bucket(0), 0);
        assert_eq!(relative_position_bucket(127), 127);
        assert_eq!(relative_position_bucket(128), 128);
        assert_eq!(relative_position_bucket(-128), -128);
        assert!(relative_position_bucket(511) < 256);
        assert_eq!(relative_position_bucket(511), 255);
        assert_eq!(relative_position_bucket(-511), -255);
    }
}
