//! PP-OCRv6 medium text recognizer using Transformers' PP-OCRv6 small-rec model class.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/pp_ocrv6_small_rec/modeling_pp_ocrv6_small_rec.py

use std::path::Path;

use crate::nn::{self, Module, ModuleT};
use anyhow::Result;
use tch::{Kind, Tensor};

use super::super::pp_lcnet_v4::PPLCNetV4Backbone;
use super::config::PPOCRV6MediumRecConfig;

#[derive(Debug)]
pub(crate) struct Model {
    vs: nn::VarStore,
    backbone: PPLCNetV4Backbone,
    head: Head,
}

impl Model {
    pub(crate) fn new(
        config: &PPOCRV6MediumRecConfig,
        device: tch::Device,
        kind: Kind,
    ) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let backbone = PPLCNetV4Backbone::new(
            &(&vs.root() / "model" / "backbone"),
            &config.backbone_config,
        )?;
        let head = Head::new(&(&vs.root() / "head"), config)?;
        Ok(Self { vs, backbone, head })
    }

    pub(crate) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub(crate) fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        let feature_maps = self
            .backbone
            .forward(&pixel_values.f_to_kind(self.vs.kind())?)?;
        let hidden_states = feature_maps.last().expect("PP-LCNetV4 stage4 feature");
        let hidden_states =
            hidden_states.f_avg_pool2d([3, 2], [3, 2], [0, 0], false, true, None::<i64>)?;
        self.head.forward(&hidden_states)
    }
}

fn activate(hidden_states: Tensor, activation: &str) -> tch::Result<Tensor> {
    match activation {
        "silu" => hidden_states.f_silu(),
        "relu" => hidden_states.f_relu(),
        "gelu" => hidden_states.f_gelu("none"),
        name => panic!("unsupported PP-OCRv6 recognition activation {name}"),
    }
}

#[derive(Debug)]
struct ConvLayer {
    convolution: nn::Conv2D,
    normalization: nn::BatchNorm,
    activation: String,
}

impl ConvLayer {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: [i64; 2],
        groups: i64,
        activation: &str,
    ) -> Result<Self> {
        Ok(Self {
            convolution: nn::conv(
                path / "convolution",
                in_channels,
                out_channels,
                kernel_size,
                nn::ConvConfigND {
                    stride: [1, 1],
                    padding: [kernel_size[0] / 2, kernel_size[1] / 2],
                    groups,
                    bias: false,
                    ..Default::default()
                },
            )?,
            normalization: nn::batch_norm2d(
                path / "normalization",
                out_channels,
                Default::default(),
            )?,
            activation: activation.into(),
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = self.convolution.forward(hidden_states)?;
        let hidden_states = self.normalization.forward_t(&hidden_states, false)?;
        activate(hidden_states, &self.activation)
    }
}

#[derive(Debug)]
struct Attention {
    qkv: nn::Linear,
    projection: nn::Linear,
    num_heads: i64,
    head_dim: i64,
    scale: f64,
}

impl Attention {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumRecConfig) -> Result<Self> {
        let head_dim = config.hidden_size / config.num_attention_heads;
        assert_eq!(head_dim * config.num_attention_heads, config.hidden_size);
        Ok(Self {
            qkv: nn::linear(
                path / "qkv",
                config.hidden_size,
                config.hidden_size * 3,
                nn::LinearConfig {
                    bias: config.qkv_bias,
                },
            )?,
            projection: nn::linear(
                path / "projection",
                config.hidden_size,
                config.hidden_size,
                Default::default(),
            )?,
            num_heads: config.num_attention_heads,
            head_dim,
            scale: (head_dim as f64).powf(-0.5),
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let size = hidden_states.size();
        let (batch_size, sequence_length, embed_dim) = (size[0], size[1], size[2]);
        let mixed_qkv = self
            .qkv
            .forward(hidden_states)?
            .f_view([
                batch_size,
                sequence_length,
                3,
                self.num_heads,
                self.head_dim,
            ])?
            .f_permute([2, 0, 3, 1, 4])?;
        let query = mixed_qkv.f_get(0)?;
        let key = mixed_qkv.f_get(1)?;
        let value = mixed_qkv.f_get(2)?;
        let weights = (query.f_matmul(&key.f_transpose(-1, -2)?)?)
            .f_mul_scalar(self.scale)?
            .f_softmax(-1, None::<Kind>)?;
        let hidden_states = weights
            .f_matmul(&value)?
            .f_transpose(1, 2)?
            .f_contiguous()?
            .f_view([batch_size, sequence_length, embed_dim])?;
        self.projection.forward(&hidden_states)
    }
}

#[derive(Debug)]
struct Mlp {
    fc1: nn::Linear,
    fc2: nn::Linear,
    activation: String,
}

impl Mlp {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumRecConfig) -> Result<Self> {
        let hidden_features = (config.hidden_size as f64 * config.mlp_ratio) as i64;
        Ok(Self {
            fc1: nn::linear(
                path / "fc1",
                config.hidden_size,
                hidden_features,
                Default::default(),
            )?,
            fc2: nn::linear(
                path / "fc2",
                hidden_features,
                config.hidden_size,
                Default::default(),
            )?,
            activation: config.hidden_act.clone(),
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = activate(self.fc1.forward(hidden_states)?, &self.activation)?;
        self.fc2.forward(&hidden_states)
    }
}

#[derive(Debug)]
struct Block {
    self_attn: Attention,
    layer_norm1: nn::LayerNorm,
    mlp: Mlp,
    layer_norm2: nn::LayerNorm,
}

impl Block {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumRecConfig) -> Result<Self> {
        let norm_config = nn::LayerNormConfig {
            eps: config.layer_norm_eps,
            ..Default::default()
        };
        Ok(Self {
            self_attn: Attention::new(&(path / "self_attn"), config)?,
            layer_norm1: nn::layer_norm(
                path / "layer_norm1",
                vec![config.hidden_size],
                norm_config,
            )?,
            mlp: Mlp::new(&(path / "mlp"), config)?,
            layer_norm2: nn::layer_norm(
                path / "layer_norm2",
                vec![config.hidden_size],
                norm_config,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = hidden_states.f_add(
            &self
                .self_attn
                .forward(&self.layer_norm1.forward(hidden_states)?)?,
        )?;
        hidden_states.f_add(
            &self
                .mlp
                .forward(&self.layer_norm2.forward(&hidden_states)?)?,
        )
    }
}

#[derive(Debug)]
struct EncoderWithSvtr {
    conv_block: [ConvLayer; 3],
    svtr_block: Vec<Block>,
    norm: nn::LayerNorm,
}

impl EncoderWithSvtr {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumRecConfig) -> Result<Self> {
        let in_channels = config
            .backbone_config
            .block_configs
            .last()
            .and_then(|stage| stage.last())
            .expect("PP-LCNetV4 final block")
            .out_channels;
        let hidden_size = config.hidden_size;
        let kernel_size = [config.conv_kernel_size[0], config.conv_kernel_size[1]];
        Ok(Self {
            conv_block: [
                ConvLayer::new(
                    &(path / "conv_block" / 0),
                    in_channels,
                    hidden_size,
                    [1, 1],
                    1,
                    &config.hidden_act,
                )?,
                ConvLayer::new(
                    &(path / "conv_block" / 1),
                    in_channels,
                    hidden_size,
                    [1, 1],
                    1,
                    &config.hidden_act,
                )?,
                ConvLayer::new(
                    &(path / "conv_block" / 2),
                    hidden_size,
                    hidden_size,
                    kernel_size,
                    hidden_size,
                    &config.hidden_act,
                )?,
            ],
            svtr_block: (0..config.depth)
                .map(|index| Block::new(&(path / "svtr_block" / index), config))
                .collect::<Result<_>>()?,
            norm: nn::layer_norm(
                path / "norm",
                vec![hidden_size],
                nn::LayerNormConfig {
                    eps: config.layer_norm_eps,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let residual = self.conv_block[0].forward(hidden_states)?;
        let mut hidden_states = self.conv_block[1].forward(hidden_states)?;
        hidden_states = hidden_states.f_add(&self.conv_block[2].forward(&hidden_states)?)?;

        let size = hidden_states.size();
        let (batch_size, channels, height, width) = (size[0], size[1], size[2], size[3]);
        hidden_states = hidden_states.f_flatten(2, -1)?.f_transpose(1, 2)?;
        for block in &self.svtr_block {
            hidden_states = block.forward(&hidden_states)?;
        }
        hidden_states = self.norm.forward(&hidden_states)?;
        hidden_states = hidden_states
            .f_view([batch_size, height, width, channels])?
            .f_permute([0, 3, 1, 2])?;
        hidden_states
            .f_add(&residual)?
            .f_squeeze_dim(2)?
            .f_transpose(1, 2)
    }
}

#[derive(Debug)]
struct Head {
    encoder: EncoderWithSvtr,
    head: nn::Linear,
}

impl Head {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumRecConfig) -> Result<Self> {
        Ok(Self {
            encoder: EncoderWithSvtr::new(&(path / "encoder"), config)?,
            head: nn::linear(
                path / "head",
                config.hidden_size,
                config.head_out_channels,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        self.head
            .forward(&self.encoder.forward(hidden_states)?)?
            .f_softmax(2, Some(Kind::Float))
    }
}
