//! Inference-only port of Transformers' PP-DocLayout-v3 detector.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/modeling_pp_doclayout_v3.py

use crate::nn::Apply;
use std::path::Path;

use crate::nn::{self, Module};
use anyhow::Result;
use tch::{Device, IndexOp, Kind, Tensor};

use super::config::{HGNetV2Config, PPDocLayoutV3Config};

#[derive(Debug)]
pub struct Output {
    pub logits: Tensor,
    pub pred_boxes: Tensor,
    pub order_logits: Tensor,
    pub out_masks: Tensor,
}

#[derive(Debug)]
pub struct Model {
    vs: nn::VarStore,
    model: PPDocLayoutV3Model,
}

impl Model {
    pub fn new(config: PPDocLayoutV3Config, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let model = PPDocLayoutV3Model::new(&(&vs.root() / "model"), &config)?;
        Ok(Self { vs, model })
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub fn forward(&self, pixel_values: &Tensor) -> tch::Result<Output> {
        let outputs = self
            .model
            .forward(&pixel_values.f_to_kind(self.vs.kind())?)?;
        let pred_boxes = outputs.intermediate_reference_points.f_select(1, -1)?;
        let logits = outputs.intermediate_logits.f_select(1, -1)?;
        let order_logits = outputs.out_order_logits.f_select(1, -1)?;
        let out_masks = outputs.out_masks.f_select(1, -1)?;
        Ok(Output {
            logits,
            pred_boxes,
            order_logits,
            out_masks,
        })
    }
}

#[derive(Debug)]
struct PPDocLayoutV3ModelOutput {
    intermediate_logits: Tensor,
    intermediate_reference_points: Tensor,
    out_order_logits: Tensor,
    out_masks: Tensor,
}

#[derive(Debug)]
struct PPDocLayoutV3Model {
    config: PPDocLayoutV3Config,
    anchor_cache: Option<AnchorCache>,
    backbone: HGNetV2Backbone,
    encoder_input_proj: Vec<ConvBnSeq>,
    encoder: PPDocLayoutV3HybridEncoder,
    #[allow(dead_code)]
    denoising_class_embed: nn::Embedding,
    weight_embedding: Option<nn::Embedding>,
    enc_output: LinearNormSeq,
    enc_score_head: nn::Linear,
    enc_bbox_head: PPDocLayoutV3MLPPredictionHead,
    decoder_input_proj: Vec<ConvBnSeq>,
    decoder: PPDocLayoutV3Decoder,
    decoder_order_head: Vec<nn::Linear>,
    decoder_global_pointer: PPDocLayoutV3GlobalPointer,
    decoder_norm: nn::LayerNorm,
    mask_query_head: PPDocLayoutV3MLPPredictionHead,
}

impl PPDocLayoutV3Model {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        let anchor_cache = config
            .anchor_image_size
            .as_deref()
            .and_then(|image_size| {
                fixed_spatial_shapes(image_size, &config.feat_strides, config.num_feature_levels)
            })
            .map(|spatial_shapes| -> tch::Result<AnchorCache> {
                let (anchors, valid_mask) =
                    generate_anchors(&spatial_shapes, path.device(), path.kind())?;
                Ok(AnchorCache {
                    spatial_shapes,
                    anchors,
                    valid_mask,
                })
            })
            .transpose()?;
        let backbone =
            HGNetV2Backbone::new(&(path / "backbone" / "model"), &config.backbone_config)?;
        let intermediate_channel_sizes = config.backbone_config.channels();

        let encoder_input_proj = intermediate_channel_sizes
            .iter()
            .skip(1)
            .enumerate()
            .map(|(idx, &in_channels)| {
                ConvBnSeq::new(
                    &(path / "encoder_input_proj" / idx),
                    in_channels,
                    config.encoder_hidden_dim,
                    1,
                    1,
                    0,
                    1,
                    config.batch_norm_eps,
                )
            })
            .collect::<Result<Vec<_>>>()?;

        let encoder = PPDocLayoutV3HybridEncoder::new(&(path / "encoder"), config)?;
        let denoising_class_embed = nn::embedding(
            path / "denoising_class_embed",
            config.num_labels(),
            config.d_model,
            Default::default(),
        )?;
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

        let enc_output = LinearNormSeq::new(
            &(path / "enc_output"),
            config.d_model,
            config.layer_norm_eps,
        )?;
        let enc_score_head = nn::linear(
            path / "enc_score_head",
            config.d_model,
            config.num_labels(),
            Default::default(),
        )?;
        let enc_bbox_head = PPDocLayoutV3MLPPredictionHead::new(
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
            decoder_input_proj.push(ConvBnSeq::new(
                &(path / "decoder_input_proj" / idx),
                channels,
                config.d_model,
                1,
                1,
                0,
                1,
                config.batch_norm_eps,
            )?);
        }
        for idx in decoder_input_proj.len()..config.num_feature_levels {
            decoder_input_proj.push(ConvBnSeq::new(
                &(path / "decoder_input_proj" / idx),
                in_channels,
                config.d_model,
                3,
                2,
                1,
                1,
                config.batch_norm_eps,
            )?);
            in_channels = config.d_model;
        }

        let decoder = PPDocLayoutV3Decoder::new(&(path / "decoder"), config)?;
        let decoder_order_head = (0..config.decoder_layers)
            .map(|idx| {
                nn::linear(
                    path / "decoder_order_head" / idx,
                    config.d_model,
                    config.d_model,
                    Default::default(),
                )
            })
            .collect::<tch::Result<Vec<_>>>()?;
        let decoder_global_pointer =
            PPDocLayoutV3GlobalPointer::new(&(path / "decoder_global_pointer"), config)?;
        let decoder_norm = nn::layer_norm(
            path / "decoder_norm",
            vec![config.d_model],
            nn::LayerNormConfig {
                eps: config.layer_norm_eps,
                ..Default::default()
            },
        )?;
        let mask_query_head = PPDocLayoutV3MLPPredictionHead::new(
            &(path / "mask_query_head"),
            config.d_model,
            config.d_model,
            config.num_prototypes,
            3,
        )?;

        Ok(Self {
            config: config.clone(),
            anchor_cache,
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
            decoder_order_head,
            decoder_global_pointer,
            decoder_norm,
            mask_query_head,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<PPDocLayoutV3ModelOutput> {
        let features = self.backbone.forward(pixel_values)?;
        let x4_feat = vec![features[0].f_alias()?];
        let proj_feats = features
            .iter()
            .skip(1)
            .enumerate()
            .map(|(level, source)| self.encoder_input_proj[level].forward(source))
            .collect::<tch::Result<Vec<_>>>()?;
        let encoder_outputs = self.encoder.forward(proj_feats, &x4_feat)?;

        let mut sources = Vec::with_capacity(self.config.num_feature_levels);
        for (level, source) in encoder_outputs.last_hidden_state.iter().enumerate() {
            sources.push(self.decoder_input_proj[level].forward(source)?);
        }
        if self.config.num_feature_levels > sources.len() {
            let base_len = sources.len();
            sources.push(
                self.decoder_input_proj[base_len].forward(
                    encoder_outputs
                        .last_hidden_state
                        .last()
                        .expect("encoder output"),
                )?,
            );
            for idx in base_len + 1..self.config.num_feature_levels {
                sources.push(
                    self.decoder_input_proj[idx].forward(
                        encoder_outputs
                            .last_hidden_state
                            .last()
                            .expect("encoder output"),
                    )?,
                );
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
        // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/modeling_pp_doclayout_v3.py#L1611-L1640
        let (anchors, valid_mask) = self
            .anchor_cache
            .as_ref()
            .filter(|cache| cache.spatial_shapes == spatial_shapes)
            .map(|cache| -> tch::Result<(Tensor, Tensor)> {
                Ok((cache.anchors.f_alias()?, cache.valid_mask.f_alias()?))
            })
            .unwrap_or_else(|| {
                generate_anchors(
                    &spatial_shapes,
                    source_flatten.device(),
                    source_flatten.kind(),
                )
            })?;

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
        let target = output_memory.f_gather(
            1,
            &topk_ind
                .f_unsqueeze(-1)?
                .f_repeat([1, 1, output_memory.size()[2]])?,
            false,
        )?;
        let out_query = self.decoder_norm.forward(&target)?;
        let mask_query_embed = self.mask_query_head.forward(&out_query)?;
        let target = if let Some(weight_embedding) = &self.weight_embedding {
            weight_embedding
                .ws
                .f_unsqueeze(0)?
                .f_repeat([source_flatten.size()[0], 1, 1])?
        } else {
            target.f_detach()?
        };

        let mut init_reference_points = reference_points_unact;
        if self.config.mask_enhanced {
            let size = encoder_outputs.mask_feat.size();
            let enc_out_masks = mask_query_embed
                .f_bmm(&encoder_outputs.mask_feat.f_flatten(2, -1)?)?
                .f_reshape([size[0], mask_query_embed.size()[1], size[2], size[3]])?;
            let reference_points =
                mask_to_box_coordinate(&enc_out_masks.f_gt(0.0)?, init_reference_points.kind())?;
            init_reference_points = inverse_sigmoid(&reference_points)?;
        }
        init_reference_points = init_reference_points.f_detach()?;

        let spatial_shapes_tensor =
            spatial_shapes_tensor(&spatial_shapes, source_flatten.device())?;
        let level_start_index = level_start_index_tensor(&spatial_shapes, source_flatten.device())?;

        let decoder_outputs = self.decoder.forward(DecoderForwardArgs {
            inputs_embeds: &target,
            encoder_hidden_states: &source_flatten,
            reference_points: &init_reference_points,
            spatial_shapes: &spatial_shapes_tensor,
            spatial_shapes_list: &spatial_shapes,
            level_start_index: &level_start_index,
            order_head: &self.decoder_order_head,
            bbox_embed: &self.enc_bbox_head,
            class_embed: &self.enc_score_head,
            global_pointer: &self.decoder_global_pointer,
            mask_query_head: &self.mask_query_head,
            norm: &self.decoder_norm,
            mask_feat: &encoder_outputs.mask_feat,
        })?;

        Ok(PPDocLayoutV3ModelOutput {
            intermediate_logits: decoder_outputs.intermediate_logits,
            intermediate_reference_points: decoder_outputs.intermediate_reference_points,
            out_order_logits: decoder_outputs.decoder_out_order_logits,
            out_masks: decoder_outputs.decoder_out_masks,
        })
    }
}

#[derive(Debug)]
struct AnchorCache {
    spatial_shapes: Vec<(i64, i64)>,
    anchors: Tensor,
    valid_mask: Tensor,
}

// https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/hgnet_v2/modeling_hgnet_v2.py#L57-L418
#[derive(Debug)]
struct HGNetV2Backbone {
    config: HGNetV2Config,
    embedder: HGNetV2Embeddings,
    encoder: HGNetV2Encoder,
}

impl HGNetV2Backbone {
    fn new(path: &nn::Path<'_>, config: &HGNetV2Config) -> Result<Self> {
        let embedder = HGNetV2Embeddings::new(&(path / "embedder"), config)?;
        let encoder = HGNetV2Encoder::new(&(path / "encoder"), config)?;
        Ok(Self {
            config: config.clone(),
            embedder,
            encoder,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Vec<Tensor>> {
        let embedding = self.embedder.forward(pixel_values)?;
        let hidden_states = self.encoder.forward(&embedding)?;
        let mut feature_maps = Vec::new();
        for feature in &self.config.out_features {
            let idx = match feature.as_str() {
                "stem" => 0,
                "stage1" => 1,
                "stage2" => 2,
                "stage3" => 3,
                "stage4" => 4,
                _ => continue,
            };
            feature_maps.push(hidden_states[idx].f_alias()?);
        }
        Ok(feature_maps)
    }
}

#[derive(Debug)]
struct HGNetV2Embeddings {
    stem1: HGNetV2ConvLayer,
    stem2a: HGNetV2ConvLayer,
    stem2b: HGNetV2ConvLayer,
    stem3: HGNetV2ConvLayer,
    stem4: HGNetV2ConvLayer,
}

impl HGNetV2Embeddings {
    fn new(path: &nn::Path<'_>, config: &HGNetV2Config) -> Result<Self> {
        let act = Activation::from_name(&config.hidden_act);
        Ok(Self {
            stem1: HGNetV2ConvLayer::new(
                &(path / "stem1"),
                config.stem_channels[0],
                config.stem_channels[1],
                3,
                config.stem_strides[0],
                1,
                act,
                config.use_learnable_affine_block,
            )?,
            stem2a: HGNetV2ConvLayer::new(
                &(path / "stem2a"),
                config.stem_channels[1],
                config.stem_channels[1] / 2,
                2,
                config.stem_strides[1],
                1,
                act,
                config.use_learnable_affine_block,
            )?,
            stem2b: HGNetV2ConvLayer::new(
                &(path / "stem2b"),
                config.stem_channels[1] / 2,
                config.stem_channels[1],
                2,
                config.stem_strides[2],
                1,
                act,
                config.use_learnable_affine_block,
            )?,
            stem3: HGNetV2ConvLayer::new(
                &(path / "stem3"),
                config.stem_channels[1] * 2,
                config.stem_channels[1],
                3,
                config.stem_strides[3],
                1,
                act,
                config.use_learnable_affine_block,
            )?,
            stem4: HGNetV2ConvLayer::new(
                &(path / "stem4"),
                config.stem_channels[1],
                config.stem_channels[2],
                1,
                config.stem_strides[4],
                1,
                act,
                config.use_learnable_affine_block,
            )?,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> tch::Result<Tensor> {
        let embedding = self
            .stem1
            .forward(pixel_values)?
            .f_constant_pad_nd([0, 1, 0, 1])?;
        let stem_2a = self
            .stem2a
            .forward(&embedding)?
            .f_constant_pad_nd([0, 1, 0, 1])?;
        let stem_2a = self.stem2b.forward(&stem_2a)?;
        let pooled = embedding.f_max_pool2d([2, 2], [1, 1], [0, 0], [1, 1], true)?;
        let embedding = Tensor::f_cat(&[pooled, stem_2a], 1)?;
        let embedding = self.stem3.forward(&embedding)?;
        self.stem4.forward(&embedding)
    }
}

#[derive(Debug)]
struct HGNetV2Encoder {
    stages: Vec<HGNetV2Stage>,
}

impl HGNetV2Encoder {
    fn new(path: &nn::Path<'_>, config: &HGNetV2Config) -> Result<Self> {
        let stages = (0..config.stage_in_channels.len())
            .map(|idx| HGNetV2Stage::new(&(path / "stages" / idx), config, idx))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { stages })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Vec<Tensor>> {
        let mut hidden_states = Vec::with_capacity(self.stages.len() + 1);
        let mut hidden_state = hidden_state.f_alias()?;
        for stage in &self.stages {
            hidden_states.push(hidden_state.f_alias()?);
            hidden_state = stage.forward(&hidden_state)?;
        }
        hidden_states.push(hidden_state);
        Ok(hidden_states)
    }
}

#[derive(Debug)]
struct HGNetV2Stage {
    downsample: Option<HGNetV2ConvLayer>,
    blocks: Vec<HGNetV2BasicLayer>,
}

impl HGNetV2Stage {
    fn new(path: &nn::Path<'_>, config: &HGNetV2Config, stage_index: usize) -> Result<Self> {
        let in_channels = config.stage_in_channels[stage_index];
        let out_channels = config.stage_out_channels[stage_index];
        let downsample = config.stage_downsample[stage_index]
            .then(|| {
                HGNetV2ConvLayer::new(
                    &(path / "downsample"),
                    in_channels,
                    in_channels,
                    3,
                    config.stage_downsample_strides[stage_index],
                    in_channels,
                    Activation::None,
                    false,
                )
            })
            .transpose()?;
        let blocks = (0..config.stage_num_blocks[stage_index])
            .map(|idx| {
                HGNetV2BasicLayer::new(
                    &(path / "blocks" / idx),
                    if idx == 0 { in_channels } else { out_channels },
                    config.stage_mid_channels[stage_index],
                    out_channels,
                    config.stage_numb_of_layers[stage_index],
                    idx != 0,
                    config.stage_kernel_size[stage_index],
                    config.stage_light_block[stage_index],
                    config.use_learnable_affine_block,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { downsample, blocks })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        let mut hidden_state = self
            .downsample
            .as_ref()
            .map(|downsample| downsample.forward(hidden_state))
            .unwrap_or_else(|| hidden_state.f_alias())?;
        for block in &self.blocks {
            hidden_state = block.forward(&hidden_state)?;
        }
        Ok(hidden_state)
    }
}

#[derive(Debug)]
struct HGNetV2BasicLayer {
    residual: bool,
    layers: Vec<HGNetV2Block>,
    aggregation: Vec<HGNetV2ConvLayer>,
}

impl HGNetV2BasicLayer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        middle_channels: i64,
        out_channels: i64,
        layer_num: usize,
        residual: bool,
        kernel_size: i64,
        light_block: bool,
        use_learnable_affine_block: bool,
    ) -> Result<Self> {
        let layers = (0..layer_num)
            .map(|idx| {
                let in_c = if idx == 0 {
                    in_channels
                } else {
                    middle_channels
                };
                if light_block {
                    Ok(HGNetV2Block::Light(Box::new(HGNetV2ConvLayerLight::new(
                        &(path / "layers" / idx),
                        in_c,
                        middle_channels,
                        kernel_size,
                        use_learnable_affine_block,
                    )?)))
                } else {
                    Ok(HGNetV2Block::Conv(Box::new(HGNetV2ConvLayer::new(
                        &(path / "layers" / idx),
                        in_c,
                        middle_channels,
                        kernel_size,
                        1,
                        1,
                        Activation::Relu,
                        use_learnable_affine_block,
                    )?)))
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let total_channels = in_channels + layer_num as i64 * middle_channels;
        let aggregation = vec![
            HGNetV2ConvLayer::new(
                &(path / "aggregation" / 0),
                total_channels,
                out_channels / 2,
                1,
                1,
                1,
                Activation::Relu,
                use_learnable_affine_block,
            )?,
            HGNetV2ConvLayer::new(
                &(path / "aggregation" / 1),
                out_channels / 2,
                out_channels,
                1,
                1,
                1,
                Activation::Relu,
                use_learnable_affine_block,
            )?,
        ];
        Ok(Self {
            residual,
            layers,
            aggregation,
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        let identity = hidden_state.f_alias()?;
        let mut output = vec![hidden_state.f_alias()?];
        let mut hidden_state = hidden_state.f_alias()?;
        for layer in &self.layers {
            hidden_state = layer.forward(&hidden_state)?;
            output.push(hidden_state.f_alias()?);
        }
        let mut hidden_state = Tensor::f_cat(&output, 1)?;
        for layer in &self.aggregation {
            hidden_state = layer.forward(&hidden_state)?;
        }
        if self.residual {
            hidden_state.f_add(&identity)
        } else {
            Ok(hidden_state)
        }
    }
}

#[derive(Debug)]
enum HGNetV2Block {
    Conv(Box<HGNetV2ConvLayer>),
    Light(Box<HGNetV2ConvLayerLight>),
}

impl HGNetV2Block {
    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        match self {
            Self::Conv(layer) => layer.forward(hidden_state),
            Self::Light(layer) => layer.forward(hidden_state),
        }
    }
}

#[derive(Debug)]
struct HGNetV2ConvLayerLight {
    conv1: HGNetV2ConvLayer,
    conv2: HGNetV2ConvLayer,
}

impl HGNetV2ConvLayerLight {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        use_learnable_affine_block: bool,
    ) -> Result<Self> {
        Ok(Self {
            conv1: HGNetV2ConvLayer::new(
                &(path / "conv1"),
                in_channels,
                out_channels,
                1,
                1,
                1,
                Activation::None,
                use_learnable_affine_block,
            )?,
            conv2: HGNetV2ConvLayer::new(
                &(path / "conv2"),
                out_channels,
                out_channels,
                kernel_size,
                1,
                out_channels,
                Activation::Relu,
                use_learnable_affine_block,
            )?,
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        let hidden_state = self.conv1.forward(hidden_state)?;
        self.conv2.forward(&hidden_state)
    }
}

#[derive(Debug)]
struct HGNetV2ConvLayer {
    convolution: nn::Conv2D,
    normalization: nn::BatchNorm,
    activation: Activation,
    lab: Option<HGNetV2LearnableAffineBlock>,
}

impl HGNetV2ConvLayer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        groups: i64,
        activation: Activation,
        use_learnable_affine_block: bool,
    ) -> Result<Self> {
        let convolution = nn::conv2d(
            path / "convolution",
            in_channels,
            out_channels,
            kernel_size,
            nn::ConvConfig {
                stride,
                padding: (kernel_size - 1) / 2,
                groups,
                bias: false,
                ..Default::default()
            },
        )?;
        let normalization =
            nn::batch_norm2d(path / "normalization", out_channels, Default::default())?;
        let lab = (activation != Activation::None && use_learnable_affine_block)
            .then(|| HGNetV2LearnableAffineBlock::new(&(path / "lab")))
            .transpose()?;
        Ok(Self {
            convolution,
            normalization,
            activation,
            lab,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let hidden_state = input
            .f_apply(&self.convolution)?
            .f_apply_t(&self.normalization, false)?;
        let hidden_state = self.activation.apply(hidden_state)?;
        self.lab
            .as_ref()
            .map(|lab| lab.forward(&hidden_state))
            .unwrap_or(Ok(hidden_state))
    }
}

#[derive(Debug)]
struct HGNetV2LearnableAffineBlock {
    scale: Tensor,
    bias: Tensor,
}

impl HGNetV2LearnableAffineBlock {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            scale: path.var("scale", &[1])?,
            bias: path.var("bias", &[1])?,
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        (hidden_state.f_mul(&self.scale)?).f_add(&self.bias)
    }
}

#[derive(Debug)]
struct PPDocLayoutV3HybridEncoderOutput {
    last_hidden_state: Vec<Tensor>,
    mask_feat: Tensor,
}

#[derive(Debug)]
struct PPDocLayoutV3HybridEncoder {
    config: PPDocLayoutV3Config,
    encoder: Vec<PPDocLayoutV3AIFILayer>,
    lateral_convs: Vec<PPDocLayoutV3ConvNormLayer>,
    fpn_blocks: Vec<PPDocLayoutV3CSPRepLayer>,
    downsample_convs: Vec<PPDocLayoutV3ConvNormLayer>,
    pan_blocks: Vec<PPDocLayoutV3CSPRepLayer>,
    mask_feature_head: PPDocLayoutV3MaskFeatFPN,
    encoder_mask_lateral: PPDocLayoutV3ConvLayer,
    encoder_mask_output: PPDocLayoutV3EncoderMaskOutput,
}

impl PPDocLayoutV3HybridEncoder {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        let encoder = config
            .encode_proj_layers
            .iter()
            .enumerate()
            .map(|(idx, _)| PPDocLayoutV3AIFILayer::new(&(path / "encoder" / idx), config))
            .collect::<Result<Vec<_>>>()?;
        let stages = config.encoder_in_channels.len() - 1;
        let lateral_convs = (0..stages)
            .map(|idx| {
                PPDocLayoutV3ConvNormLayer::new(
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
            .collect::<Result<Vec<_>>>()?;
        let fpn_blocks = (0..stages)
            .map(|idx| PPDocLayoutV3CSPRepLayer::new(&(path / "fpn_blocks" / idx), config))
            .collect::<Result<Vec<_>>>()?;
        let downsample_convs = (0..stages)
            .map(|idx| {
                PPDocLayoutV3ConvNormLayer::new(
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
            .collect::<Result<Vec<_>>>()?;
        let pan_blocks = (0..stages)
            .map(|idx| PPDocLayoutV3CSPRepLayer::new(&(path / "pan_blocks" / idx), config))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            config: config.clone(),
            encoder,
            lateral_convs,
            fpn_blocks,
            downsample_convs,
            pan_blocks,
            mask_feature_head: PPDocLayoutV3MaskFeatFPN::new(
                &(path / "mask_feature_head"),
                config,
            )?,
            encoder_mask_lateral: PPDocLayoutV3ConvLayer::new(
                &(path / "encoder_mask_lateral"),
                config.x4_feat_dim,
                config.mask_feature_channels[1],
                3,
                1,
                Activation::Silu,
                config.batch_norm_eps,
            )?,
            encoder_mask_output: PPDocLayoutV3EncoderMaskOutput::new(
                &(path / "encoder_mask_output"),
                config.mask_feature_channels[1],
                config.num_prototypes,
                config.batch_norm_eps,
            )?,
        })
    }

    fn forward(
        &self,
        mut feature_maps: Vec<Tensor>,
        x4_feat: &[Tensor],
    ) -> tch::Result<PPDocLayoutV3HybridEncoderOutput> {
        if self.config.encoder_layers > 0 {
            for (idx, &enc_ind) in self.config.encode_proj_layers.iter().enumerate() {
                feature_maps[enc_ind] = self.encoder[idx].forward(&feature_maps[enc_ind])?;
            }
        }

        // PP-DocLayout extends RT-DETR's AIFI/FPN/PAN encoder with a mask head;
        // keep the resolution traversal identical to Transformers.
        // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/modeling_pp_doclayout_v3.py#L953-L995
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

        let mut mask_feat = self.mask_feature_head.forward(&pan_feature_maps)?;
        let size = mask_feat.size();
        mask_feat = mask_feat.f_upsample_bilinear2d(
            [size[2] * 2, size[3] * 2],
            false,
            None::<f64>,
            None::<f64>,
        )?;
        mask_feat = mask_feat.f_add(&self.encoder_mask_lateral.forward(&x4_feat[0])?)?;
        mask_feat = self.encoder_mask_output.forward(&mask_feat)?;

        Ok(PPDocLayoutV3HybridEncoderOutput {
            last_hidden_state: pan_feature_maps,
            mask_feat,
        })
    }
}

#[derive(Debug)]
struct PPDocLayoutV3AIFILayer {
    encoder_hidden_dim: i64,
    position_embedding: PPDocLayoutV3SinePositionEmbedding,
    layers: Vec<PPDocLayoutV3EncoderLayer>,
}

impl PPDocLayoutV3AIFILayer {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        Ok(Self {
            encoder_hidden_dim: config.encoder_hidden_dim,
            position_embedding: PPDocLayoutV3SinePositionEmbedding {
                embed_dim: config.encoder_hidden_dim,
                temperature: config.positional_encoding_temperature,
            },
            layers: (0..config.encoder_layers)
                .map(|idx| PPDocLayoutV3EncoderLayer::new(&(path / "layers" / idx), config))
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
struct PPDocLayoutV3EncoderLayer {
    normalize_before: bool,
    self_attn: PPDocLayoutV3SelfAttention,
    self_attn_layer_norm: nn::LayerNorm,
    mlp: PPDocLayoutV3MLP,
    final_layer_norm: nn::LayerNorm,
}

impl PPDocLayoutV3EncoderLayer {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        Ok(Self {
            normalize_before: config.normalize_before,
            self_attn: PPDocLayoutV3SelfAttention::new(
                &(path / "self_attn"),
                config,
                config.encoder_hidden_dim,
                config.encoder_attention_heads,
            )?,
            self_attn_layer_norm: layer_norm(
                &(path / "self_attn_layer_norm"),
                config.encoder_hidden_dim,
                config.layer_norm_eps,
            )?,
            mlp: PPDocLayoutV3MLP::new(
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
struct PPDocLayoutV3DecoderOutput {
    intermediate_logits: Tensor,
    intermediate_reference_points: Tensor,
    decoder_out_order_logits: Tensor,
    decoder_out_masks: Tensor,
}

#[derive(Debug)]
struct PPDocLayoutV3Decoder {
    layers: Vec<PPDocLayoutV3DecoderLayer>,
    query_pos_head: PPDocLayoutV3MLPPredictionHead,
    num_queries: i64,
}

impl PPDocLayoutV3Decoder {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        Ok(Self {
            layers: (0..config.decoder_layers)
                .map(|idx| PPDocLayoutV3DecoderLayer::new(&(path / "layers" / idx), config))
                .collect::<Result<Vec<_>>>()?,
            query_pos_head: PPDocLayoutV3MLPPredictionHead::new(
                &(path / "query_pos_head"),
                4,
                2 * config.d_model,
                config.d_model,
                2,
            )?,
            num_queries: config.num_queries,
        })
    }

    fn forward(&self, args: DecoderForwardArgs<'_>) -> tch::Result<PPDocLayoutV3DecoderOutput> {
        let mut hidden_states = args.inputs_embeds.f_alias()?;
        let mut reference_points = args.reference_points.f_sigmoid()?;
        let mut intermediate_logits = Vec::with_capacity(self.layers.len());
        let mut intermediate_reference_points = Vec::with_capacity(self.layers.len());
        let mut decoder_out_order_logits = Vec::with_capacity(self.layers.len());
        let mut decoder_out_masks = Vec::with_capacity(self.layers.len());

        for (idx, decoder_layer) in self.layers.iter().enumerate() {
            let reference_points_input = reference_points.f_unsqueeze(2)?;
            let object_queries_position_embeddings =
                self.query_pos_head.forward(&reference_points)?;
            hidden_states = decoder_layer.forward(DecoderLayerArgs {
                hidden_states: &hidden_states,
                object_queries_position_embeddings: &object_queries_position_embeddings,
                reference_points: &reference_points_input,
                spatial_shapes: args.spatial_shapes,
                spatial_shapes_list: args.spatial_shapes_list,
                level_start_index: args.level_start_index,
                encoder_hidden_states: args.encoder_hidden_states,
            })?;

            let predicted_corners = args.bbox_embed.forward(&hidden_states)?;
            let new_reference_points = predicted_corners
                .f_add(&inverse_sigmoid(&reference_points)?)?
                .f_sigmoid()?;
            reference_points = new_reference_points.f_detach()?;

            // Each decoder layer emits masks and reading-order logits. Only the
            // configured detection queries participate in the order matrix.
            // https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/modeling_pp_doclayout_v3.py#L1190-L1229
            let out_query = args.norm.forward(&hidden_states)?;
            let mask_query_embed = args.mask_query_head.forward(&out_query)?;
            let size = args.mask_feat.size();
            let out_mask = mask_query_embed
                .f_bmm(&args.mask_feat.f_flatten(2, -1)?)?
                .f_reshape([size[0], mask_query_embed.size()[1], size[2], size[3]])?;
            decoder_out_masks.push(out_mask);

            intermediate_logits.push(args.class_embed.forward(&out_query)?);
            intermediate_reference_points.push(new_reference_points);

            let valid_query_start = out_query.size()[1] - self.num_queries;
            let valid_query = out_query.f_i((.., valid_query_start..))?;
            let order_hidden = args.order_head[idx].forward(&valid_query)?;
            decoder_out_order_logits.push(args.global_pointer.forward(&order_hidden)?);
        }

        Ok(PPDocLayoutV3DecoderOutput {
            intermediate_logits: Tensor::f_stack(&intermediate_logits, 1)?,
            intermediate_reference_points: Tensor::f_stack(&intermediate_reference_points, 1)?,
            decoder_out_order_logits: Tensor::f_stack(&decoder_out_order_logits, 1)?,
            decoder_out_masks: Tensor::f_stack(&decoder_out_masks, 1)?,
        })
    }
}

struct DecoderForwardArgs<'a> {
    inputs_embeds: &'a Tensor,
    encoder_hidden_states: &'a Tensor,
    reference_points: &'a Tensor,
    spatial_shapes: &'a Tensor,
    spatial_shapes_list: &'a [(i64, i64)],
    level_start_index: &'a Tensor,
    order_head: &'a [nn::Linear],
    bbox_embed: &'a PPDocLayoutV3MLPPredictionHead,
    class_embed: &'a nn::Linear,
    global_pointer: &'a PPDocLayoutV3GlobalPointer,
    mask_query_head: &'a PPDocLayoutV3MLPPredictionHead,
    norm: &'a nn::LayerNorm,
    mask_feat: &'a Tensor,
}

#[derive(Debug)]
struct PPDocLayoutV3DecoderLayer {
    self_attn: PPDocLayoutV3SelfAttention,
    self_attn_layer_norm: nn::LayerNorm,
    encoder_attn: PPDocLayoutV3MultiscaleDeformableAttention,
    encoder_attn_layer_norm: nn::LayerNorm,
    mlp: PPDocLayoutV3MLP,
    final_layer_norm: nn::LayerNorm,
}

impl PPDocLayoutV3DecoderLayer {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        Ok(Self {
            self_attn: PPDocLayoutV3SelfAttention::new(
                &(path / "self_attn"),
                config,
                config.d_model,
                config.decoder_attention_heads,
            )?,
            self_attn_layer_norm: layer_norm(
                &(path / "self_attn_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            )?,
            encoder_attn: PPDocLayoutV3MultiscaleDeformableAttention::new(
                &(path / "encoder_attn"),
                config,
                config.decoder_attention_heads,
                config.decoder_n_points,
            )?,
            encoder_attn_layer_norm: layer_norm(
                &(path / "encoder_attn_layer_norm"),
                config.d_model,
                config.layer_norm_eps,
            )?,
            mlp: PPDocLayoutV3MLP::new(
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

    fn forward(&self, args: DecoderLayerArgs<'_>) -> tch::Result<Tensor> {
        let residual = args.hidden_states.f_alias()?;
        let hidden_states = self.self_attn.forward(
            args.hidden_states,
            Some(args.object_queries_position_embeddings),
        )?;
        let hidden_states = self
            .self_attn_layer_norm
            .forward(&residual.f_add(&hidden_states)?)?;

        let residual = hidden_states.f_alias()?;
        let hidden_states = self.encoder_attn.forward(MultiscaleAttentionArgs {
            hidden_states: &hidden_states,
            encoder_hidden_states: args.encoder_hidden_states,
            position_embeddings: args.object_queries_position_embeddings,
            reference_points: args.reference_points,
            spatial_shapes: args.spatial_shapes,
            spatial_shapes_list: args.spatial_shapes_list,
            level_start_index: args.level_start_index,
        })?;
        let hidden_states = self
            .encoder_attn_layer_norm
            .forward(&residual.f_add(&hidden_states)?)?;
        let residual = hidden_states.f_alias()?;
        self.final_layer_norm
            .forward(&residual.f_alias()?.f_add(&self.mlp.forward(&residual)?)?)
    }
}

struct DecoderLayerArgs<'a> {
    hidden_states: &'a Tensor,
    object_queries_position_embeddings: &'a Tensor,
    reference_points: &'a Tensor,
    spatial_shapes: &'a Tensor,
    spatial_shapes_list: &'a [(i64, i64)],
    level_start_index: &'a Tensor,
    encoder_hidden_states: &'a Tensor,
}

#[derive(Debug)]
struct PPDocLayoutV3SelfAttention {
    head_dim: i64,
    num_heads: i64,
    scaling: f64,
    k_proj: nn::Linear,
    v_proj: nn::Linear,
    q_proj: nn::Linear,
    out_proj: nn::Linear,
}

impl PPDocLayoutV3SelfAttention {
    fn new(
        path: &nn::Path<'_>,
        _config: &PPDocLayoutV3Config,
        hidden_size: i64,
        num_heads: i64,
    ) -> Result<Self> {
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
            out_proj: nn::linear(
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
        self.out_proj.forward(&attn_output.f_reshape([
            batch_size,
            sequence_length,
            self.num_heads * self.head_dim,
        ])?)
    }
}

#[derive(Debug)]
struct PPDocLayoutV3MultiscaleDeformableAttention {
    d_model: i64,
    n_levels: i64,
    n_heads: i64,
    n_points: i64,
    sampling_offsets: nn::Linear,
    attention_weights: nn::Linear,
    value_proj: nn::Linear,
    output_proj: nn::Linear,
}

impl PPDocLayoutV3MultiscaleDeformableAttention {
    fn new(
        path: &nn::Path<'_>,
        config: &PPDocLayoutV3Config,
        num_heads: i64,
        n_points: i64,
    ) -> Result<Self> {
        Ok(Self {
            d_model: config.d_model,
            n_levels: config.num_feature_levels as i64,
            n_heads: num_heads,
            n_points,
            sampling_offsets: nn::linear(
                path / "sampling_offsets",
                config.d_model,
                num_heads * config.num_feature_levels as i64 * n_points * 2,
                Default::default(),
            )?,
            attention_weights: nn::linear(
                path / "attention_weights",
                config.d_model,
                num_heads * config.num_feature_levels as i64 * n_points,
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

    fn forward(&self, args: MultiscaleAttentionArgs<'_>) -> tch::Result<Tensor> {
        let hidden_states = args.hidden_states.f_add(args.position_embeddings)?;
        let batch_size = hidden_states.size()[0];
        let num_queries = hidden_states.size()[1];
        let sequence_length = args.encoder_hidden_states.size()[1];
        let dim_per_head = self.d_model / self.n_heads;

        let value = self
            .value_proj
            .forward(args.encoder_hidden_states)?
            .f_view([batch_size, sequence_length, self.n_heads, dim_per_head])?;
        let sampling_offsets = self.sampling_offsets.forward(&hidden_states)?.f_view([
            batch_size,
            num_queries,
            self.n_heads,
            self.n_levels,
            self.n_points,
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
            .f_softmax(-1, None::<Kind>)?
            .f_view([
                batch_size,
                num_queries,
                self.n_heads,
                self.n_levels,
                self.n_points,
            ])?;

        let sampling_locations = if args.reference_points.size().last().copied() == Some(4) {
            let ref_xy = args
                .reference_points
                .f_slice(-1, 0, 2, 1)?
                .f_unsqueeze(2)?
                .f_unsqueeze(4)?;
            let ref_wh = args
                .reference_points
                .f_slice(-1, 2, 4, 1)?
                .f_unsqueeze(2)?
                .f_unsqueeze(4)?;
            ref_xy.f_add(
                &((sampling_offsets.f_div_scalar(self.n_points as f64)?).f_mul(&ref_wh)?)
                    .f_mul_scalar(0.5)?,
            )?
        } else {
            let normalizer = Tensor::f_stack(
                &[
                    args.spatial_shapes.f_i((.., 1))?,
                    args.spatial_shapes.f_i((.., 0))?,
                ],
                -1,
            )?
            .f_to_kind(value.kind())?;
            args.reference_points
                .f_unsqueeze(2)?
                .f_unsqueeze(4)?
                .f_add(
                    &sampling_offsets.f_div(
                        &normalizer
                            .f_unsqueeze(0)?
                            .f_unsqueeze(0)?
                            .f_unsqueeze(0)?
                            .f_unsqueeze(4)?,
                    )?,
                )?
        };

        let output = multiscale_deformable_attention(
            &value,
            args.spatial_shapes_list,
            &sampling_locations,
            &attention_weights,
        )?;
        self.output_proj.forward(&output)
    }
}

struct MultiscaleAttentionArgs<'a> {
    hidden_states: &'a Tensor,
    encoder_hidden_states: &'a Tensor,
    position_embeddings: &'a Tensor,
    reference_points: &'a Tensor,
    spatial_shapes: &'a Tensor,
    spatial_shapes_list: &'a [(i64, i64)],
    #[allow(dead_code)]
    level_start_index: &'a Tensor,
}

#[derive(Debug)]
struct PPDocLayoutV3GlobalPointer {
    head_size: i64,
    dense: nn::Linear,
}

impl PPDocLayoutV3GlobalPointer {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        Ok(Self {
            head_size: config.global_pointer_head_size,
            dense: nn::linear(
                path / "dense",
                config.d_model,
                config.global_pointer_head_size * 2,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, inputs: &Tensor) -> tch::Result<Tensor> {
        let size = inputs.size();
        let batch_size = size[0];
        let sequence_length = size[1];
        let query_key_projection = self.dense.forward(inputs)?.f_reshape([
            batch_size,
            sequence_length,
            2,
            self.head_size,
        ])?;
        let parts = query_key_projection.f_unbind(2)?;
        let queries = &parts[0];
        let keys = &parts[1];
        let logits = (queries.f_matmul(&keys.f_transpose(-2, -1)?)?)
            .f_div_scalar((self.head_size as f64).sqrt())?;
        let mask = Tensor::f_ones(
            [sequence_length, sequence_length],
            (Kind::Bool, logits.device()),
        )?
        .f_tril(0)?
        .f_unsqueeze(0)?;
        logits.f_masked_fill(&mask, -1e4)
    }
}

#[derive(Debug)]
struct PPDocLayoutV3MLP {
    fc1: nn::Linear,
    fc2: nn::Linear,
    activation: Activation,
}

impl PPDocLayoutV3MLP {
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
struct PPDocLayoutV3MLPPredictionHead {
    layers: Vec<nn::Linear>,
}

impl PPDocLayoutV3MLPPredictionHead {
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
struct PPDocLayoutV3MaskFeatFPN {
    reorder_index: Vec<usize>,
    scale_heads: Vec<PPDocLayoutV3ScaleHead>,
    output_conv: PPDocLayoutV3ConvLayer,
}

impl PPDocLayoutV3MaskFeatFPN {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        let mut order = (0..config.feat_strides.len()).collect::<Vec<_>>();
        order.sort_by_key(|&idx| config.feat_strides[idx]);
        let fpn_strides = order
            .iter()
            .map(|&idx| config.feat_strides[idx])
            .collect::<Vec<_>>();
        let base_stride = fpn_strides[0];
        let scale_heads = order
            .iter()
            .enumerate()
            .map(|(scale_idx, &source_idx)| {
                PPDocLayoutV3ScaleHead::new(
                    &(path / "scale_heads" / scale_idx),
                    config.encoder_hidden_dim,
                    config.mask_feature_channels[0],
                    config.feat_strides[source_idx],
                    base_stride,
                    config.batch_norm_eps,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            reorder_index: order,
            scale_heads,
            output_conv: PPDocLayoutV3ConvLayer::new(
                &(path / "output_conv"),
                config.mask_feature_channels[0],
                config.mask_feature_channels[1],
                3,
                1,
                Activation::Silu,
                config.batch_norm_eps,
            )?,
        })
    }

    fn forward(&self, inputs: &[Tensor]) -> tch::Result<Tensor> {
        let mut output = self.scale_heads[0].forward(&inputs[self.reorder_index[0]])?;
        for (scale_head, &input_index) in self.scale_heads.iter().zip(&self.reorder_index).skip(1) {
            let scaled = scale_head.forward(&inputs[input_index])?;
            let size = output.size();
            let scaled = scaled.f_upsample_bilinear2d(
                [size[2], size[3]],
                false,
                None::<f64>,
                None::<f64>,
            )?;
            output = output.f_add(&scaled)?;
        }
        self.output_conv.forward(&output)
    }
}

#[derive(Debug)]
struct PPDocLayoutV3ScaleHead {
    layers: Vec<ScaleHeadLayer>,
}

impl PPDocLayoutV3ScaleHead {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        feature_channels: i64,
        fpn_stride: i64,
        base_stride: i64,
        eps: f64,
    ) -> Result<Self> {
        let head_length =
            ((fpn_stride as f64).log2() - (base_stride as f64).log2()).max(1.0) as usize;
        let mut layers = Vec::new();
        let mut module_idx = 0;
        for idx in 0..head_length {
            let in_c = if idx == 0 {
                in_channels
            } else {
                feature_channels
            };
            layers.push(ScaleHeadLayer::Conv(Box::new(PPDocLayoutV3ConvLayer::new(
                &(path / "layers" / module_idx),
                in_c,
                feature_channels,
                3,
                1,
                Activation::Silu,
                eps,
            )?)));
            module_idx += 1;
            if fpn_stride != base_stride {
                layers.push(ScaleHeadLayer::Upsample2x);
                module_idx += 1;
            }
        }
        Ok(Self { layers })
    }

    fn forward(&self, x: &Tensor) -> tch::Result<Tensor> {
        let mut x = x.f_alias()?;
        for layer in &self.layers {
            x = match layer {
                ScaleHeadLayer::Conv(layer) => layer.forward(&x)?,
                ScaleHeadLayer::Upsample2x => {
                    let size = x.size();
                    x.f_upsample_bilinear2d(
                        [size[2] * 2, size[3] * 2],
                        false,
                        None::<f64>,
                        None::<f64>,
                    )?
                }
            }
        }
        Ok(x)
    }
}

#[derive(Debug)]
enum ScaleHeadLayer {
    Conv(Box<PPDocLayoutV3ConvLayer>),
    Upsample2x,
}

#[derive(Debug)]
struct PPDocLayoutV3EncoderMaskOutput {
    base_conv: PPDocLayoutV3ConvLayer,
    conv: nn::Conv2D,
}

impl PPDocLayoutV3EncoderMaskOutput {
    fn new(path: &nn::Path<'_>, in_channels: i64, num_prototypes: i64, eps: f64) -> Result<Self> {
        Ok(Self {
            base_conv: PPDocLayoutV3ConvLayer::new(
                &(path / "base_conv"),
                in_channels,
                in_channels,
                3,
                1,
                Activation::Silu,
                eps,
            )?,
            conv: nn::conv2d(
                path / "conv",
                in_channels,
                num_prototypes,
                1,
                Default::default(),
            )?,
        })
    }

    fn forward(&self, x: &Tensor) -> tch::Result<Tensor> {
        self.base_conv.forward(x)?.f_apply(&self.conv)
    }
}

#[derive(Debug)]
struct PPDocLayoutV3CSPRepLayer {
    conv1: PPDocLayoutV3ConvNormLayer,
    conv2: PPDocLayoutV3ConvNormLayer,
    bottlenecks: Vec<PPDocLayoutV3RepVggBlock>,
    conv3: Option<PPDocLayoutV3ConvNormLayer>,
}

impl PPDocLayoutV3CSPRepLayer {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        let in_channels = config.encoder_hidden_dim * 2;
        let out_channels = config.encoder_hidden_dim;
        let hidden_channels = (out_channels as f64 * config.hidden_expansion) as i64;
        Ok(Self {
            conv1: PPDocLayoutV3ConvNormLayer::new(
                &(path / "conv1"),
                in_channels,
                hidden_channels,
                1,
                1,
                Some(0),
                Activation::from_name(&config.activation_function),
                config.batch_norm_eps,
            )?,
            conv2: PPDocLayoutV3ConvNormLayer::new(
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
                .map(|idx| PPDocLayoutV3RepVggBlock::new(&(path / "bottlenecks" / idx), config))
                .collect::<Result<Vec<_>>>()?,
            conv3: (hidden_channels != out_channels)
                .then(|| {
                    PPDocLayoutV3ConvNormLayer::new(
                        &(path / "conv3"),
                        hidden_channels,
                        out_channels,
                        1,
                        1,
                        Some(0),
                        Activation::from_name(&config.activation_function),
                        config.batch_norm_eps,
                    )
                })
                .transpose()?,
        })
    }

    fn forward(&self, hidden_state: &Tensor) -> tch::Result<Tensor> {
        let mut hidden_state_1 = self.conv1.forward(hidden_state)?;
        for bottleneck in &self.bottlenecks {
            hidden_state_1 = bottleneck.forward(&hidden_state_1)?;
        }
        let hidden_state_2 = self.conv2.forward(hidden_state)?;
        let out = hidden_state_1.f_add(&hidden_state_2)?;
        self.conv3
            .as_ref()
            .map(|conv3| conv3.forward(&out))
            .unwrap_or(Ok(out))
    }
}

#[derive(Debug)]
struct PPDocLayoutV3RepVggBlock {
    conv1: PPDocLayoutV3ConvNormLayer,
    conv2: PPDocLayoutV3ConvNormLayer,
    activation: Activation,
}

impl PPDocLayoutV3RepVggBlock {
    fn new(path: &nn::Path<'_>, config: &PPDocLayoutV3Config) -> Result<Self> {
        let hidden_channels = (config.encoder_hidden_dim as f64 * config.hidden_expansion) as i64;
        Ok(Self {
            conv1: PPDocLayoutV3ConvNormLayer::new(
                &(path / "conv1"),
                hidden_channels,
                hidden_channels,
                3,
                1,
                Some(1),
                Activation::None,
                config.batch_norm_eps,
            )?,
            conv2: PPDocLayoutV3ConvNormLayer::new(
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
struct PPDocLayoutV3ConvNormLayer {
    conv: nn::Conv2D,
    norm: nn::BatchNorm,
    activation: Activation,
}

impl PPDocLayoutV3ConvNormLayer {
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
struct PPDocLayoutV3ConvLayer {
    convolution: nn::Conv2D,
    normalization: nn::BatchNorm,
    activation: Activation,
}

impl PPDocLayoutV3ConvLayer {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        activation: Activation,
        eps: f64,
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
            normalization: nn::batch_norm2d(
                path / "normalization",
                out_channels,
                nn::BatchNormConfig {
                    eps,
                    ..Default::default()
                },
            )?,
            activation,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        self.activation.apply(
            input
                .f_apply(&self.convolution)?
                .f_apply_t(&self.normalization, false)?,
        )
    }
}

#[derive(Debug)]
struct ConvBnSeq {
    conv: nn::Conv2D,
    bn: nn::BatchNorm,
}

impl ConvBnSeq {
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        padding: i64,
        groups: i64,
        eps: f64,
    ) -> Result<Self> {
        Ok(Self {
            conv: nn::conv2d(
                path / 0,
                in_channels,
                out_channels,
                kernel_size,
                nn::ConvConfig {
                    stride,
                    padding,
                    groups,
                    bias: false,
                    ..Default::default()
                },
            )?,
            bn: nn::batch_norm2d(
                path / 1,
                out_channels,
                nn::BatchNormConfig {
                    eps,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        xs.f_apply(&self.conv)?.f_apply_t(&self.bn, false)
    }
}

#[derive(Debug)]
struct LinearNormSeq {
    linear: nn::Linear,
    norm: nn::LayerNorm,
}

impl LinearNormSeq {
    fn new(path: &nn::Path<'_>, hidden_dim: i64, eps: f64) -> Result<Self> {
        Ok(Self {
            linear: nn::linear(path / 0, hidden_dim, hidden_dim, Default::default())?,
            norm: layer_norm(&(path / 1), hidden_dim, eps)?,
        })
    }

    fn forward(&self, xs: &Tensor) -> tch::Result<Tensor> {
        self.norm.forward(&self.linear.forward(xs)?)
    }
}

#[derive(Debug)]
struct PPDocLayoutV3SinePositionEmbedding {
    embed_dim: i64,
    temperature: f64,
}

impl PPDocLayoutV3SinePositionEmbedding {
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

fn multiscale_deformable_attention(
    value: &Tensor,
    spatial_shapes_list: &[(i64, i64)],
    sampling_locations: &Tensor,
    attention_weights: &Tensor,
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
    let sampling_grids = sampling_locations.f_mul_scalar(2.0)?.f_sub_scalar(1.0)?;

    let mut sampling_values = Vec::with_capacity(spatial_shapes_list.len());
    for (level, (height, width)) in spatial_shapes_list.iter().copied().enumerate() {
        let value_l = value_list[level]
            .f_flatten(2, -1)?
            .f_transpose(1, 2)?
            .f_reshape([batch_size * num_heads, hidden_dim, height, width])?;
        let sampling_grid_l = sampling_grids
            .f_i((.., .., .., level as i64))?
            .f_transpose(1, 2)?
            .f_flatten(0, 1)?;
        sampling_values.push(value_l.f_grid_sampler_2d(&sampling_grid_l, 0, 0, false)?);
    }

    let attention_weights = attention_weights.f_transpose(1, 2)?.f_reshape([
        batch_size * num_heads,
        1,
        num_queries,
        spatial_shapes_list.len() as i64 * sampling_locations.size()[4],
    ])?;
    (Tensor::f_stack(&sampling_values, -2)?.f_flatten(-2, -1)?)
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
    let max = Tensor::f_full([1, total, 4], 1.0e20, (kind, device))?;
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

fn mask_to_box_coordinate(mask: &Tensor, kind: Kind) -> tch::Result<Tensor> {
    let mask = mask.f_to_kind(Kind::Bool)?;
    let size = mask.size();
    let batch_size = size[0];
    let queries = size[1];
    let height = size[2];
    let width = size[3];
    let device = mask.device();
    let mask_f = mask.f_to_kind(kind)?;
    let x_coords = Tensor::f_arange(width, (kind, device))?
        .f_view([1, 1, 1, width])?
        .f_expand([batch_size, queries, height, width], true)?;
    let y_coords = Tensor::f_arange(height, (kind, device))?
        .f_view([1, 1, height, 1])?
        .f_expand([batch_size, queries, height, width], true)?;
    let big = Tensor::f_full([batch_size, queries, height, width], 1.0e20, (kind, device))?;

    let x_max = (x_coords
        .f_mul(&mask_f)?
        .f_flatten(-2, -1)?
        .f_max_dim(-1, false)?
        .0)
        .f_add_scalar(1.0)?;
    let x_min = x_coords
        .f_where_self(&mask, &big)?
        .f_flatten(-2, -1)?
        .f_min_dim(-1, false)?
        .0;
    let y_max = (y_coords
        .f_mul(&mask_f)?
        .f_flatten(-2, -1)?
        .f_max_dim(-1, false)?
        .0)
        .f_add_scalar(1.0)?;
    let y_min = y_coords
        .f_where_self(&mask, &big)?
        .f_flatten(-2, -1)?
        .f_min_dim(-1, false)?
        .0;

    let unnormalized_bbox = Tensor::f_stack(&[x_min, y_min, x_max, y_max], -1)?;
    let is_non_empty = mask
        .f_any_dims(&[-2i64, -1][..], false)?
        .f_unsqueeze(-1)?
        .f_to_kind(kind)?;
    let norm = Tensor::f_from_slice(&[width as f32, height as f32, width as f32, height as f32])?
        .f_to_kind(kind)?
        .f_to_device(device)?
        .f_view([1, 1, 4])?;
    let bbox = unnormalized_bbox.f_mul(&is_non_empty)?.f_div(&norm)?;
    let x_min = bbox.f_i((.., .., 0))?;
    let y_min = bbox.f_i((.., .., 1))?;
    let x_max = bbox.f_i((.., .., 2))?;
    let y_max = bbox.f_i((.., .., 3))?;
    let center_x = (x_min.f_add(&x_max)?).f_div_scalar(2.0)?;
    let center_y = (y_min.f_add(&y_max)?).f_div_scalar(2.0)?;
    let box_width = x_max.f_sub(&x_min)?;
    let box_height = y_max.f_sub(&y_min)?;
    Tensor::f_stack(&[center_x, center_y, box_width, box_height], -1)
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

fn level_start_index_tensor(spatial_shapes: &[(i64, i64)], device: Device) -> tch::Result<Tensor> {
    let mut offset = 0;
    let mut starts = Vec::with_capacity(spatial_shapes.len());
    for (height, width) in spatial_shapes {
        starts.push(offset);
        offset += height * width;
    }
    Tensor::f_from_slice(&starts)?.f_to_device(device)
}
