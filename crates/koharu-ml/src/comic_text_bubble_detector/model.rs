//! Inference-only port of Transformers' RT-DETR-v2 object detector.
//!
//! Original implementations:
//! - https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py
//! - https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr/modeling_rt_detr_resnet.py

use std::path::Path;

use anyhow::Result;
use koharu_torch::{
    Device, IndexOp, Kind, Tensor,
    nn::{self, Module, ModuleT},
};

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
    pub fn new(config: RTDetrV2Config, device: Device) -> Self {
        let mut vs = nn::VarStore::new(device);
        crate::backend::set_precision(&mut vs);
        let model = RTDetrV2ForObjectDetection::new(&vs.root(), &config);
        vs.freeze();
        Self { vs, model }
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        crate::backend::set_precision(&mut self.vs);
        Ok(())
    }

    pub fn forward(&self, pixel_values: &Tensor) -> RTDetrV2ObjectDetectionOutput {
        self.model.forward(&pixel_values.to_kind(self.vs.kind()))
    }
}

#[derive(Debug)]
struct RTDetrV2ForObjectDetection {
    model: RTDetrV2Model,
    class_embed: Vec<nn::Linear>,
    bbox_embed: Vec<RTDetrV2MLPPredictionHead>,
}

impl RTDetrV2ForObjectDetection {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        Self {
            model: RTDetrV2Model::new(&(path / "model"), config),
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
                .collect(),
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
                .collect(),
        }
    }

    fn forward(&self, pixel_values: &Tensor) -> RTDetrV2ObjectDetectionOutput {
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
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        let (anchors, valid_mask) = config
            .anchor_image_size
            .as_deref()
            .and_then(|image_size| {
                fixed_spatial_shapes(image_size, &config.feat_strides, config.num_feature_levels)
            })
            .map(|spatial_shapes| generate_anchors(&spatial_shapes, path.device(), path.kind()))
            .map_or((None, None), |(anchors, valid_mask)| {
                (Some(anchors), Some(valid_mask))
            });
        let backbone = RTDetrV2ConvEncoder::new(&(path / "backbone"), config);
        let intermediate_channel_sizes = backbone.intermediate_channel_sizes();

        let encoder_input_proj = intermediate_channel_sizes
            .iter()
            .enumerate()
            .map(|(idx, &in_channels)| {
                let path = path / "encoder_input_proj" / idx;
                nn::seq_t()
                    .add(nn::conv2d(
                        &path / 0,
                        in_channels,
                        config.encoder_hidden_dim,
                        1,
                        nn::ConvConfig {
                            bias: false,
                            ..Default::default()
                        },
                    ))
                    .add(nn::batch_norm2d(
                        &path / 1,
                        config.encoder_hidden_dim,
                        Default::default(),
                    ))
            })
            .collect();

        let encoder = RTDetrV2HybridEncoder::new(&(path / "encoder"), config);
        let denoising_class_embed = (config.num_denoising > 0).then(|| {
            nn::embedding(
                path / "denoising_class_embed",
                config.num_labels() + 1,
                config.d_model,
                nn::EmbeddingConfig {
                    padding_idx: config.num_labels(),
                    ..Default::default()
                },
            )
        });
        let weight_embedding = if config.learn_initial_query {
            Some(nn::embedding(
                path / "weight_embedding",
                config.num_queries,
                config.d_model,
                Default::default(),
            ))
        } else {
            None
        };

        let enc_output = nn::seq()
            .add(nn::linear(
                path / "enc_output" / 0,
                config.d_model,
                config.d_model,
                Default::default(),
            ))
            .add(layer_norm(
                &(path / "enc_output" / 1),
                config.d_model,
                config.layer_norm_eps,
            ));
        let enc_score_head = nn::linear(
            path / "enc_score_head",
            config.d_model,
            config.num_labels(),
            Default::default(),
        );
        let enc_bbox_head = RTDetrV2MLPPredictionHead::new(
            &(path / "enc_bbox_head"),
            config.d_model,
            config.d_model,
            4,
            3,
        );

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
                    ))
                    .add(nn::batch_norm2d(
                        &path / 1,
                        config.d_model,
                        nn::BatchNormConfig {
                            eps: config.batch_norm_eps,
                            ..Default::default()
                        },
                    )),
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
                    ))
                    .add(nn::batch_norm2d(
                        &path / 1,
                        config.d_model,
                        nn::BatchNormConfig {
                            eps: config.batch_norm_eps,
                            ..Default::default()
                        },
                    )),
            );
            in_channels = config.d_model;
        }

        let decoder = RTDetrV2Decoder::new(&(path / "decoder"), config);

        Self {
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
        }
    }

    fn forward(
        &self,
        pixel_values: &Tensor,
        bbox_embed: &[RTDetrV2MLPPredictionHead],
        class_embed: &[nn::Linear],
    ) -> RTDetrV2ObjectDetectionOutput {
        let features = self.backbone.forward(pixel_values);
        let proj_feats = features
            .iter()
            .enumerate()
            .map(|(level, source)| self.encoder_input_proj[level].forward_t(source, false))
            .collect::<Vec<_>>();
        let encoder_outputs = self.encoder.forward(proj_feats);

        let mut sources = Vec::with_capacity(self.config.num_feature_levels);
        for (level, source) in encoder_outputs.iter().enumerate() {
            sources.push(self.decoder_input_proj[level].forward_t(source, false));
        }
        if self.config.num_feature_levels > sources.len() {
            let base_len = sources.len();
            let last_encoder_output = encoder_outputs.last().expect("encoder output");
            for level in base_len..self.config.num_feature_levels {
                sources.push(self.decoder_input_proj[level].forward_t(last_encoder_output, false));
            }
        }

        let mut source_flatten = Vec::with_capacity(sources.len());
        let mut spatial_shapes = Vec::with_capacity(sources.len());
        for source in &sources {
            let size = source.size();
            let height = size[2];
            let width = size[3];
            spatial_shapes.push((height, width));
            source_flatten.push(source.flatten(2, -1).transpose(1, 2));
        }
        let source_flatten = Tensor::cat(&source_flatten, 1);

        // Transformers caches these tensors when `anchor_image_size` is fixed.
        // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py#L1392-L1420
        let (anchors, valid_mask) = match (&self.anchors, &self.valid_mask) {
            (Some(anchors), Some(valid_mask)) => {
                (anchors.shallow_clone(), valid_mask.shallow_clone())
            }
            _ => generate_anchors(
                &spatial_shapes,
                source_flatten.device(),
                source_flatten.kind(),
            ),
        };
        let memory = valid_mask.to_kind(source_flatten.kind()) * &source_flatten;
        let output_memory = self.enc_output.forward(&memory);
        let enc_outputs_class = self.enc_score_head.forward(&output_memory);
        let enc_outputs_coord_logits = self.enc_bbox_head.forward(&output_memory) + anchors;

        let (_, topk_ind) =
            enc_outputs_class
                .max_dim(-1, false)
                .0
                .topk(self.config.num_queries, 1, true, true);

        let reference_points_unact = enc_outputs_coord_logits.gather(
            1,
            &topk_ind
                .unsqueeze(-1)
                .repeat([1, 1, enc_outputs_coord_logits.size()[2]]),
            false,
        );

        let target = if let Some(weight_embedding) = &self.weight_embedding {
            weight_embedding.ws.tile([source_flatten.size()[0], 1, 1])
        } else {
            output_memory
                .gather(
                    1,
                    &topk_ind
                        .unsqueeze(-1)
                        .repeat([1, 1, output_memory.size()[2]]),
                    false,
                )
                .detach()
        };
        let init_reference_points = reference_points_unact.detach();

        let decoder_outputs = self.decoder.forward(
            &target,
            &source_flatten,
            &init_reference_points,
            &spatial_shapes,
            bbox_embed,
            class_embed,
        );

        RTDetrV2ObjectDetectionOutput {
            logits: decoder_outputs.intermediate_logits.select(1, -1),
            pred_boxes: decoder_outputs.intermediate_reference_points.select(1, -1),
        }
    }
}

#[derive(Debug)]
struct RTDetrV2ConvEncoder {
    model: RTDetrResNetBackbone,
}

impl RTDetrV2ConvEncoder {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        Self {
            model: RTDetrResNetBackbone::new(&(path / "model"), &config.backbone_config),
        }
    }

    fn intermediate_channel_sizes(&self) -> Vec<i64> {
        self.model.channels()
    }

    fn forward(&self, pixel_values: &Tensor) -> Vec<Tensor> {
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
    fn new(path: &nn::Path<'_>, channels: i64) -> Self {
        Self {
            weight: path.ones_no_train("weight", &[channels]),
            bias: path.zeros_no_train("bias", &[channels]),
            running_mean: path.zeros_no_train("running_mean", &[channels]),
            running_var: path.ones_no_train("running_var", &[channels]),
        }
    }

    fn forward(&self, input: &Tensor) -> Tensor {
        let weight = self.weight.reshape([1, -1, 1, 1]);
        let bias = self.bias.reshape([1, -1, 1, 1]);
        let running_mean = self.running_mean.reshape([1, -1, 1, 1]);
        let running_var = self.running_var.reshape([1, -1, 1, 1]);
        let scale = weight * (running_var + 1e-5).rsqrt();
        input * &scale + bias - running_mean * scale
    }
}

#[derive(Debug)]
struct RTDetrResNetBackbone {
    config: RTDetrResNetConfig,
    embedder: RTDetrResNetEmbeddings,
    encoder: RTDetrResNetEncoder,
}

impl RTDetrResNetBackbone {
    fn new(path: &nn::Path<'_>, config: &RTDetrResNetConfig) -> Self {
        Self {
            config: config.clone(),
            embedder: RTDetrResNetEmbeddings::new(&(path / "embedder"), config),
            encoder: RTDetrResNetEncoder::new(&(path / "encoder"), config),
        }
    }

    fn channels(&self) -> Vec<i64> {
        self.config.channels()
    }

    fn forward(&self, pixel_values: &Tensor) -> Vec<Tensor> {
        let embedding = self.embedder.forward(pixel_values);
        let hidden_states = self.encoder.forward(&embedding);
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
            .map(|(idx, _)| hidden_states[idx].shallow_clone())
            .collect()
    }
}

#[derive(Debug)]
struct RTDetrResNetEmbeddings {
    embedder: Vec<RTDetrResNetConvLayer>,
}

impl RTDetrResNetEmbeddings {
    fn new(path: &nn::Path<'_>, config: &RTDetrResNetConfig) -> Self {
        Self {
            embedder: vec![
                RTDetrResNetConvLayer::new(
                    &(path / "embedder" / 0),
                    config.num_channels,
                    config.embedding_size / 2,
                    3,
                    2,
                    Activation::from_name(&config.hidden_act),
                ),
                RTDetrResNetConvLayer::new(
                    &(path / "embedder" / 1),
                    config.embedding_size / 2,
                    config.embedding_size / 2,
                    3,
                    1,
                    Activation::from_name(&config.hidden_act),
                ),
                RTDetrResNetConvLayer::new(
                    &(path / "embedder" / 2),
                    config.embedding_size / 2,
                    config.embedding_size,
                    3,
                    1,
                    Activation::from_name(&config.hidden_act),
                ),
            ],
        }
    }

    fn forward(&self, pixel_values: &Tensor) -> Tensor {
        let mut hidden_state = pixel_values.shallow_clone();
        for layer in &self.embedder {
            hidden_state = layer.forward(&hidden_state);
        }
        hidden_state.max_pool2d([3, 3], [2, 2], [1, 1], [1, 1], false)
    }
}

#[derive(Debug)]
struct RTDetrResNetEncoder {
    stages: Vec<RTDetrResNetStage>,
}

impl RTDetrResNetEncoder {
    fn new(path: &nn::Path<'_>, config: &RTDetrResNetConfig) -> Self {
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
        ));
        for idx in 1..config.depths.len() {
            stages.push(RTDetrResNetStage::new(
                &(path / "stages" / idx),
                config,
                config.hidden_sizes[idx - 1],
                config.hidden_sizes[idx],
                2,
                config.depths[idx],
            ));
        }
        Self { stages }
    }

    fn forward(&self, input: &Tensor) -> Vec<Tensor> {
        let mut hidden_state = input.shallow_clone();
        let mut hidden_states = Vec::with_capacity(self.stages.len() + 1);
        for stage in &self.stages {
            hidden_states.push(hidden_state.shallow_clone());
            hidden_state = stage.forward(&hidden_state);
        }
        hidden_states.push(hidden_state);
        hidden_states
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
    ) -> Self {
        let mut layers = Vec::with_capacity(depth);
        layers.push(RTDetrResNetBottleNeckLayer::new(
            &(path / "layers" / 0),
            config,
            in_channels,
            out_channels,
            stride,
        ));
        for idx in 1..depth {
            layers.push(RTDetrResNetBottleNeckLayer::new(
                &(path / "layers" / idx),
                config,
                out_channels,
                out_channels,
                1,
            ));
        }
        Self { layers }
    }

    fn forward(&self, input: &Tensor) -> Tensor {
        let mut hidden_state = input.shallow_clone();
        for layer in &self.layers {
            hidden_state = layer.forward(&hidden_state);
        }
        hidden_state
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
    ) -> Self {
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
        Self {
            layer: vec![
                RTDetrResNetConvLayer::new(
                    &(path / "layer" / 0),
                    in_channels,
                    reduces_channels,
                    1,
                    conv1_stride,
                    Activation::from_name(&config.hidden_act),
                ),
                RTDetrResNetConvLayer::new(
                    &(path / "layer" / 1),
                    reduces_channels,
                    reduces_channels,
                    3,
                    conv2_stride,
                    Activation::from_name(&config.hidden_act),
                ),
                RTDetrResNetConvLayer::new(
                    &(path / "layer" / 2),
                    reduces_channels,
                    out_channels,
                    1,
                    1,
                    Activation::None,
                ),
            ],
            shortcut: should_apply_shortcut.then(|| {
                let shortcut_path = if stride == 2 {
                    path / "shortcut" / 1
                } else {
                    path / "shortcut"
                };
                RTDetrResNetShortCut::new(&shortcut_path, in_channels, out_channels)
            }),
            shortcut_avg_pool: stride == 2,
            activation: Activation::from_name(&config.hidden_act),
        }
    }

    fn forward(&self, hidden_state: &Tensor) -> Tensor {
        let residual = if self.shortcut_avg_pool {
            hidden_state.avg_pool2d([2, 2], [2, 2], [0, 0], true, true, None)
        } else {
            hidden_state.shallow_clone()
        };
        let residual = self
            .shortcut
            .as_ref()
            .map(|shortcut| shortcut.forward(&residual))
            .unwrap_or(residual);

        let mut hidden_state = hidden_state.shallow_clone();
        for layer in &self.layer {
            hidden_state = layer.forward(&hidden_state);
        }
        self.activation.apply(hidden_state + residual)
    }
}

#[derive(Debug)]
struct RTDetrResNetShortCut {
    convolution: nn::Conv2D,
    normalization: RTDetrV2FrozenBatchNorm2d,
}

impl RTDetrResNetShortCut {
    fn new(path: &nn::Path<'_>, in_channels: i64, out_channels: i64) -> Self {
        Self {
            convolution: nn::conv2d(
                path / "convolution",
                in_channels,
                out_channels,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            ),
            normalization: RTDetrV2FrozenBatchNorm2d::new(&(path / "normalization"), out_channels),
        }
    }

    fn forward(&self, input: &Tensor) -> Tensor {
        self.normalization.forward(&input.apply(&self.convolution))
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
    ) -> Self {
        Self {
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
            ),
            normalization: RTDetrV2FrozenBatchNorm2d::new(&(path / "normalization"), out_channels),
            activation,
        }
    }

    fn forward(&self, input: &Tensor) -> Tensor {
        self.activation
            .apply(self.normalization.forward(&input.apply(&self.convolution)))
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
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        let stages = config.encoder_in_channels.len() - 1;
        Self {
            config: config.clone(),
            encoder: config
                .encode_proj_layers
                .iter()
                .enumerate()
                .map(|(idx, _)| RTDetrV2AIFILayer::new(&(path / "encoder" / idx), config))
                .collect(),
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
                .collect(),
            fpn_blocks: (0..stages)
                .map(|idx| RTDetrV2CSPRepLayer::new(&(path / "fpn_blocks" / idx), config))
                .collect(),
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
                .collect(),
            pan_blocks: (0..stages)
                .map(|idx| RTDetrV2CSPRepLayer::new(&(path / "pan_blocks" / idx), config))
                .collect(),
        }
    }

    fn forward(&self, mut feature_maps: Vec<Tensor>) -> Vec<Tensor> {
        if self.config.encoder_layers > 0 {
            for (idx, &enc_ind) in self.config.encode_proj_layers.iter().enumerate() {
                feature_maps[enc_ind] = self.encoder[idx].forward(&feature_maps[enc_ind]);
            }
        }

        // Preserve Transformers' AIFI -> top-down FPN -> bottom-up PAN ordering;
        // changing the traversal changes which resolution is fused at each stage.
        // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py#L1112-L1145
        let mut fpn_feature_maps = vec![feature_maps.last().expect("feature map").shallow_clone()];
        for idx in 0..self.lateral_convs.len() {
            let backbone_feature_map =
                feature_maps[self.lateral_convs.len() - idx - 1].shallow_clone();
            let top = self.lateral_convs[idx].forward(fpn_feature_maps.last().expect("fpn"));
            *fpn_feature_maps.last_mut().expect("fpn") = top.shallow_clone();
            let size = top.size();
            let upsampled =
                top.upsample_nearest2d([size[2] * 2, size[3] * 2], None::<f64>, None::<f64>);
            let fused = Tensor::cat(&[upsampled, backbone_feature_map], 1);
            fpn_feature_maps.push(self.fpn_blocks[idx].forward(&fused));
        }
        fpn_feature_maps.reverse();

        let mut pan_feature_maps = vec![fpn_feature_maps[0].shallow_clone()];
        for idx in 0..self.downsample_convs.len() {
            let downsampled =
                self.downsample_convs[idx].forward(pan_feature_maps.last().expect("pan"));
            let fused = Tensor::cat(&[downsampled, fpn_feature_maps[idx + 1].shallow_clone()], 1);
            pan_feature_maps.push(self.pan_blocks[idx].forward(&fused));
        }

        pan_feature_maps
    }
}

#[derive(Debug)]
struct RTDetrV2AIFILayer {
    encoder_hidden_dim: i64,
    position_embedding: RTDetrV2SinePositionEmbedding,
    layers: Vec<RTDetrV2EncoderLayer>,
}

impl RTDetrV2AIFILayer {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        Self {
            encoder_hidden_dim: config.encoder_hidden_dim,
            position_embedding: RTDetrV2SinePositionEmbedding {
                embed_dim: config.encoder_hidden_dim,
                temperature: config.positional_encoding_temperature,
            },
            layers: (0..config.encoder_layers)
                .map(|idx| RTDetrV2EncoderLayer::new(&(path / "layers" / idx), config))
                .collect(),
        }
    }

    fn forward(&self, hidden_states: &Tensor) -> Tensor {
        let size = hidden_states.size();
        let batch_size = size[0];
        let height = size[2];
        let width = size[3];
        let mut hidden_states = hidden_states.flatten(2, -1).permute([0, 2, 1]);
        let pos_embed = self.position_embedding.forward(
            width,
            height,
            hidden_states.device(),
            hidden_states.kind(),
        );
        for layer in &self.layers {
            hidden_states = layer.forward(&hidden_states, Some(&pos_embed));
        }
        hidden_states
            .permute([0, 2, 1])
            .reshape([batch_size, self.encoder_hidden_dim, height, width])
            .contiguous()
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
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        Self {
            normalize_before: config.normalize_before,
            self_attn: RTDetrV2SelfAttention::new(
                &(path / "self_attn"),
                config.encoder_hidden_dim,
                config.encoder_attention_heads,
            ),
            self_attn_layer_norm: layer_norm(
                &(path / "self_attn_layer_norm"),
                config.encoder_hidden_dim,
                config.layer_norm_eps,
            ),
            mlp: RTDetrV2MLP::new(
                path,
                config.encoder_hidden_dim,
                config.encoder_ffn_dim,
                Activation::from_name(&config.encoder_activation_function),
            ),
            final_layer_norm: layer_norm(
                &(path / "final_layer_norm"),
                config.encoder_hidden_dim,
                config.layer_norm_eps,
            ),
        }
    }

    fn forward(&self, hidden_states: &Tensor, position_embeddings: Option<&Tensor>) -> Tensor {
        let residual = hidden_states.shallow_clone();
        let hidden_states = if self.normalize_before {
            self.self_attn_layer_norm.forward(hidden_states)
        } else {
            hidden_states.shallow_clone()
        };
        let hidden_states = self.self_attn.forward(&hidden_states, position_embeddings);
        let hidden_states = residual + hidden_states;
        let hidden_states = if self.normalize_before {
            hidden_states
        } else {
            self.self_attn_layer_norm.forward(&hidden_states)
        };

        let residual = if self.normalize_before {
            self.final_layer_norm.forward(&hidden_states)
        } else {
            hidden_states.shallow_clone()
        };
        let hidden_states = residual.shallow_clone() + self.mlp.forward(&residual);
        if self.normalize_before {
            hidden_states
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
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        Self {
            layers: (0..config.decoder_layers)
                .map(|idx| RTDetrV2DecoderLayer::new(&(path / "layers" / idx), config))
                .collect(),
            query_pos_head: RTDetrV2MLPPredictionHead::new(
                &(path / "query_pos_head"),
                4,
                2 * config.d_model,
                config.d_model,
                2,
            ),
        }
    }

    fn forward(
        &self,
        inputs_embeds: &Tensor,
        encoder_hidden_states: &Tensor,
        reference_points: &Tensor,
        spatial_shapes_list: &[(i64, i64)],
        bbox_embed: &[RTDetrV2MLPPredictionHead],
        class_embed: &[nn::Linear],
    ) -> RTDetrV2DecoderOutput {
        let mut hidden_states = inputs_embeds.shallow_clone();
        let mut reference_points = reference_points.sigmoid();
        let mut intermediate_logits = Vec::with_capacity(self.layers.len());
        let mut intermediate_reference_points = Vec::with_capacity(self.layers.len());

        for (idx, decoder_layer) in self.layers.iter().enumerate() {
            let reference_points_input = reference_points.unsqueeze(2);
            let object_queries_position_embeddings = self.query_pos_head.forward(&reference_points);
            hidden_states = decoder_layer.forward(
                &hidden_states,
                &object_queries_position_embeddings,
                &reference_points_input,
                spatial_shapes_list,
                encoder_hidden_states,
            );

            // Iterative box refinement intentionally detaches only the reference
            // used by the next decoder layer, matching Transformers inference.
            // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr_v2/modeling_rt_detr_v2.py#L626-L636
            let predicted_corners = bbox_embed[idx].forward(&hidden_states);
            let new_reference_points =
                (predicted_corners + inverse_sigmoid(&reference_points)).sigmoid();
            reference_points = new_reference_points.detach();
            intermediate_reference_points.push(new_reference_points);
            intermediate_logits.push(class_embed[idx].forward(&hidden_states));
        }

        RTDetrV2DecoderOutput {
            intermediate_logits: Tensor::stack(&intermediate_logits, 1),
            intermediate_reference_points: Tensor::stack(&intermediate_reference_points, 1),
        }
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
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        Self {
            self_attn: RTDetrV2SelfAttention::new(
                &(path / "self_attn"),
                config.d_model,
                config.decoder_attention_heads,
            ),
            self_attn_layer_norm: layer_norm(
                &(path / "self_attn_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            ),
            encoder_attn: RTDetrV2MultiscaleDeformableAttention::new(
                &(path / "encoder_attn"),
                config,
            ),
            encoder_attn_layer_norm: layer_norm(
                &(path / "encoder_attn_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            ),
            mlp: RTDetrV2MLP::new(
                path,
                config.d_model,
                config.decoder_ffn_dim,
                Activation::from_name(&config.decoder_activation_function),
            ),
            final_layer_norm: layer_norm(
                &(path / "final_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            ),
        }
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        object_queries_position_embeddings: &Tensor,
        reference_points: &Tensor,
        spatial_shapes_list: &[(i64, i64)],
        encoder_hidden_states: &Tensor,
    ) -> Tensor {
        let residual = hidden_states.shallow_clone();
        let hidden_states = self
            .self_attn
            .forward(hidden_states, Some(object_queries_position_embeddings));
        let hidden_states = self
            .self_attn_layer_norm
            .forward(&(residual + hidden_states));

        let residual = hidden_states.shallow_clone();
        let hidden_states = self.encoder_attn.forward(
            &hidden_states,
            encoder_hidden_states,
            object_queries_position_embeddings,
            reference_points,
            spatial_shapes_list,
        );
        let hidden_states = self
            .encoder_attn_layer_norm
            .forward(&(residual + hidden_states));
        let residual = hidden_states.shallow_clone();
        self.final_layer_norm
            .forward(&(residual.shallow_clone() + self.mlp.forward(&residual)))
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
    fn new(path: &nn::Path<'_>, hidden_size: i64, num_heads: i64) -> Self {
        let head_dim = hidden_size / num_heads;
        Self {
            head_dim,
            num_heads,
            scaling: (head_dim as f64).powf(-0.5),
            k_proj: nn::linear(
                path / "k_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            ),
            v_proj: nn::linear(
                path / "v_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            ),
            q_proj: nn::linear(
                path / "q_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            ),
            // The model was exported by Transformers 4.49, before this field was
            // renamed from `out_proj` to `o_proj`; retain its checkpoint path.
            o_proj: nn::linear(
                path / "out_proj",
                hidden_size,
                hidden_size,
                Default::default(),
            ),
        }
    }

    fn forward(&self, hidden_states: &Tensor, position_embeddings: Option<&Tensor>) -> Tensor {
        let size = hidden_states.size();
        let batch_size = size[0];
        let sequence_length = size[1];
        let query_key_input = position_embeddings
            .map(|position_embeddings| hidden_states + position_embeddings)
            .unwrap_or_else(|| hidden_states.shallow_clone());
        let query_states = self
            .q_proj
            .forward(&query_key_input)
            .view([batch_size, sequence_length, self.num_heads, self.head_dim])
            .transpose(1, 2);
        let key_states = self
            .k_proj
            .forward(&query_key_input)
            .view([batch_size, sequence_length, self.num_heads, self.head_dim])
            .transpose(1, 2);
        let value_states = self
            .v_proj
            .forward(hidden_states)
            .view([batch_size, sequence_length, self.num_heads, self.head_dim])
            .transpose(1, 2);
        let attn_weights = query_states.matmul(&key_states.transpose(2, 3)) * self.scaling;
        let attn_weights = attn_weights.softmax(-1, None::<Kind>);
        let attn_output = attn_weights
            .matmul(&value_states)
            .transpose(1, 2)
            .contiguous();
        self.o_proj.forward(&attn_output.reshape([
            batch_size,
            sequence_length,
            self.num_heads * self.head_dim,
        ]))
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
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        let n_levels = config.decoder_n_levels;
        let n_points = config.decoder_n_points;
        let n_points_scale_values = (0..n_levels)
            .flat_map(|_| (0..n_points).map(move |_| 1.0f32 / n_points as f32))
            .collect::<Vec<_>>();
        let mut n_points_scale = path.zeros_no_train("n_points_scale", &[n_levels * n_points]);
        n_points_scale.copy_(&Tensor::from_slice(&n_points_scale_values));
        Self {
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
            ),
            attention_weights: nn::linear(
                path / "attention_weights",
                config.d_model,
                config.decoder_attention_heads * n_levels * n_points,
                Default::default(),
            ),
            value_proj: nn::linear(
                path / "value_proj",
                config.d_model,
                config.d_model,
                Default::default(),
            ),
            output_proj: nn::linear(
                path / "output_proj",
                config.d_model,
                config.d_model,
                Default::default(),
            ),
        }
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        encoder_hidden_states: &Tensor,
        position_embeddings: &Tensor,
        reference_points: &Tensor,
        spatial_shapes_list: &[(i64, i64)],
    ) -> Tensor {
        let hidden_states = hidden_states + position_embeddings;
        let batch_size = hidden_states.size()[0];
        let num_queries = hidden_states.size()[1];
        let sequence_length = encoder_hidden_states.size()[1];
        let dim_per_head = self.d_model / self.n_heads;

        let value = self.value_proj.forward(encoder_hidden_states).view([
            batch_size,
            sequence_length,
            self.n_heads,
            dim_per_head,
        ]);
        let sampling_offsets = self.sampling_offsets.forward(&hidden_states).view([
            batch_size,
            num_queries,
            self.n_heads,
            self.n_levels * self.n_points,
            2,
        ]);
        let attention_weights = self
            .attention_weights
            .forward(&hidden_states)
            .view([
                batch_size,
                num_queries,
                self.n_heads,
                self.n_levels * self.n_points,
            ])
            .softmax(-1, None::<Kind>);

        let sampling_locations = if reference_points.size().last().copied() == Some(4) {
            let scale = self.n_points_scale.to_kind(hidden_states.kind()).view([
                1,
                1,
                1,
                self.n_levels * self.n_points,
                1,
            ]);
            let ref_xy = reference_points.slice(-1, 0, 2, 1).unsqueeze(2);
            let ref_wh = reference_points.slice(-1, 2, 4, 1).unsqueeze(2);
            ref_xy + sampling_offsets * scale * ref_wh * self.offset_scale
        } else {
            let spatial_shapes = spatial_shapes_tensor(spatial_shapes_list, value.device())
                .to_kind(hidden_states.kind());
            let normalizer =
                Tensor::stack(&[spatial_shapes.i((.., 1)), spatial_shapes.i((.., 0))], -1);
            reference_points.unsqueeze(2).unsqueeze(4)
                + sampling_offsets
                    / normalizer
                        .unsqueeze(0)
                        .unsqueeze(0)
                        .unsqueeze(0)
                        .unsqueeze(4)
        };

        let output = multiscale_deformable_attention_v2(
            &value,
            spatial_shapes_list,
            &sampling_locations,
            &attention_weights,
            self.n_points as usize,
            &self.method,
        );
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
    ) -> Self {
        Self {
            fc1: nn::linear(
                path / "fc1",
                hidden_size,
                intermediate_size,
                Default::default(),
            ),
            fc2: nn::linear(
                path / "fc2",
                intermediate_size,
                hidden_size,
                Default::default(),
            ),
            activation,
        }
    }

    fn forward(&self, hidden_states: &Tensor) -> Tensor {
        let hidden_states = self.activation.apply(self.fc1.forward(hidden_states));
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
    ) -> Self {
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
            .collect();
        Self { layers }
    }

    fn forward(&self, x: &Tensor) -> Tensor {
        let mut x = x.shallow_clone();
        for (idx, layer) in self.layers.iter().enumerate() {
            x = layer.forward(&x);
            if idx + 1 != self.layers.len() {
                x = x.relu();
            }
        }
        x
    }
}

#[derive(Debug)]
struct RTDetrV2CSPRepLayer {
    conv1: RTDetrV2ConvNormLayer,
    conv2: RTDetrV2ConvNormLayer,
    bottlenecks: Vec<RTDetrV2RepVggBlock>,
}

impl RTDetrV2CSPRepLayer {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        let in_channels = config.encoder_hidden_dim * 2;
        let out_channels = config.encoder_hidden_dim;
        let hidden_channels = (out_channels as f64 * config.hidden_expansion) as i64;
        Self {
            conv1: RTDetrV2ConvNormLayer::new(
                &(path / "conv1"),
                in_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::from_name(&config.activation_function),
                config.batch_norm_eps,
            ),
            conv2: RTDetrV2ConvNormLayer::new(
                &(path / "conv2"),
                in_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::from_name(&config.activation_function),
                config.batch_norm_eps,
            ),
            bottlenecks: (0..3)
                .map(|idx| RTDetrV2RepVggBlock::new(&(path / "bottlenecks" / idx), config))
                .collect(),
        }
    }

    fn forward(&self, hidden_state: &Tensor) -> Tensor {
        let mut hidden_state_1 = self.conv1.forward(hidden_state);
        for bottleneck in &self.bottlenecks {
            hidden_state_1 = bottleneck.forward(&hidden_state_1);
        }
        hidden_state_1 + self.conv2.forward(hidden_state)
    }
}

#[derive(Debug)]
struct RTDetrV2RepVggBlock {
    conv1: RTDetrV2ConvNormLayer,
    conv2: RTDetrV2ConvNormLayer,
    activation: Activation,
}

impl RTDetrV2RepVggBlock {
    fn new(path: &nn::Path<'_>, config: &RTDetrV2Config) -> Self {
        let hidden_channels = (config.encoder_hidden_dim as f64 * config.hidden_expansion) as i64;
        Self {
            conv1: RTDetrV2ConvNormLayer::new(
                &(path / "conv1"),
                hidden_channels,
                hidden_channels,
                3,
                1,
                Some(1),
                Activation::None,
                config.batch_norm_eps,
            ),
            conv2: RTDetrV2ConvNormLayer::new(
                &(path / "conv2"),
                hidden_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::None,
                config.batch_norm_eps,
            ),
            activation: Activation::from_name(&config.activation_function),
        }
    }

    fn forward(&self, x: &Tensor) -> Tensor {
        self.activation
            .apply(self.conv1.forward(x) + self.conv2.forward(x))
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
    ) -> Self {
        Self {
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
            ),
            norm: nn::batch_norm2d(
                path / "norm",
                out_channels,
                nn::BatchNormConfig {
                    eps,
                    ..Default::default()
                },
            ),
            activation,
        }
    }

    fn forward(&self, hidden_state: &Tensor) -> Tensor {
        self.activation
            .apply(hidden_state.apply(&self.conv).apply_t(&self.norm, false))
    }
}

#[derive(Debug)]
struct RTDetrV2SinePositionEmbedding {
    embed_dim: i64,
    temperature: f64,
}

impl RTDetrV2SinePositionEmbedding {
    fn forward(&self, width: i64, height: i64, device: Device, kind: Kind) -> Tensor {
        let grid_w = Tensor::arange(width, (kind, device));
        let grid_h = Tensor::arange(height, (kind, device));
        let mesh = Tensor::meshgrid_indexing(&[grid_w, grid_h], "xy");
        let grid_w = mesh[0].flatten(0, -1).unsqueeze(-1);
        let grid_h = mesh[1].flatten(0, -1).unsqueeze(-1);
        let pos_dim = self.embed_dim / 4;
        let omega = Tensor::arange(pos_dim, (kind, device)) / pos_dim as f64;
        let omega = (self.temperature.ln() * &omega).exp().reciprocal();
        let out_w = grid_w.matmul(&omega.unsqueeze(0));
        let out_h = grid_h.matmul(&omega.unsqueeze(0));
        Tensor::cat(&[out_h.sin(), out_h.cos(), out_w.sin(), out_w.cos()], 1).unsqueeze(0)
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

    fn apply(self, x: Tensor) -> Tensor {
        match self {
            Self::None => x,
            Self::Relu => x.relu(),
            Self::Silu => x.silu(),
            Self::Gelu => x.gelu("none"),
        }
    }
}

fn layer_norm(path: &nn::Path<'_>, hidden_dim: i64, eps: f64) -> nn::LayerNorm {
    nn::layer_norm(
        path,
        vec![hidden_dim],
        nn::LayerNormConfig {
            eps,
            ..Default::default()
        },
    )
}

fn multiscale_deformable_attention_v2(
    value: &Tensor,
    spatial_shapes_list: &[(i64, i64)],
    sampling_locations: &Tensor,
    attention_weights: &Tensor,
    n_points: usize,
    method: &str,
) -> Tensor {
    let size = value.size();
    let batch_size = size[0];
    let num_heads = size[2];
    let hidden_dim = size[3];
    let num_queries = sampling_locations.size()[1];
    let split_sizes = spatial_shapes_list
        .iter()
        .map(|(height, width)| height * width)
        .collect::<Vec<_>>();
    let value_list = value.split_with_sizes(split_sizes, 1);
    let sampling_grids = match method {
        "default" => sampling_locations * 2.0 - 1.0,
        "discrete" => sampling_locations.shallow_clone(),
        _ => panic!("unsupported RT-DETR-v2 decoder method: {method}"),
    };
    let sampling_grids = sampling_grids.permute([0, 2, 1, 3, 4]).flatten(0, 1);
    let point_splits = vec![n_points as i64; spatial_shapes_list.len()];
    let sampling_grid_list = sampling_grids.split_with_sizes(point_splits, -2);

    let mut sampling_values = Vec::with_capacity(spatial_shapes_list.len());
    for (level, (height, width)) in spatial_shapes_list.iter().copied().enumerate() {
        let value_l = value_list[level].flatten(2, -1).transpose(1, 2).reshape([
            batch_size * num_heads,
            hidden_dim,
            height,
            width,
        ]);
        let sampling_grid = &sampling_grid_list[level];
        let sampling_value = if method == "default" {
            value_l.grid_sampler_2d(sampling_grid, 0, 0, false)
        } else {
            let scale = Tensor::from_slice(&[width, height])
                .to_device(value.device())
                .view([1, 1, 1, 2]);
            let sampling_coord = (sampling_grid * scale + 0.5).to_kind(Kind::Int64);
            let x = sampling_coord.i((.., .., .., 0)).clamp(0, width - 1);
            let y = sampling_coord.i((.., .., .., 1)).clamp(0, height - 1);
            let index = (y * width + x).flatten(1, -1);
            value_l
                .flatten(2, -1)
                .gather(2, &index.unsqueeze(1).repeat([1, hidden_dim, 1]), false)
                .view([
                    batch_size * num_heads,
                    hidden_dim,
                    num_queries,
                    n_points as i64,
                ])
        };
        sampling_values.push(sampling_value);
    }

    let attention_weights = attention_weights.permute([0, 2, 1, 3]).reshape([
        batch_size * num_heads,
        1,
        num_queries,
        -1,
    ]);
    (Tensor::cat(&sampling_values, -1) * attention_weights)
        .sum_dim_intlist(&[-1i64][..], false, None::<Kind>)
        .view([batch_size, num_heads * hidden_dim, num_queries])
        .transpose(1, 2)
        .contiguous()
}

fn generate_anchors(spatial_shapes: &[(i64, i64)], device: Device, kind: Kind) -> (Tensor, Tensor) {
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
    let anchors = Tensor::from_slice(&anchors)
        .view([1, total, 4])
        .to_device(device)
        .to_kind(kind);
    let valid_mask = Tensor::from_slice(&valid)
        .view([1, total, 1])
        .to_device(device);
    let one_minus_anchors = anchors.ones_like() - &anchors;
    let logit = (&anchors / one_minus_anchors).log();
    let finfo_max = match kind {
        Kind::Half => 65_504.0,
        Kind::Double => f64::MAX,
        // The exact bfloat16 maximum rounds past it on conversion and
        // Torch refuses the fill, so stay just below.
        Kind::BFloat16 => 3.38e38,
        _ => f32::MAX as f64,
    };
    let max = Tensor::full([1, total, 4], finfo_max, (kind, device));
    let anchors = logit.where_self(&valid_mask.expand([1, total, 4], true), &max);
    (anchors, valid_mask)
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

fn inverse_sigmoid(x: &Tensor) -> Tensor {
    let x = x.clamp(0.0, 1.0);
    let x1 = x.clamp_min(1e-5);
    let x2 = (x.ones_like() - &x).clamp_min(1e-5);
    (x1 / x2).log()
}

fn spatial_shapes_tensor(spatial_shapes: &[(i64, i64)], device: Device) -> Tensor {
    let values = spatial_shapes
        .iter()
        .flat_map(|(h, w)| [*h, *w])
        .collect::<Vec<_>>();
    Tensor::from_slice(&values)
        .view([spatial_shapes.len() as i64, 2])
        .to_device(device)
}
