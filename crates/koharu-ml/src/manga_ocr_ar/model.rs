//! Inference-only manga-ocr-ar network.
//!
//! Upstream architecture and decoding:
//! https://huggingface.co/muscgab/manga-ocr-ar-experimental/blob/7873d8492307a021d6087b2c2b5f576a30b4acae/infer.py
//! Upstream decodes through `_GraphStep`, a CUDA graph of `Model.step`, where
//! it can; this port follows `greedy_cached`, which upstream says matches it
//! up to floating-point noise.
//!
//! Upstream runs F32 weights under `torch.autocast`, so linears and attention
//! compute in BF16 while residuals, norms, and RoPE stay F32. This port has no
//! autocast: `dense`, `attention`, and the depth fusion's product cast their
//! inputs to the kind of the store's weights, and every weight used outside
//! them is declared F32, which is the same arithmetic.

use std::path::Path;

use anyhow::Result;
use koharu_torch::{Device, Kind, Tensor};

use super::{
    config::{Config, DecoderConfig, EncoderConfig},
    processor::{Batch, Crop, PATCH},
};
use crate::nn::{self, Module};

/// Upstream `RMSNorm`'s default.
const RMS_EPS: f64 = 1e-6;
const ROPE_THETA: f32 = 10_000.0;
/// Upstream `Memory.fourier`'s octaves over both axes, sine and cosine.
const FOURIER_OCTAVES: i32 = 8;
const FOURIER_DIM: i64 = 4 * FOURIER_OCTAVES as i64;
const BRIDGE_DIM: i64 = 1024;
const NO_BIAS: nn::LinearConfig = nn::LinearConfig { bias: false };

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    encoder: Encoder,
    memory: Memory,
    blocks: Vec<ArBlock>,
    tok: Tensor,
    final_norm: RmsNorm,
    /// Outputs are the characters and the end token; the start token is input
    /// only.
    eos: i64,
    bos: i64,
    slots: i64,
    cos: Tensor,
    sin: Tensor,
    pos: Tensor,
    device: Device,
}

impl Model {
    pub(super) fn new(config: &Config, n_chars: i64, device: Device, kind: Kind) -> Result<Self> {
        let (encoder, decoder) = (&config.encoder, &config.decoder);
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();
        let slots = config.max_length + 1;
        let head_dim = decoder.dim / decoder.heads;
        let angles = Tensor::arange(slots, (Kind::Float, device))?
            .outer(&inverse_frequencies(head_dim, device)?)?;
        let (cos, sin) = (angles.cos()?, angles.sin()?);

        Ok(Self {
            encoder: Encoder::new(&(&root / "encoder"), encoder)?,
            memory: Memory::new(&(&root / "memory"), encoder, decoder)?,
            blocks: (0..decoder.layers)
                .map(|index| ArBlock::new(&(&root / "blocks" / index), decoder))
                .collect::<Result<Vec<_>>>()?,
            tok: (&root / "tok").var_with_kind(
                "weight",
                &[n_chars + 2, decoder.dim],
                Kind::Float,
            )?,
            final_norm: RmsNorm::new(&(&root / "final_norm"), decoder.dim)?,
            eos: n_chars,
            bos: n_chars + 1,
            slots,
            cos: Tensor::cat(&[&cos, &cos], -1)?.unsqueeze(0)?.unsqueeze(2)?,
            sin: Tensor::cat(&[&sin, &sin], -1)?.unsqueeze(0)?.unsqueeze(2)?,
            pos: Tensor::arange(slots, (Kind::Int64, device))?,
            device,
            vs,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    /// Greedy decoding of a batch, each row ending at its first end token or
    /// running to the last slot. Upstream `greedy_cached`.
    pub(super) fn generate(&self, crops: &[Crop]) -> Result<Vec<Vec<i64>>> {
        let batch = Batch::new(crops, self.device)?;
        let rows = crops.len() as i64;
        let states = self.encoder.forward(&batch)?;
        let (memories, key_mask) = self.memory.forward(&states, &batch)?;
        let contexts = self
            .blocks
            .iter()
            .zip(&memories)
            .map(|(block, memory)| block.context(memory, rows, self.slots, self.vs.kind()))
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        let output = self
            .tok
            .narrow(0, 0, self.eos + 1)?
            .to_kind(self.vs.kind())?;

        let mut token = Tensor::full([rows], self.bos, (Kind::Int64, self.device))?;
        let mut done = Tensor::zeros([rows], (Kind::Bool, self.device))?;
        let mut steps = Vec::new();
        for t in 0..self.slots {
            let next = self.step(&token, t, &contexts, &key_mask, &output)?;
            let next = Tensor::where_scalarself(&done, self.eos, &next)?;
            done = done.logical_or(&next.eq_scalar(self.eos)?)?;
            steps.push(next.alias()?);
            // Upstream checks every 8 steps, for fewer syncs from the device.
            if t % 8 == 7 && i64::try_from(done.all()?)? != 0 {
                break;
            }
            token = next;
        }

        let ids = Vec::<i64>::try_from(Tensor::stack(&steps, 1)?.reshape([-1])?)?;
        Ok(ids.chunks(steps.len()).map(<[i64]>::to_vec).collect())
    }

    /// The next token of each row from the one before it.
    fn step(
        &self,
        token: &Tensor,
        t: i64,
        contexts: &[Context],
        key_mask: &Tensor,
        output: &Tensor,
    ) -> koharu_torch::Result<Tensor> {
        let t = self.pos.narrow(0, t, 1)?;
        let position = Position {
            cos: self.cos.index_select(1, &t)?,
            sin: self.sin.index_select(1, &t)?,
            mask: self.pos.le_tensor(&t)?.view([1, 1, 1, -1])?,
            t,
        };
        let mut x = Tensor::embedding(&self.tok, token, -1, false, false)?.unsqueeze(1)?;
        for (block, context) in self.blocks.iter().zip(contexts) {
            x = block.step(&x, context, key_mask, &position)?;
        }
        let x = self.final_norm.forward(&x.squeeze_dim(1)?)?;
        Tensor::linear(&x.to_kind(output.kind())?, output, None::<&Tensor>)?.argmax(-1, false)
    }
}

/// A linear layer as autocast runs it: on an input of its weights' kind.
fn dense(layer: &nn::Linear, xs: &Tensor) -> koharu_torch::Result<Tensor> {
    layer.forward(&xs.to_kind(layer.ws.kind())?)
}

/// Attention as autocast runs it, where values are already in the store's
/// kind and rotated queries and keys aren't. `mask` marks what to attend to.
fn attention(
    query: &Tensor,
    key: &Tensor,
    value: &Tensor,
    mask: &Tensor,
) -> koharu_torch::Result<Tensor> {
    Tensor::scaled_dot_product_attention(
        &query.to_kind(value.kind())?,
        &key.to_kind(value.kind())?,
        value,
        Some(mask),
        0.0,
        false,
        None::<f64>,
        false,
    )
}

/// Upstream's `1.0 / (10000.0 ** (arange(0, dim, 2).float() / dim))`.
fn inverse_frequencies(dim: i64, device: Device) -> koharu_torch::Result<Tensor> {
    let frequencies = (0..dim)
        .step_by(2)
        .map(|index| ROPE_THETA.powf(index as f32 / dim as f32).recip())
        .collect::<Vec<_>>();
    Tensor::from_slice(&frequencies)?.to(device)
}

/// Upstream's `(x * cos + rotate_half(x) * sin)`, in F32 as upstream's is.
fn rotate(x: &Tensor, cos: &Tensor, sin: &Tensor) -> koharu_torch::Result<Tensor> {
    let x = x.to_kind(Kind::Float)?;
    let half = x.chunk(2, -1)?;
    let rotated = Tensor::cat(&[half[1].neg()?, half[0].alias()?], -1)?;
    x.mul_tensor(cos)?.add_tensor(&rotated.mul_tensor(sin)?, 1)
}

/// Upstream `RMSNorm`. Its weight is F32 and its output keeps the input's kind.
#[derive(Debug)]
struct RmsNorm {
    weight: Tensor,
    dim: i64,
}

impl RmsNorm {
    fn new(path: &nn::Path<'_>, dim: i64) -> Result<Self> {
        Ok(Self {
            weight: path.var_with_kind("weight", &[dim], Kind::Float)?,
            dim,
        })
    }

    fn forward(&self, x: &Tensor) -> koharu_torch::Result<Tensor> {
        Tensor::rms_norm(
            &x.to_kind(Kind::Float)?,
            [self.dim],
            Some(&self.weight),
            RMS_EPS,
        )?
        .to_kind(x.kind())
    }
}

/// Upstream `VitRMSNorm`: it normalizes in F32, casts back, and then scales by
/// its F32 weight, so its output is F32 whatever the input's kind.
#[derive(Debug)]
struct VitRmsNorm {
    weight: Tensor,
    eps: f64,
}

impl VitRmsNorm {
    fn new(path: &nn::Path<'_>, dim: i64, eps: f64) -> Result<Self> {
        Ok(Self {
            weight: path.var_with_kind("weight", &[dim], Kind::Float)?,
            eps,
        })
    }

    fn forward(&self, x: &Tensor) -> koharu_torch::Result<Tensor> {
        let y = x.to_kind(Kind::Float)?;
        let scale = y
            .pow_tensor_scalar(2.0)?
            .mean_dim(&[-1i64][..], true, None::<Kind>)?
            .add_scalar(self.eps, 1)?
            .rsqrt()?;
        y.mul_tensor(&scale)?
            .to_kind(x.kind())?
            .mul_tensor(&self.weight)
    }
}

/// Upstream `Encoder` and the `_Vit` it holds, its nesting kept in the
/// parameter paths alone.
#[derive(Debug)]
struct Encoder {
    /// `(dim, 3, PATCH, PATCH)`, a convolution's, applied to flattened patches.
    proj_weight: Tensor,
    proj_bias: Tensor,
    norm: VitRmsNorm,
    blocks: Vec<VitBlock>,
    post_trunk_norm: VitRmsNorm,
    inv_freq: Tensor,
}

impl Encoder {
    fn new(path: &nn::Path<'_>, config: &EncoderConfig) -> Result<Self> {
        let vit = path / "vit";
        let patchifier = &vit / "patch_embed" / "patchifier";
        let proj = &patchifier / "proj";
        Ok(Self {
            proj_weight: proj.var("weight", &[config.dim, 3, PATCH, PATCH])?,
            proj_bias: proj.var("bias", &[config.dim])?,
            norm: VitRmsNorm::new(&(&patchifier / "norm"), config.dim, config.rms_eps)?,
            blocks: (0..config.layers)
                .map(|index| VitBlock::new(&(&vit / "blocks" / index), config))
                .collect::<Result<Vec<_>>>()?,
            post_trunk_norm: VitRmsNorm::new(
                &(&vit / "post_trunk_norm"),
                config.dim,
                config.rms_eps,
            )?,
            inv_freq: inverse_frequencies(config.dim / config.heads / 2, path.device())?,
        })
    }

    /// RoPE for each patch, `(batch, patches, 1, head_dim)`, its row and column
    /// frequencies interleaved.
    fn rope(&self, batch: &Batch) -> koharu_torch::Result<(Tensor, Tensor)> {
        let size = batch.rows.size();
        let table = Tensor::arange(batch.side, (Kind::Float, self.inv_freq.device()))?
            .outer(&self.inv_freq)?;
        let frequencies = |positions: &Tensor| {
            table
                .index_select(0, &positions.reshape([-1])?)?
                .reshape([size[0], size[1], -1])
        };
        let emb = Tensor::stack(&[frequencies(&batch.rows)?, frequencies(&batch.cols)?], -1)?
            .flatten_using_ints(-2, -1)?;
        let (cos, sin) = (emb.cos()?, emb.sin()?);
        Ok((
            Tensor::cat(&[&cos, &cos], -1)?.unsqueeze(2)?,
            Tensor::cat(&[&sin, &sin], -1)?.unsqueeze(2)?,
        ))
    }

    /// The state before each block, then the normalized final state.
    fn forward(&self, batch: &Batch) -> koharu_torch::Result<Vec<Tensor>> {
        let weight = self.proj_weight.flatten_using_ints(1, -1)?;
        let mut x = self.norm.forward(&Tensor::linear(
            &batch.patches.to_kind(weight.kind())?,
            &weight,
            Some(&self.proj_bias),
        )?)?;
        let (cos, sin) = self.rope(batch)?;
        let mask = batch.valid.unsqueeze(1)?.unsqueeze(1)?;
        let mut states = vec![x.alias()?];
        for block in &self.blocks {
            x = block.forward(&x, &mask, &cos, &sin)?;
            states.push(x.alias()?);
        }
        states.pop();
        states.push(self.post_trunk_norm.forward(&x)?);
        Ok(states)
    }
}

/// Upstream `_VitBlock`, with its `_Attn` and `_MLP` kept in the parameter
/// paths alone.
#[derive(Debug)]
struct VitBlock {
    qkv: nn::Linear,
    proj: nn::Linear,
    fc1: nn::Linear,
    fc2: nn::Linear,
    fc3: nn::Linear,
    norm1: VitRmsNorm,
    norm2: VitRmsNorm,
    heads: i64,
    head_dim: i64,
}

impl VitBlock {
    fn new(path: &nn::Path<'_>, config: &EncoderConfig) -> Result<Self> {
        let (dim, ffn) = (config.dim, config.ffn);
        let (attn, mlp) = (path / "attn", path / "mlp");
        Ok(Self {
            qkv: nn::linear(&attn / "qkv", dim, 3 * dim, NO_BIAS)?,
            proj: nn::linear(&attn / "proj", dim, dim, NO_BIAS)?,
            fc1: nn::linear(&mlp / "fc1", dim, ffn, NO_BIAS)?,
            fc2: nn::linear(&mlp / "fc2", ffn, dim, NO_BIAS)?,
            fc3: nn::linear(&mlp / "fc3", dim, ffn, NO_BIAS)?,
            norm1: VitRmsNorm::new(&(path / "norm1"), dim, config.rms_eps)?,
            norm2: VitRmsNorm::new(&(path / "norm2"), dim, config.rms_eps)?,
            heads: config.heads,
            head_dim: dim / config.heads,
        })
    }

    fn forward(
        &self,
        x: &Tensor,
        mask: &Tensor,
        cos: &Tensor,
        sin: &Tensor,
    ) -> koharu_torch::Result<Tensor> {
        let size = x.size();
        let qkv = dense(&self.qkv, &self.norm1.forward(x)?)?
            .view([size[0], size[1], 3, self.heads, self.head_dim])?
            .unbind_int(2)?;
        let q = rotate(&qkv[0], cos, sin)?.transpose_int(1, 2)?;
        let k = rotate(&qkv[1], cos, sin)?.transpose_int(1, 2)?;
        let attended = attention(&q, &k, &qkv[2].transpose_int(1, 2)?, mask)?
            .transpose_int(1, 2)?
            .reshape(&size[..])?;
        let x = x.add_tensor(&dense(&self.proj, &attended)?, 1)?;
        let y = self.norm2.forward(&x)?;
        let gated = dense(&self.fc1, &y)?
            .silu()?
            .mul_tensor(&dense(&self.fc3, &y)?)?;
        x.add_tensor(&dense(&self.fc2, &gated)?, 1)
    }
}

/// Upstream `Memory`, its static depth fusion.
#[derive(Debug)]
struct Memory {
    layers: i64,
    bridge_norm: RmsNorm,
    bridge_up: nn::Linear,
    bridge_down: nn::Linear,
    bridge_out_norm: RmsNorm,
    pos: nn::Linear,
    null: Tensor,
    norms: Vec<RmsNorm>,
    wo: nn::Linear,
    gamma: Tensor,
    depth_logits: Tensor,
}

impl Memory {
    fn new(path: &nn::Path<'_>, encoder: &EncoderConfig, decoder: &DecoderConfig) -> Result<Self> {
        let (enc_dim, dim) = (encoder.dim, decoder.dim);
        let bridge = path / "bridge";
        Ok(Self {
            layers: decoder.layers,
            bridge_norm: RmsNorm::new(&(&bridge / 0), enc_dim)?,
            bridge_up: nn::linear(&bridge / 1, enc_dim, BRIDGE_DIM, NO_BIAS)?,
            bridge_down: nn::linear(&bridge / 3, BRIDGE_DIM, dim, NO_BIAS)?,
            bridge_out_norm: RmsNorm::new(&(&bridge / 4), dim)?,
            pos: nn::linear(path / "pos", FOURIER_DIM, dim, NO_BIAS)?,
            null: path.var_with_kind("null", &[2, dim], Kind::Float)?,
            norms: (0..encoder.layers)
                .map(|index| RmsNorm::new(&(path / "norms" / index), enc_dim))
                .collect::<Result<Vec<_>>>()?,
            wo: nn::linear(path / "wo", enc_dim, dim, NO_BIAS)?,
            gamma: path.var_with_kind("gamma", &[decoder.layers, dim], Kind::Float)?,
            depth_logits: path.var_with_kind(
                "depth_logits",
                &[decoder.layers, encoder.layers],
                Kind::Float,
            )?,
        })
    }

    /// One memory for each decoder block, `(batch, 2 + patches, dim)`, and
    /// which of its entries to attend to.
    fn forward(
        &self,
        states: &[Tensor],
        batch: &Batch,
    ) -> koharu_torch::Result<(Vec<Tensor>, Tensor)> {
        let (last, depths) = states.split_last().expect("the encoder has states");
        let rows = last.size()[0];

        let bridged = dense(&self.bridge_up, &self.bridge_norm.forward(last)?)?.gelu("none")?;
        let bridged = self
            .bridge_out_norm
            .forward(&dense(&self.bridge_down, &bridged)?)?;
        let base = bridged.add_tensor(&dense(&self.pos, &fourier(batch)?)?, 1)?;

        let normed = self
            .norms
            .iter()
            .zip(depths)
            .map(|(norm, state)| norm.forward(state))
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        // Autocast runs this product in the store's kind, as it does a linear.
        let kind = self.wo.ws.kind();
        let alpha = self.depth_logits.softmax_int(-1, Kind::Float)?;
        let fused = Tensor::einsum(
            "jl,lbnd->jbnd",
            &[
                alpha.to_kind(kind)?,
                Tensor::stack(&normed, 0)?.to_kind(kind)?,
            ],
            None::<&[i64]>,
        )?;
        let memories = base.unsqueeze(0)?.add_tensor(
            &self
                .gamma
                .unsqueeze(1)?
                .unsqueeze(1)?
                .mul_tensor(&dense(&self.wo, &fused)?)?,
            1,
        )?;
        let null = self.null.expand([self.layers, rows, -1, -1], false)?;
        let memories = Tensor::cat(&[&null, &memories], 2)?;
        let key_mask = Tensor::cat(
            &[
                &Tensor::ones([rows, 2], (Kind::Bool, batch.valid.device()))?,
                &batch.valid,
            ],
            1,
        )?;
        Ok((memories.unbind_int(0)?, key_mask))
    }
}

/// Upstream `Memory.fourier`: each patch's position in its own grid, `(batch,
/// patches, FOURIER_DIM)`.
fn fourier(batch: &Batch) -> koharu_torch::Result<Tensor> {
    let device = batch.rows.device();
    let unit = |positions: &Tensor, sizes: &Tensor| {
        positions
            .to_kind(Kind::Float)?
            .add_scalar(0.5, 1)?
            .div_tensor(&sizes.unsqueeze(1)?.to_kind(Kind::Float)?)?
            .unsqueeze(-1)
    };
    let frequencies = Tensor::from_slice(
        &(0..FOURIER_OCTAVES)
            .map(|octave| std::f32::consts::PI * 2f32.powi(octave))
            .collect::<Vec<_>>(),
    )?
    .to(device)?;
    let rows = unit(&batch.rows, &batch.heights)?.mul_tensor(&frequencies)?;
    let cols = unit(&batch.cols, &batch.widths)?.mul_tensor(&frequencies)?;
    Tensor::cat(&[rows.sin()?, rows.cos()?, cols.sin()?, cols.cos()?], -1)
}

/// Upstream `ARBlock`.
#[derive(Debug)]
struct ArBlock {
    self_norm: RmsNorm,
    qkv: nn::Linear,
    self_proj: nn::Linear,
    cross_norm: RmsNorm,
    q: nn::Linear,
    kv: nn::Linear,
    cross_proj: nn::Linear,
    ffn_norm: RmsNorm,
    gate_up: nn::Linear,
    down: nn::Linear,
    heads: i64,
    head_dim: i64,
}

/// What a block holds for a batch while decoding it: the memory's keys and
/// values, `(batch, heads, entries, head_dim)`, and upstream `new_cache`'s
/// self-attention cache, written at each position.
#[derive(Debug)]
struct Context {
    memory_keys: Tensor,
    memory_values: Tensor,
    keys: Tensor,
    values: Tensor,
}

/// One position of the decoder, `t` holding its index.
#[derive(Debug)]
struct Position {
    t: Tensor,
    cos: Tensor,
    sin: Tensor,
    mask: Tensor,
}

impl ArBlock {
    fn new(path: &nn::Path<'_>, config: &DecoderConfig) -> Result<Self> {
        let (dim, ffn) = (config.dim, config.ffn);
        Ok(Self {
            self_norm: RmsNorm::new(&(path / "self_norm"), dim)?,
            qkv: nn::linear(path / "qkv", dim, 3 * dim, NO_BIAS)?,
            self_proj: nn::linear(path / "self_proj", dim, dim, NO_BIAS)?,
            cross_norm: RmsNorm::new(&(path / "cross_norm"), dim)?,
            q: nn::linear(path / "q", dim, dim, NO_BIAS)?,
            kv: nn::linear(path / "kv", dim, 2 * dim, NO_BIAS)?,
            cross_proj: nn::linear(path / "cross_proj", dim, dim, NO_BIAS)?,
            ffn_norm: RmsNorm::new(&(path / "ffn_norm"), dim)?,
            gate_up: nn::linear(path / "gate_up", dim, 2 * ffn, NO_BIAS)?,
            down: nn::linear(path / "down", ffn, dim, NO_BIAS)?,
            heads: config.heads,
            head_dim: dim / config.heads,
        })
    }

    fn context(
        &self,
        memory: &Tensor,
        rows: i64,
        slots: i64,
        kind: Kind,
    ) -> koharu_torch::Result<Context> {
        let size = memory.size();
        let memory_kv = dense(&self.kv, memory)?
            .view([size[0], size[1], 2, self.heads, self.head_dim])?
            .unbind_int(2)?;
        let cache = || {
            Tensor::zeros(
                [rows, self.heads, slots, self.head_dim],
                (kind, memory.device()),
            )
        };
        Ok(Context {
            memory_keys: memory_kv[0].transpose_int(1, 2)?,
            memory_values: memory_kv[1].transpose_int(1, 2)?,
            keys: cache()?,
            values: cache()?,
        })
    }

    /// One new token per row. Upstream `ARBlock.step`.
    fn step(
        &self,
        x: &Tensor,
        context: &Context,
        key_mask: &Tensor,
        position: &Position,
    ) -> koharu_torch::Result<Tensor> {
        let size = x.size();
        let (rows, dim) = (size[0], size[2]);
        let qkv = dense(&self.qkv, &self.self_norm.forward(x)?)?
            .view([rows, 1, 3, self.heads, self.head_dim])?
            .unbind_int(2)?;
        let q = rotate(&qkv[0], &position.cos, &position.sin)?.transpose_int(1, 2)?;
        let k = rotate(&qkv[1], &position.cos, &position.sin)?.transpose_int(1, 2)?;
        context
            .keys
            .index_copy_(2, &position.t, &k.to_kind(context.keys.kind())?)?;
        context
            .values
            .index_copy_(2, &position.t, &qkv[2].transpose_int(1, 2)?)?;
        let attended = attention(&q, &context.keys, &context.values, &position.mask)?
            .transpose_int(1, 2)?
            .reshape([rows, 1, dim])?;
        let x = x.add_tensor(&dense(&self.self_proj, &attended)?, 1)?;

        let query = dense(&self.q, &self.cross_norm.forward(&x)?)?
            .view([rows, 1, self.heads, self.head_dim])?
            .transpose_int(1, 2)?;
        let attended = attention(
            &query,
            &context.memory_keys,
            &context.memory_values,
            &key_mask.unsqueeze(1)?.unsqueeze(1)?,
        )?
        .transpose_int(1, 2)?
        .reshape([rows, 1, dim])?;
        let x = x.add_tensor(&dense(&self.cross_proj, &attended)?, 1)?;

        let gate_up = dense(&self.gate_up, &self.ffn_norm.forward(&x)?)?.chunk(2, -1)?;
        let gated = gate_up[0].silu()?.mul_tensor(&gate_up[1])?;
        x.add_tensor(&dense(&self.down, &gated)?, 1)
    }
}
