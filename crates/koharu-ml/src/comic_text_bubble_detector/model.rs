//! Inference-only port of Transformers' RT-DETR-v2 object detector.
//!
//! Original implementations:
//! - https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py
//! - https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr/modeling_rt_detr_resnet.py

use crate::nn::Apply;
use std::path::Path;

use crate::nn::{self, Module, ModuleT};
use anyhow::Result;
use tch::{Device, IndexOp, Kind, Tensor};

use super::config::{RTDetrResNetConfig, RTDetrV2Config};

#[derive(Debug)]
pub(super) struct RTDetrV2ObjectDetectionOutput {
    pub logits: Tensor,
    pub pred_boxes: Tensor,
}

#[derive(Debug)]
pub struct Model {
    vs: nn::VarStore,
    model: RTDetrV2ForObjectDetection,
}

impl Model {
    pub fn new(config: RTDetrV2Config, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let model = RTDetrV2ForObjectDetection::new(&vs.root(), &config)?;
        Ok(Self { vs, model })
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub fn forward(&self, pixel_values: &Tensor) -> tch::Result<RTDetrV2ObjectDetectionOutput> {
        self.model.forward(&pixel_values.f_to_kind(self.vs.kind())?)
    }
}

#[derive(Debug)]
struct RTDetrV2ForObjectDetection {
    model: RTDetrV2Model,
    class_embed: Vec<nn::Linear>,
    bbox_embed: Vec<RTDetrV2MLPPredictionHead>,
}

impl RTDetrV2ForObjectDetection {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        Ok(Self {
            model: RTDetrV2Model::new(&(path / "model"), config)?,
            // The checkpoint serializes Transformers' tied top-level detection
            // heads under their canonical `model.decoder` paths.
            class_embed: (0..config.decoder_layers)
                .map(|index| {
                    nn::linear(
                        path / "model" / "decoder" / "class_embed" / index,
                        config.d_model,
                        config.num_labels(),
                        Default::default(),
                    )
                })
                .collect::<tch::Result<Vec<_>>>()?,
            bbox_embed: (0..config.decoder_layers)
                .map(|index| {
                    RTDetrV2MLPPredictionHead::new(
                        &(path / "model" / "decoder" / "bbox_embed" / index),
                        config.d_model,
                        config.d_model,
                        4,
                        3,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<RTDetrV2ObjectDetectionOutput> {
        self.model
            .forward(pixel_values, &self.bbox_embed, &self.class_embed)
    }
}

#[derive(Debug)]
struct RTDetrV2Model {
    config: RTDetrV2Config,
    anchors: Option<Tensor>,
    valid_mask: Option<Tensor>,
    backbone: RTDetrV2ConvEncoder,
    encoder_input_proj: Vec<nn::SequentialT>,
    encoder: RTDetrV2HybridEncoder,
    #[allow(dead_code)]
    denoising_class_embed: Option<nn::Embedding>,
    weight_embedding: Option<nn::Embedding>,
    enc_output: nn::Sequential,
    enc_score_head: nn::Linear,
    enc_bbox_head: RTDetrV2MLPPredictionHead,
    decoder_input_proj: Vec<nn::SequentialT>,
    decoder: RTDetrV2Decoder,
}

impl RTDetrV2Model {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        let (anchors, valid_mask) = config
            .anchor_image_size
            .as_deref()
            .and_then(|image_size| {
                fixed_spatial_shapes(image_size, &config.feat_strides, config.num_feature_levels)
            })
            .map(|spatial_shapes| generate_anchors(&spatial_shapes, path.device(), path.kind()))
            .transpose()?
            .map_or((None, None), |(anchors, valid_mask)| {
                (Some(anchors), Some(valid_mask))
            });
        let backbone = RTDetrV2ConvEncoder::new(&(path / "backbone"), config)?;
        let intermediate_channel_sizes = backbone.intermediate_channel_sizes();

        let encoder_input_proj = intermediate_channel_sizes
            .iter()
            .enumerate()
            .map(|(idx, &in_channels)| {
                let path = path / "encoder_input_proj" / idx;
                Ok(nn::seq_t()
                    .add(nn::conv2d(
                        &path / 0,
                        in_channels,
                        config.encoder_hidden_dim,
                        1,
                        nn::ConvConfig {
                            bias: false,
                            ..Default::default()
                        },
                    )?)
                    .add(nn::batch_norm2d(
                        &path / 1,
                        config.encoder_hidden_dim,
                        Default::default(),
                    )?))
            })
            .collect::<tch::Result<Vec<_>>>()?;

        let encoder = RTDetrV2HybridEncoder::new(&(path / "encoder"), config)?;
        let denoising_class_embed = (config.num_denoising > 0)
            .then(|| {
                nn::embedding(
                    path / "denoising_class_embed",
                    config.num_labels() + 1,
                    config.d_model,
                    nn::EmbeddingConfig {
                        padding_idx: config.num_labels(),
                        ..Default::default()
                    },
                )
            })
            .transpose()?;
        let weight_embedding = if config.learn_initial_query {
            Some(nn::embedding(
                path / "weight_embedding",
                config.num_queries,
                config.d_model,
                Default::default(),
            )?)
        } else {
            None
        };

        let enc_output = nn::seq()
            .add(nn::linear(
                path / "enc_output" / 0,
                config.d_model,
                config.d_model,
                Default::default(),
            )?)
            .add(layer_norm(
                &(path / "enc_output" / 1),
                config.d_model,
                config.layer_norm_eps,
            )?);
        let enc_score_head = nn::linear(
            path / "enc_score_head",
            config.d_model,
            config.num_labels(),
            Default::default(),
        )?;
        let enc_bbox_head = RTDetrV2MLPPredictionHead::new(
            &(path / "enc_bbox_head"),
            config.d_model,
            config.d_model,
            4,
            3,
        )?;

        let mut decoder_input_proj = Vec::new();
        let mut in_channels = 0;
        for (idx, &channels) in config.decoder_in_channels.iter().enumerate() {
            in_channels = channels;
            let path = path / "decoder_input_proj" / idx;
            decoder_input_proj.push(
                nn::seq_t()
                    .add(nn::conv2d(
                        &path / 0,
                        channels,
                        config.d_model,
                        1,
                        nn::ConvConfig {
                            bias: false,
                            ..Default::default()
                        },
                    )?)
                    .add(nn::batch_norm2d(
                        &path / 1,
                        config.d_model,
                        nn::BatchNormConfig {
                            eps: config.batch_norm_eps,
                            ..Default::default()
                        },
                    )?),
            );
        }
        for idx in decoder_input_proj.len()..config.num_feature_levels {
            let path = path / "decoder_input_proj" / idx;
            decoder_input_proj.push(
                nn::seq_t()
                    .add(nn::conv2d(
                        &path / 0,
                        in_channels,
                        config.d_model,
                        3,
                        nn::ConvConfig {
                            stride: 2,
                            padding: 1,
                            bias: false,
                            ..Default::default()
                        },
                    )?)
                    .add(nn::batch_norm2d(
                        &path / 1,
                        config.d_model,
                        nn::BatchNormConfig {
                            eps: config.batch_norm_eps,
                            ..Default::default()
                        },
                    )?),
            );
            in_channels = config.d_model;
        }

        let decoder = RTDetrV2Decoder::new(&(path / "decoder"), config)?;

        Ok(Self {
            config: config.clone(),
            anchors,
            valid_mask,
            backbone,
            encoder_input_proj,
            encoder,
            denoising_class_embed,
            weight_embedding,
            enc_output,
            enc_score_head,
            enc_bbox_head,
            decoder_input_proj,
            decoder,
        })
    }

    fn forward(
        &self,
        pixel_values: &Tensor,
        bbox_embed: &[RTDetrV2MLPPredictionHead],
        class_embed: &[nn::Linear],
    ) -> tch::Result<RTDetrV2ObjectDetectionOutput> {
        let features = self.backbone.forward(pixel_values)?;
        let proj_feats = features
            .iter()
            .enumerate()
            .map(|(level, source)| self.encoder_input_proj[level].forward_t(source, false))
            .collect::<tch::Result<Vec<_>>>()?;
        let encoder_outputs = self.encoder.forward(proj_feats)?;

        let mut sources = Vec::with_capacity(self.config.num_feature_levels);
        for (level, source) in encoder_outputs.iter().enumerate() {
            sources.push(self.decoder_input_proj[level].forward_t(source, false)?);
        }
        if self.config.num_feature_levels > sources.len() {
            let base_len = sources.len();
            let last_encoder_output = encoder_outputs.last().expect("encoder output");
            for level in base_len..self.config.num_feature_levels {
                sources.push(self.decoder_input_proj[level].forward_t(last_encoder_output, false)?);
            }
        }

        let mut source_flatten = Vec::with_capacity(sources.len());
        let mut spatial_shapes = Vec::with_capacity(sources.len());
        for source in &sources {
            let size = source.size();
            let height = size[2];
            let width = size[3];
            spatial_shapes.push((height, width));
            source_flatten.push(source.f_flatten(2, -1)?.f_transpose(1, 2)?);
        }
        let source_flatten = Tensor::f_cat(&source_flatten, 1)?;

        // Transformers caches these tensors when `anchor_image_size` is fixed.
        // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py#L1392-L1420
        let (anchors, valid_mask) = match (&self.anchors, &self.valid_mask) {
            (Some(anchors), Some(valid_mask)) => (anchors.f_alias()?, valid_mask.f_alias()?),
            _ => generate_anchors(
                &spatial_shapes,
                source_flatten.device(),
                source_flatten.kind(),
            )?,
        };
        let memory = valid_mask
            .f_to_kind(source_flatten.kind())?
            .f_mul(&source_flatten)?;
        let output_memory = self.enc_output.forward(&memory)?;
        let enc_outputs_class = self.enc_score_head.forward(&output_memory)?;
        let enc_outputs_coord_logits =
            (self.enc_bbox_head.forward(&output_memory)?).f_add(&anchors)?;

        let (_, topk_ind) = enc_outputs_class.f_max_dim(-1, false)?.0.f_topk(
            self.config.num_queries,
            1,
            true,
            true,
        )?;

        let reference_points_unact = enc_outputs_coord_logits.f_gather(
            1,
            &topk_ind
                .f_unsqueeze(-1)?
                .f_repeat([1, 1, enc_outputs_coord_logits.size()[2]])?,
            false,
        )?;

        let target = if let Some(weight_embedding) = &self.weight_embedding {
            weight_embedding
                .ws
                .f_tile([source_flatten.size()[0], 1, 1])?
        } else {
            output_memory
                .f_gather(
                    1,
                    &topk_ind
                        .f_unsqueeze(-1)?
                        .f_repeat([1, 1, output_memory.size()[2]])?,
                    false,
                )?
                .f_detach()?
        };
        let init_reference_points = reference_points_unact.f_detach()?;

        let decoder_outputs = self.decoder.forward(
            &target,
            &source_flatten,
            &init_reference_points,
            &spatial_shapes,
            bbox_embed,
            class_embed,
        )?;

        Ok(RTDetrV2ObjectDetectionOutput {
            logits: decoder_outputs.intermediate_logits.f_select(1, -1)?,
            pred_boxes: decoder_outputs
                .intermediate_reference_points
                .f_select(1, -1)?,
        })
    }
}

#[derive(Debug)]
struct RTDetrV2ConvEncoder {
    model: RTDetrResNetBackbone,
}

impl RTDetrV2ConvEncoder {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        Ok(Self {
            model: RTDetrResNetBackbone::new(&(path / "model"), &config.backbone_config)?,
        })
    }

    fn intermediate_channel_sizes(&self) -> Vec<i64> {
        self.model.channels()
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Vec<Tensor>> {
        self.model.forward(pixel_values)
    }
}

#[derive(Debug)]
struct RTDetrV2FrozenBatchNorm2d {
    weight: Tensor,
    bias: Tensor,
    running_mean: Tensor,
    running_var: Tensor,
}

impl RTDetrV2FrozenBatchNorm2d {
    fn new(path: &nn::Path<'_>, channels: i64) -> Result<Self> {
        Ok(Self {
            weight: path.var("weight", &[channels])?,
            bias: path.var("bias", &[channels])?,
            running_mean: path.var("running_mean", &[channels])?,
            running_var: path.var("running_var", &[channels])?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let weight = self.weight.f_reshape([1, -1, 1, 1])?;
        let bias = self.bias.f_reshape([1, -1, 1, 1])?;
        let running_mean = self.running_mean.f_reshape([1, -1, 1, 1])?;
        let running_var = self.running_var.f_reshape([1, -1, 1, 1])?;
        let scale = weight.f_mul(&running_var.f_add_scalar(1e-5)?.f_rsqrt()?)?;
        ((input.f_mul(&scale)?).f_add(&bias)?).f_sub(&running_mean.f_mul(&scale)?)
    }
}

#[derive(Debug)]
struct RTDetrResNetBackbone {
    config: RTDetrResNetConfig,
    embedder: RTDetrResNetEmbeddings,
    encoder: RTDetrResNetEncoder,
}

impl RTDetrResNetBackbone {
    fn new(path: &nn::Path<'_>, config: &RTDetrResNetConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            embedder: RTDetrResNetEmbeddings::new(&(path / "embedder"), config)?,
            encoder: RTDetrResNetEncoder::new(&(path / "encoder"), config)?,
        })
    }

    fn channels(&self) -> Vec<i64> {
        self.config.channels()
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Vec<Tensor>> {
        let embedding = self.embedder.forward(pixel_values)?;
        let hidden_states = self.encoder.forward(&embedding)?;
        let stage_names = ["stem", "stage1", "stage2", "stage3", "stage4"];
        stage_names
            .iter()
            .enumerate()
            .filter(|(_, stage)| {
                self.config
                    .out_features
                    .iter()
                    .any(|feature| feature == *stage)
            })
            .map(|(idx, _)| hidden_states[idx].f_alias())
            .collect()
    }
}

#[derive(Debug)]
struct RTDetrResNetEmbeddings {
    embedder: Vec<RTDetrResNetConvLayer>,
}

impl RTDetrResNetEmbeddings {
    fn new(path: &nn::Path<'_>, config: &RTDetrResNetConfig) -> Result<Self> {
        Ok(Self {
            embedder: vec![
                RTDetrResNetConvLayer::new(
                    &(path / "embedder" / 0),
                    config.num_channels,
                    config.embedding_size / 2,
                    3,
                    2,
                    Activation::from_name(&config.hidden_act),
                )?,
                RTDetrResNetConvLayer::new(
                    &(path / "embedder" / 1),
                    config.embedding_size / 2,
                    config.embedding_size / 2,
                    3,
                    1,
                    Activation::from_name(&config.hidden_act),
                )?,
                RTDetrResNetConvLayer::new(
                    &(path / "embedder" / 2),
                    config.embedding_size / 2,
                    config.embedding_size,
                    3,
                    1,
                    Activation::from_name(&config.hidden_act),
                )?,
            ],
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        let mut hidden_state = pixel_values.f_alias()?;
        for layer in &self.embedder {
            hidden_state = layer.forward(&hidden_state)?;
        }
        hidden_state.f_max_pool2d([3, 3], [2, 2], [1, 1], [1, 1], false)
    }
}

#[derive(Debug)]
struct RTDetrResNetEncoder {
    stages: Vec<RTDetrResNetStage>,
}

impl RTDetrResNetEncoder {
    fn new(path: &nn::Path<'_>, config: &RTDetrResNetConfig) -> Result<Self> {
        let mut stages = Vec::new();
        stages.push(RTDetrResNetStage::new(
            &(path / "stages" / 0),
            config,
            config.embedding_size,
            config.hidden_sizes[0],
            if config.downsample_in_first_stage {
                2
            } else {
                1
            },
            config.depths[0],
        )?);
        for idx in 1..config.depths.len() {
            stages.push(RTDetrResNetStage::new(
                &(path / "stages" / idx),
                config,
                config.hidden_sizes[idx - 1],
                config.hidden_sizes[idx],
                2,
                config.depths[idx],
            )?);
        }
        Ok(Self { stages })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Vec<Tensor>> {
        let mut hidden_state = input.f_alias()?;
        let mut hidden_states = Vec::with_capacity(self.stages.len() + 1);
        for stage in &self.stages {
            hidden_states.push(hidden_state.f_alias()?);
            hidden_state = stage.forward(&hidden_state)?;
        }
        hidden_states.push(hidden_state);
        Ok(hidden_states)
    }
}

#[derive(Debug)]
struct RTDetrResNetStage {
    layers: Vec<RTDetrResNetBottleNeckLayer>,
}

impl RTDetrResNetStage {
    fn new(
        path: &nn::Path<'_>,
        config: &RTDetrResNetConfig,
        in_channels: i64,
        out_channels: i64,
        stride: i64,
        depth: usize,
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(depth);
        layers.push(RTDetrResNetBottleNeckLayer::new(
            &(path / "layers" / 0),
            config,
            in_channels,
            out_channels,
            stride,
        )?);
        for idx in 1..depth {
            layers.push(RTDetrResNetBottleNeckLayer::new(
                &(path / "layers" / idx),
                config,
                out_channels,
                out_channels,
                1,
            )?);
        }
        Ok(Self { layers })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let mut hidden_state = input.f_alias()?;
        for layer in &self.layers {
            hidden_state = layer.forward(&hidden_state)?;
        }
        Ok(hidden_state)
    }
}

#[derive(Debug)]
struct RTDetrResNetBottleNeckLayer {
    layer: Vec<RTDetrResNetConvLayer>,
    shortcut: Option<RTDetrResNetShortCut>,
    shortcut_avg_pool: bool,
    activation: Activation,
}

impl RTDetrResNetBottleNeckLayer {
    fn new(
        path: &nn::Path<'_>,
        config: &RTDetrResNetConfig,
        in_channels: i64,
        out_channels: i64,
        stride: i64,
    ) -> Result<Self> {
        let reduction = 4;
        let should_apply_shortcut = in_channels != out_channels || stride != 1;
        let reduces_channels = out_channels / reduction;
        let conv1_stride = if config.downsample_in_bottleneck {
            stride
        } else {
            1
        };
        let conv2_stride = if config.downsample_in_bottleneck {
            1
        } else {
            stride
        };
        Ok(Self {
            layer: vec![
                RTDetrResNetConvLayer::new(
                    &(path / "layer" / 0),
                    in_channels,
                    reduces_channels,
                    1,
                    conv1_stride,
                    Activation::from_name(&config.hidden_act),
                )?,
                RTDetrResNetConvLayer::new(
                    &(path / "layer" / 1),
                    reduces_channels,
                    reduces_channels,
                    3,
                    conv2_stride,
                    Activation::from_name(&config.hidden_act),
                )?,
                RTDetrResNetConvLayer::new(
                    &(path / "layer" / 2),
                    reduces_channels,
                    out_channels,
                    1,
                    1,
                    Activation::None,
                )?,
            ],
            shortcut: should_apply_shortcut
                .then(|| {
                    let shortcut_path = if stride == 2 {
                        path / "shortcut" / 1
                    } else {
                        path / "shortcut"
                    };
                    RTDetrResNetShortCut::new(&shortcut_path, in_channels, out_channels)
                })
                .transpose()?,
            shortcut_avg_pool: stride == 2,
            activation: Activation::from_name(&config.hidden_act),
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        let residual = if self.shortcut_avg_pool {
            hidden_state.f_avg_pool2d([2, 2], [2, 2], [0, 0], true, true, None)?
        } else {
            hidden_state.f_alias()?
        };
        let residual = self
            .shortcut
            .as_ref()
            .map(|shortcut| shortcut.forward(&residual))
            .unwrap_or(Ok(residual))?;

        let mut hidden_state = hidden_state.f_alias()?;
        for layer in &self.layer {
            hidden_state = layer.forward(&hidden_state)?;
        }
        self.activation.apply(hidden_state.f_add(&residual)?)
    }
}

#[derive(Debug)]
struct RTDetrResNetShortCut {
    convolution: nn::Conv2D,
    normalization: RTDetrV2FrozenBatchNorm2d,
}

impl RTDetrResNetShortCut {
    fn new(path: &nn::Path<'_>, in_channels: i64, out_channels: i64) -> Result<Self> {
        Ok(Self {
            convolution: nn::conv2d(
                path / "convolution",
                in_channels,
                out_channels,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?,
            normalization: RTDetrV2FrozenBatchNorm2d::new(&(path / "normalization"), out_channels)?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        self.normalization
            .forward(&input.f_apply(&self.convolution)?)
    }
}

#[derive(Debug)]
struct RTDetrResNetConvLayer {
    convolution: nn::Conv2D,
    normalization: RTDetrV2FrozenBatchNorm2d,
    activation: Activation,
}

impl RTDetrResNetConvLayer {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        activation: Activation,
    ) -> Result<Self> {
        Ok(Self {
            convolution: nn::conv2d(
                path / "convolution",
                in_channels,
                out_channels,
                kernel_size,
                nn::ConvConfig {
                    stride,
                    padding: kernel_size / 2,
                    bias: false,
                    ..Default::default()
                },
            )?,
            normalization: RTDetrV2FrozenBatchNorm2d::new(&(path / "normalization"), out_channels)?,
            activation,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        self.activation.apply(
            self.normalization
                .forward(&input.f_apply(&self.convolution)?)?,
        )
    }
}

#[derive(Debug)]
struct RTDetrV2HybridEncoder {
    config: RTDetrV2Config,
    encoder: Vec<RTDetrV2AIFILayer>,
    lateral_convs: Vec<RTDetrV2ConvNormLayer>,
    fpn_blocks: Vec<RTDetrV2CSPRepLayer>,
    downsample_convs: Vec<RTDetrV2ConvNormLayer>,
    pan_blocks: Vec<RTDetrV2CSPRepLayer>,
}

impl RTDetrV2HybridEncoder {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        let stages = config.encoder_in_channels.len() - 1;
        Ok(Self {
            config: config.clone(),
            encoder: config
                .encode_proj_layers
                .iter()
                .enumerate()
                .map(|(idx, _)| RTDetrV2AIFILayer::new(&(path / "encoder" / idx), config))
                .collect::<Result<Vec<_>>>()?,
            lateral_convs: (0..stages)
                .map(|idx| {
                    RTDetrV2ConvNormLayer::new(
                        &(path / "lateral_convs" / idx),
                        config.encoder_hidden_dim,
                        config.encoder_hidden_dim,
                        1,
                        1,
                        Some(0),
                        Activation::from_name(&config.activation_function),
                        config.batch_norm_eps,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
            fpn_blocks: (0..stages)
                .map(|idx| RTDetrV2CSPRepLayer::new(&(path / "fpn_blocks" / idx), config))
                .collect::<Result<Vec<_>>>()?,
            downsample_convs: (0..stages)
                .map(|idx| {
                    RTDetrV2ConvNormLayer::new(
                        &(path / "downsample_convs" / idx),
                        config.encoder_hidden_dim,
                        config.encoder_hidden_dim,
                        3,
                        2,
                        None,
                        Activation::from_name(&config.activation_function),
                        config.batch_norm_eps,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
            pan_blocks: (0..stages)
                .map(|idx| RTDetrV2CSPRepLayer::new(&(path / "pan_blocks" / idx), config))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    fn forward(&self, mut feature_maps: Vec<Tensor>) -> tch::Result<Vec<Tensor>> {
        if self.config.encoder_layers > 0 {
            for (idx, &enc_ind) in self.config.encode_proj_layers.iter().enumerate() {
                feature_maps[enc_ind] = self.encoder[idx].forward(&feature_maps[enc_ind])?;
            }
        }

        // Preserve Transformers' AIFI -> top-down FPN -> bottom-up PAN ordering;
        // changing the traversal changes which resolution is fused at each stage.
        // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py#L1112-L1145
        let mut fpn_feature_maps = vec![feature_maps.last().expect("feature map").f_alias()?];
        for idx in 0..self.lateral_convs.len() {
            let backbone_feature_map =
                feature_maps[self.lateral_convs.len() - idx - 1].f_alias()?;
            let top = self.lateral_convs[idx].forward(fpn_feature_maps.last().expect("fpn"))?;
            *fpn_feature_maps.last_mut().expect("fpn") = top.f_alias()?;
            let size = top.size();
            let upsampled =
                top.f_upsample_nearest2d([size[2] * 2, size[3] * 2], None::<f64>, None::<f64>)?;
            let fused = Tensor::f_cat(&[upsampled, backbone_feature_map], 1)?;
            fpn_feature_maps.push(self.fpn_blocks[idx].forward(&fused)?);
        }
        fpn_feature_maps.reverse();

        let mut pan_feature_maps = vec![fpn_feature_maps[0].f_alias()?];
        for idx in 0..self.downsample_convs.len() {
            let downsampled =
                self.downsample_convs[idx].forward(pan_feature_maps.last().expect("pan"))?;
            let fused = Tensor::f_cat(&[downsampled, fpn_feature_maps[idx + 1].f_alias()?], 1)?;
            pan_feature_maps.push(self.pan_blocks[idx].forward(&fused)?);
        }

        Ok(pan_feature_maps)
    }
}

#[derive(Debug)]
struct RTDetrV2AIFILayer {
    encoder_hidden_dim: i64,
    position_embedding: RTDetrV2SinePositionEmbedding,
    layers: Vec<RTDetrV2EncoderLayer>,
}

impl RTDetrV2AIFILayer {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        Ok(Self {
            encoder_hidden_dim: config.encoder_hidden_dim,
            position_embedding: RTDetrV2SinePositionEmbedding {
                embed_dim: config.encoder_hidden_dim,
                temperature: config.positional_encoding_temperature,
            },
            layers: (0..config.encoder_layers)
                .map(|idx| RTDetrV2EncoderLayer::new(&(path / "layers" / idx), config))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let size = hidden_states.size();
        let batch_size = size[0];
        let height = size[2];
        let width = size[3];
        let mut hidden_states = hidden_states.f_flatten(2, -1)?.f_permute([0, 2, 1])?;
        let pos_embed = self.position_embedding.forward(
            width,
            height,
            hidden_states.device(),
            hidden_states.kind(),
        )?;
        for layer in &self.layers {
            hidden_states = layer.forward(&hidden_states, Some(&pos_embed))?;
        }
        hidden_states
            .f_permute([0, 2, 1])?
            .f_reshape([batch_size, self.encoder_hidden_dim, height, width])?
            .f_contiguous()
    }
}

#[derive(Debug)]
struct RTDetrV2EncoderLayer {
    normalize_before: bool,
    self_attn: RTDetrV2SelfAttention,
    self_attn_layer_norm: nn::LayerNorm,
    mlp: RTDetrV2MLP,
    final_layer_norm: nn::LayerNorm,
}

impl RTDetrV2EncoderLayer {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        Ok(Self {
            normalize_before: config.normalize_before,
            self_attn: RTDetrV2SelfAttention::new(
                &(path / "self_attn"),
                config.encoder_hidden_dim,
                config.encoder_attention_heads,
            )?,
            self_attn_layer_norm: layer_norm(
                &(path / "self_attn_layer_norm"),
                config.encoder_hidden_dim,
                config.layer_norm_eps,
            )?,
            mlp: RTDetrV2MLP::new(
                path,
                config.encoder_hidden_dim,
                config.encoder_ffn_dim,
                Activation::from_name(&config.encoder_activation_function),
            )?,
            final_layer_norm: layer_norm(
                &(path / "final_layer_norm"),
                config.encoder_hidden_dim,
                config.layer_norm_eps,
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        position_embeddings: Option<&Tensor>,
    ) -> tch::Result<Tensor> {
        let residual = hidden_states.f_alias()?;
        let hidden_states = if self.normalize_before {
            self.self_attn_layer_norm.forward(hidden_states)?
        } else {
            hidden_states.f_alias()?
        };
        let hidden_states = self
            .self_attn
            .forward(&hidden_states, position_embeddings)?;
        let hidden_states = residual.f_add(&hidden_states)?;
        let hidden_states = if self.normalize_before {
            hidden_states
        } else {
            self.self_attn_layer_norm.forward(&hidden_states)?
        };

        let residual = if self.normalize_before {
            self.final_layer_norm.forward(&hidden_states)?
        } else {
            hidden_states.f_alias()?
        };
        let hidden_states = residual.f_alias()?.f_add(&self.mlp.forward(&residual)?)?;
        if self.normalize_before {
            Ok(hidden_states)
        } else {
            self.final_layer_norm.forward(&hidden_states)
        }
    }
}

#[derive(Debug)]
struct RTDetrV2DecoderOutput {
    intermediate_logits: Tensor,
    intermediate_reference_points: Tensor,
}

#[derive(Debug)]
struct RTDetrV2Decoder {
    layers: Vec<RTDetrV2DecoderLayer>,
    query_pos_head: RTDetrV2MLPPredictionHead,
}

impl RTDetrV2Decoder {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        Ok(Self {
            layers: (0..config.decoder_layers)
                .map(|idx| RTDetrV2DecoderLayer::new(&(path / "layers" / idx), config))
                .collect::<Result<Vec<_>>>()?,
            query_pos_head: RTDetrV2MLPPredictionHead::new(
                &(path / "query_pos_head"),
                4,
                2 * config.d_model,
                config.d_model,
                2,
            )?,
        })
    }

    fn forward(
        &self,
        inputs_embeds: &Tensor,
        encoder_hidden_states: &Tensor,
        reference_points: &Tensor,
        spatial_shapes_list: &[(i64, i64)],
        bbox_embed: &[RTDetrV2MLPPredictionHead],
        class_embed: &[nn::Linear],
    ) -> tch::Result<RTDetrV2DecoderOutput> {
        let mut hidden_states = inputs_embeds.f_alias()?;
        let mut reference_points = reference_points.f_sigmoid()?;
        let mut intermediate_logits = Vec::with_capacity(self.layers.len());
        let mut intermediate_reference_points = Vec::with_capacity(self.layers.len());

        for (idx, decoder_layer) in self.layers.iter().enumerate() {
            let reference_points_input = reference_points.f_unsqueeze(2)?;
            let object_queries_position_embeddings =
                self.query_pos_head.forward(&reference_points)?;
            hidden_states = decoder_layer.forward(
                &hidden_states,
                &object_queries_position_embeddings,
                &reference_points_input,
                spatial_shapes_list,
                encoder_hidden_states,
            )?;

            // Iterative box refinement intentionally detaches only the reference
            // used by the next decoder layer, matching Transformers inference.
            // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py#L626-L636
            let predicted_corners = bbox_embed[idx].forward(&hidden_states)?;
            let new_reference_points = predicted_corners
                .f_add(&inverse_sigmoid(&reference_points)?)?
                .f_sigmoid()?;
            reference_points = new_reference_points.f_detach()?;
            intermediate_reference_points.push(new_reference_points);
            intermediate_logits.push(class_embed[idx].forward(&hidden_states)?);
        }

        Ok(RTDetrV2DecoderOutput {
            intermediate_logits: Tensor::f_stack(&intermediate_logits, 1)?,
            intermediate_reference_points: Tensor::f_stack(&intermediate_reference_points, 1)?,
        })
    }
}

#[derive(Debug)]
struct RTDetrV2DecoderLayer {
    self_attn: RTDetrV2SelfAttention,
    self_attn_layer_norm: nn::LayerNorm,
    encoder_attn: RTDetrV2MultiscaleDeformableAttention,
    encoder_attn_layer_norm: nn::LayerNorm,
    mlp: RTDetrV2MLP,
    final_layer_norm: nn::LayerNorm,
}

impl RTDetrV2DecoderLayer {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        Ok(Self {
            self_attn: RTDetrV2SelfAttention::new(
                &(path / "self_attn"),
                config.d_model,
                config.decoder_attention_heads,
            )?,
            self_attn_layer_norm: layer_norm(
                &(path / "self_attn_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            )?,
            encoder_attn: RTDetrV2MultiscaleDeformableAttention::new(
                &(path / "encoder_attn"),
                config,
            )?,
            encoder_attn_layer_norm: layer_norm(
                &(path / "encoder_attn_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            )?,
            mlp: RTDetrV2MLP::new(
                path,
                config.d_model,
                config.decoder_ffn_dim,
                Activation::from_name(&config.decoder_activation_function),
            )?,
            final_layer_norm: layer_norm(
                &(path / "final_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        object_queries_position_embeddings: &Tensor,
        reference_points: &Tensor,
        spatial_shapes_list: &[(i64, i64)],
        encoder_hidden_states: &Tensor,
    ) -> tch::Result<Tensor> {
        let residual = hidden_states.f_alias()?;
        let hidden_states = self
            .self_attn
            .forward(hidden_states, Some(object_queries_position_embeddings))?;
        let hidden_states = self
            .self_attn_layer_norm
            .forward(&residual.f_add(&hidden_states)?)?;

        let residual = hidden_states.f_alias()?;
        let hidden_states = self.encoder_attn.forward(
            &hidden_states,
            encoder_hidden_states,
            object_queries_position_embeddings,
            reference_points,
            spatial_shapes_list,
        )?;
        let hidden_states = self
            .encoder_attn_layer_norm
            .forward(&residual.f_add(&hidden_states)?)?;
        let residual = hidden_states.f_alias()?;
        self.final_layer_norm
            .forward(&residual.f_alias()?.f_add(&self.mlp.forward(&residual)?)?)
    }
}

#[derive(Debug)]
struct RTDetrV2SelfAttention {
    head_dim: i64,
    num_heads: i64,
    scaling: f64,
    k_proj: nn::Linear,
    v_proj: nn::Linear,
    q_proj: nn::Linear,
    o_proj: nn::Linear,
}

impl RTDetrV2SelfAttention {
    fn new(path: &nn::Path<'_>, hidden_size: i64, num_heads: i64) -> Result<Self> {
        let head_dim = hidden_size / num_heads;
        Ok(Self {
            head_dim,
            num_heads,
            scaling: (head_dim as f64).powf(-0.5),
            k_proj: nn::linear(
                path / "k_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            )?,
            v_proj: nn::linear(
                path / "v_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            )?,
            q_proj: nn::linear(
                path / "q_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            )?,
            // The model was exported by Transformers 4.49, before this field was
            // renamed from `out_proj` to `o_proj`; retain its checkpoint path.
            o_proj: nn::linear(
                path / "out_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        position_embeddings: Option<&Tensor>,
    ) -> tch::Result<Tensor> {
        let size = hidden_states.size();
        let batch_size = size[0];
        let sequence_length = size[1];
        let query_key_input = position_embeddings
            .map(|position_embeddings| hidden_states.f_add(position_embeddings))
            .unwrap_or_else(|| hidden_states.f_alias())?;
        let query_states = self
            .q_proj
            .forward(&query_key_input)?
            .f_view([batch_size, sequence_length, self.num_heads, self.head_dim])?
            .f_transpose(1, 2)?;
        let key_states = self
            .k_proj
            .forward(&query_key_input)?
            .f_view([batch_size, sequence_length, self.num_heads, self.head_dim])?
            .f_transpose(1, 2)?;
        let value_states = self
            .v_proj
            .forward(hidden_states)?
            .f_view([batch_size, sequence_length, self.num_heads, self.head_dim])?
            .f_transpose(1, 2)?;
        let attn_weights =
            (query_states.f_matmul(&key_states.f_transpose(2, 3)?)?).f_mul_scalar(self.scaling)?;
        let attn_weights = attn_weights.f_softmax(-1, None::<Kind>)?;
        let attn_output = attn_weights
            .f_matmul(&value_states)?
            .f_transpose(1, 2)?
            .f_contiguous()?;
        self.o_proj.forward(&attn_output.f_reshape([
            batch_size,
            sequence_length,
            self.num_heads * self.head_dim,
        ])?)
    }
}

#[derive(Debug)]
struct RTDetrV2MultiscaleDeformableAttention {
    d_model: i64,
    n_levels: i64,
    n_heads: i64,
    n_points: i64,
    offset_scale: f64,
    method: String,
    n_points_scale: Tensor,
    sampling_offsets: nn::Linear,
    attention_weights: nn::Linear,
    value_proj: nn::Linear,
    output_proj: nn::Linear,
}

impl RTDetrV2MultiscaleDeformableAttention {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        let n_levels = config.decoder_n_levels;
        let n_points = config.decoder_n_points;
        let n_points_scale_values = (0..n_levels)
            .flat_map(|_| (0..n_points).map(move |_| 1.0f32 / n_points as f32))
            .collect::<Vec<_>>();
        let mut n_points_scale = path.var("n_points_scale", &[n_levels * n_points])?;
        n_points_scale.f_copy_(&Tensor::f_from_slice(&n_points_scale_values)?)?;
        Ok(Self {
            d_model: config.d_model,
            n_levels,
            n_heads: config.decoder_attention_heads,
            n_points,
            offset_scale: config.decoder_offset_scale,
            method: config.decoder_method.clone(),
            n_points_scale,
            sampling_offsets: nn::linear(
                path / "sampling_offsets",
                config.d_model,
                config.decoder_attention_heads * n_levels * n_points * 2,
                Default::default(),
            )?,
            attention_weights: nn::linear(
                path / "attention_weights",
                config.d_model,
                config.decoder_attention_heads * n_levels * n_points,
                Default::default(),
            )?,
            value_proj: nn::linear(
                path / "value_proj",
                config.d_model,
                config.d_model,
                Default::default(),
            )?,
            output_proj: nn::linear(
                path / "output_proj",
                config.d_model,
                config.d_model,
                Default::default(),
            )?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        encoder_hidden_states: &Tensor,
        position_embeddings: &Tensor,
        reference_points: &Tensor,
        spatial_shapes_list: &[(i64, i64)],
    ) -> tch::Result<Tensor> {
        let hidden_states = hidden_states.f_add(position_embeddings)?;
        let batch_size = hidden_states.size()[0];
        let num_queries = hidden_states.size()[1];
        let sequence_length = encoder_hidden_states.size()[1];
        let dim_per_head = self.d_model / self.n_heads;

        let value = self.value_proj.forward(encoder_hidden_states)?.f_view([
            batch_size,
            sequence_length,
            self.n_heads,
            dim_per_head,
        ])?;
        let sampling_offsets = self.sampling_offsets.forward(&hidden_states)?.f_view([
            batch_size,
            num_queries,
            self.n_heads,
            self.n_levels * self.n_points,
            2,
        ])?;
        let attention_weights = self
            .attention_weights
            .forward(&hidden_states)?
            .f_view([
                batch_size,
                num_queries,
                self.n_heads,
                self.n_levels * self.n_points,
            ])?
            .f_softmax(-1, None::<Kind>)?;

        let sampling_locations = if reference_points.size().last().copied() == Some(4) {
            let scale = self
                .n_points_scale
                .f_to_kind(hidden_states.kind())?
                .f_view([1, 1, 1, self.n_levels * self.n_points, 1])?;
            let ref_xy = reference_points.f_slice(-1, 0, 2, 1)?.f_unsqueeze(2)?;
            let ref_wh = reference_points.f_slice(-1, 2, 4, 1)?.f_unsqueeze(2)?;
            ref_xy.f_add(
                &sampling_offsets
                    .f_mul(&scale)?
                    .f_mul(&ref_wh)?
                    .f_mul_scalar(self.offset_scale)?,
            )?
        } else {
            let spatial_shapes = spatial_shapes_tensor(spatial_shapes_list, value.device())?
                .f_to_kind(hidden_states.kind())?;
            let normalizer = Tensor::f_stack(
                &[spatial_shapes.f_i((.., 1))?, spatial_shapes.f_i((.., 0))?],
                -1,
            )?;
            reference_points.f_unsqueeze(2)?.f_unsqueeze(4)?.f_add(
                &sampling_offsets.f_div(
                    &normalizer
                        .f_unsqueeze(0)?
                        .f_unsqueeze(0)?
                        .f_unsqueeze(0)?
                        .f_unsqueeze(4)?,
                )?,
            )?
        };

        let output = multiscale_deformable_attention_v2(
            &value,
            spatial_shapes_list,
            &sampling_locations,
            &attention_weights,
            self.n_points as usize,
            &self.method,
        )?;
        self.output_proj.forward(&output)
    }
}

#[derive(Debug)]
struct RTDetrV2MLP {
    fc1: nn::Linear,
    fc2: nn::Linear,
    activation: Activation,
}

impl RTDetrV2MLP {
    fn new(
        path: &nn::Path<'_>,
        hidden_size: i64,
        intermediate_size: i64,
        activation: Activation,
    ) -> Result<Self> {
        Ok(Self {
            fc1: nn::linear(
                path / "fc1",
                hidden_size,
                intermediate_size,
                Default::default(),
            )?,
            fc2: nn::linear(
                path / "fc2",
                intermediate_size,
                hidden_size,
                Default::default(),
            )?,
            activation,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> tch::Result<Tensor> {
        let hidden_states = self.activation.apply(self.fc1.forward(hidden_states)?)?;
        self.fc2.forward(&hidden_states)
    }
}

#[derive(Debug)]
struct RTDetrV2MLPPredictionHead {
    layers: Vec<nn::Linear>,
}

impl RTDetrV2MLPPredictionHead {
    fn new(
        path: &nn::Path<'_>,
        input_dim: i64,
        hidden_dim: i64,
        output_dim: i64,
        num_layers: usize,
    ) -> Result<Self> {
        let mut dims = Vec::with_capacity(num_layers + 1);
        dims.push(input_dim);
        for _ in 0..num_layers - 1 {
            dims.push(hidden_dim);
        }
        dims.push(output_dim);
        let layers = (0..num_layers)
            .map(|idx| {
                nn::linear(
                    path / "layers" / idx,
                    dims[idx],
                    dims[idx + 1],
                    Default::default(),
                )
            })
            .collect::<tch::Result<Vec<_>>>()?;
        Ok(Self { layers })
    }

    fn forward(&self, x: &Tensor) -> tch::Result<Tensor> {
        let mut x = x.f_alias()?;
        for (idx, layer) in self.layers.iter().enumerate() {
            x = layer.forward(&x)?;
            if idx + 1 != self.layers.len() {
                x = x.f_relu()?;
            }
        }
        Ok(x)
    }
}

#[derive(Debug)]
struct RTDetrV2CSPRepLayer {
    conv1: RTDetrV2ConvNormLayer,
    conv2: RTDetrV2ConvNormLayer,
    bottlenecks: Vec<RTDetrV2RepVggBlock>,
}

impl RTDetrV2CSPRepLayer {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        let in_channels = config.encoder_hidden_dim * 2;
        let out_channels = config.encoder_hidden_dim;
        let hidden_channels = (out_channels as f64 * config.hidden_expansion) as i64;
        Ok(Self {
            conv1: RTDetrV2ConvNormLayer::new(
                &(path / "conv1"),
                in_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::from_name(&config.activation_function),
                config.batch_norm_eps,
            )?,
            conv2: RTDetrV2ConvNormLayer::new(
                &(path / "conv2"),
                in_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::from_name(&config.activation_function),
                config.batch_norm_eps,
            )?,
            bottlenecks: (0..3)
                .map(|idx| RTDetrV2RepVggBlock::new(&(path / "bottlenecks" / idx), config))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        let mut hidden_state_1 = self.conv1.forward(hidden_state)?;
        for bottleneck in &self.bottlenecks {
            hidden_state_1 = bottleneck.forward(&hidden_state_1)?;
        }
        hidden_state_1.f_add(&self.conv2.forward(hidden_state)?)
    }
}

#[derive(Debug)]
struct RTDetrV2RepVggBlock {
    conv1: RTDetrV2ConvNormLayer,
    conv2: RTDetrV2ConvNormLayer,
    activation: Activation,
}

impl RTDetrV2RepVggBlock {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Result<Self> {
        let hidden_channels = (config.encoder_hidden_dim as f64 * config.hidden_expansion) as i64;
        Ok(Self {
            conv1: RTDetrV2ConvNormLayer::new(
                &(path / "conv1"),
                hidden_channels,
                hidden_channels,
                3,
                1,
                Some(1),
                Activation::None,
                config.batch_norm_eps,
            )?,
            conv2: RTDetrV2ConvNormLayer::new(
                &(path / "conv2"),
                hidden_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::None,
                config.batch_norm_eps,
            )?,
            activation: Activation::from_name(&config.activation_function),
        })
    }

    fn forward(&self, x: &Tensor) -> tch::Result<Tensor> {
        self.activation
            .apply(self.conv1.forward(x)?.f_add(&self.conv2.forward(x)?)?)
    }
}

#[derive(Debug)]
struct RTDetrV2ConvNormLayer {
    conv: nn::Conv2D,
    norm: nn::BatchNorm,
    activation: Activation,
}

impl RTDetrV2ConvNormLayer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        padding: Option<i64>,
        activation: Activation,
        eps: f64,
    ) -> Result<Self> {
        Ok(Self {
            conv: nn::conv2d(
                path / "conv",
                in_channels,
                out_channels,
                kernel_size,
                nn::ConvConfig {
                    stride,
                    padding: padding.unwrap_or((kernel_size - 1) / 2),
                    bias: false,
                    ..Default::default()
                },
            )?,
            norm: nn::batch_norm2d(
                path / "norm",
                out_channels,
                nn::BatchNormConfig {
                    eps,
                    ..Default::default()
                },
            )?,
            activation,
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        self.activation.apply(
            hidden_state
                .f_apply(&self.conv)?
                .f_apply_t(&self.norm, false)?,
        )
    }
}

#[derive(Debug)]
struct RTDetrV2SinePositionEmbedding {
    embed_dim: i64,
    temperature: f64,
}

impl RTDetrV2SinePositionEmbedding {
    fn forward(&self, width: i64, height: i64, device: Device, kind: Kind) -> tch::Result<Tensor> {
        let grid_w = Tensor::f_arange(width, (kind, device))?;
        let grid_h = Tensor::f_arange(height, (kind, device))?;
        let mesh = Tensor::f_meshgrid_indexing(&[grid_w, grid_h], "xy")?;
        let grid_w = mesh[0].f_flatten(0, -1)?.f_unsqueeze(-1)?;
        let grid_h = mesh[1].f_flatten(0, -1)?.f_unsqueeze(-1)?;
        let pos_dim = self.embed_dim / 4;
        let omega = (Tensor::f_arange(pos_dim, (kind, device))?).f_div_scalar(pos_dim as f64)?;
        let omega = omega
            .f_mul_scalar(self.temperature.ln())?
            .f_exp()?
            .f_reciprocal()?;
        let out_w = grid_w.f_matmul(&omega.f_unsqueeze(0)?)?;
        let out_h = grid_h.f_matmul(&omega.f_unsqueeze(0)?)?;
        Tensor::f_cat(
            &[
                out_h.f_sin()?,
                out_h.f_cos()?,
                out_w.f_sin()?,
                out_w.f_cos()?,
            ],
            1,
        )?
        .f_unsqueeze(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activation {
    None,
    Relu,
    Silu,
    Gelu,
}

impl Activation {
    fn from_name(name: &str) -> Self {
        match name {
            "relu" => Self::Relu,
            "silu" | "swish" => Self::Silu,
            "gelu" | "gelu_new" => Self::Gelu,
            _ => Self::None,
        }
    }

    fn apply(self, x: Tensor) -> tch::Result<Tensor> {
        match self {
            Self::None => Ok(x),
            Self::Relu => x.f_relu(),
            Self::Silu => x.f_silu(),
            Self::Gelu => x.f_gelu("none"),
        }
    }
}

fn layer_norm(path: &nn::Path<'_>, hidden_dim: i64, eps: f64) -> Result<nn::LayerNorm> {
    Ok(nn::layer_norm(
        path,
        vec![hidden_dim],
        nn::LayerNormConfig {
            eps,
            ..Default::default()
        },
    )?)
}

fn multiscale_deformable_attention_v2(
    value: &Tensor,
    spatial_shapes_list: &[(i64, i64)],
    sampling_locations: &Tensor,
    attention_weights: &Tensor,
    n_points: usize,
    method: &str,
) -> tch::Result<Tensor> {
    let size = value.size();
    let batch_size = size[0];
    let num_heads = size[2];
    let hidden_dim = size[3];
    let num_queries = sampling_locations.size()[1];
    let split_sizes = spatial_shapes_list
        .iter()
        .map(|(height, width)| height * width)
        .collect::<Vec<_>>();
    let value_list = value.f_split_with_sizes(split_sizes, 1)?;
    let sampling_grids = match method {
        "default" => sampling_locations.f_mul_scalar(2.0)?.f_sub_scalar(1.0)?,
        "discrete" => sampling_locations.f_alias()?,
        _ => panic!("unsupported RT-DETR-v2 decoder method: {method}"),
    };
    let sampling_grids = sampling_grids.f_permute([0, 2, 1, 3, 4])?.f_flatten(0, 1)?;
    let point_splits = vec![n_points as i64; spatial_shapes_list.len()];
    let sampling_grid_list = sampling_grids.f_split_with_sizes(point_splits, -2)?;

    let mut sampling_values = Vec::with_capacity(spatial_shapes_list.len());
    for (level, (height, width)) in spatial_shapes_list.iter().copied().enumerate() {
        let value_l = value_list[level]
            .f_flatten(2, -1)?
            .f_transpose(1, 2)?
            .f_reshape([batch_size * num_heads, hidden_dim, height, width])?;
        let sampling_grid = &sampling_grid_list[level];
        let sampling_value = if method == "default" {
            value_l.f_grid_sampler_2d(sampling_grid, 0, 0, false)?
        } else {
            let scale = Tensor::f_from_slice(&[width, height])?
                .f_to_device(value.device())?
                .f_view([1, 1, 1, 2])?;
            let sampling_coord = sampling_grid
                .f_mul(&scale)?
                .f_add_scalar(0.5)?
                .f_to_kind(Kind::Int64)?;
            let x = sampling_coord.f_i((.., .., .., 0))?.f_clamp(0, width - 1)?;
            let y = sampling_coord
                .f_i((.., .., .., 1))?
                .f_clamp(0, height - 1)?;
            let index = y.f_mul_scalar(width)?.f_add(&x)?.f_flatten(1, -1)?;
            value_l
                .f_flatten(2, -1)?
                .f_gather(
                    2,
                    &index.f_unsqueeze(1)?.f_repeat([1, hidden_dim, 1])?,
                    false,
                )?
                .f_view([
                    batch_size * num_heads,
                    hidden_dim,
                    num_queries,
                    n_points as i64,
                ])?
        };
        sampling_values.push(sampling_value);
    }

    let attention_weights = attention_weights.f_permute([0, 2, 1, 3])?.f_reshape([
        batch_size * num_heads,
        1,
        num_queries,
        -1,
    ])?;
    (Tensor::f_cat(&sampling_values, -1)?)
        .f_mul(&attention_weights)?
        .f_sum_dim_intlist(&[-1i64][..], false, None::<Kind>)?
        .f_view([batch_size, num_heads * hidden_dim, num_queries])?
        .f_transpose(1, 2)?
        .f_contiguous()
}

fn generate_anchors(
    spatial_shapes: &[(i64, i64)],
    device: Device,
    kind: Kind,
) -> tch::Result<(Tensor, Tensor)> {
    let mut anchors = Vec::new();
    let mut valid = Vec::new();
    let eps = 1e-2f32;
    for (level, &(height, width)) in spatial_shapes.iter().enumerate() {
        let wh = 0.05f32 * 2f32.powi(level as i32);
        for y in 0..height {
            for x in 0..width {
                let cx = (x as f32 + 0.5) / width as f32;
                let cy = (y as f32 + 0.5) / height as f32;
                let row = [cx, cy, wh, wh];
                valid.push(row.iter().all(|value| *value > eps && *value < 1.0 - eps));
                anchors.extend(row);
            }
        }
    }
    let total = valid.len() as i64;
    let anchors = Tensor::f_from_slice(&anchors)?
        .f_view([1, total, 4])?
        .f_to_device(device)?
        .f_to_kind(kind)?;
    let valid_mask = Tensor::f_from_slice(&valid)?
        .f_view([1, total, 1])?
        .f_to_device(device)?;
    let one_minus_anchors = anchors.f_ones_like()?.f_sub(&anchors)?;
    let logit = anchors.f_div(&one_minus_anchors)?.f_log()?;
    let finfo_max = match kind {
        Kind::Half => 65_504.0,
        Kind::Double => f64::MAX,
        // (2 - 2^-7) * 2^127; a rounder figure overflows the conversion.
        Kind::BFloat16 => 3.389_531_389_251_535_5e38,
        _ => f32::MAX as f64,
    };
    let max = Tensor::f_full([1, total, 4], finfo_max, (kind, device))?;
    let anchors = logit.f_where_self(&valid_mask.f_expand([1, total, 4], true)?, &max)?;
    Ok((anchors, valid_mask))
}

fn fixed_spatial_shapes(
    image_size: &[i64],
    feature_strides: &[i64],
    num_feature_levels: usize,
) -> Option<Vec<(i64, i64)>> {
    let [height, width] = image_size else {
        return None;
    };
    let mut stride = *feature_strides.last()?;
    let mut spatial_shapes = Vec::with_capacity(num_feature_levels);
    for level in 0..num_feature_levels {
        if let Some(&configured_stride) = feature_strides.get(level) {
            stride = configured_stride;
        } else {
            stride *= 2;
        }
        spatial_shapes.push((height / stride, width / stride));
    }
    Some(spatial_shapes)
}

fn inverse_sigmoid(x: &Tensor) -> tch::Result<Tensor> {
    let x = x.f_clamp(0.0, 1.0)?;
    let x1 = x.f_clamp_min(1e-5)?;
    let x2 = x.f_ones_like()?.f_sub(&x)?.f_clamp_min(1e-5)?;
    x1.f_div(&x2)?.f_log()
}

fn spatial_shapes_tensor(spatial_shapes: &[(i64, i64)], device: Device) -> tch::Result<Tensor> {
    let values = spatial_shapes
        .iter()
        .flat_map(|(h, w)| [*h, *w])
        .collect::<Vec<_>>();
    Tensor::f_from_slice(&values)?
        .f_view([spatial_shapes.len() as i64, 2])?
        .f_to_device(device)
}
