//! PP-OCRv6 medium text detector, structurally ported from Transformers.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/pp_ocrv6_medium_det/modeling_pp_ocrv6_medium_det.py

use std::path::Path;

use anyhow::Result;
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module, ModuleT},
};

use super::super::pp_lcnet_v4::{PPLCNetV4Backbone, Spatial};
use super::config::{IntraclassBlockConfig, PPOCRV6MediumDetConfig};

#[derive(Debug)]
pub(crate) struct Model {
    vs: nn::VarStore,
    backbone: PPLCNetV4Backbone,
    neck: Neck,
    head: Head,
}

impl Model {
    pub(crate) fn new(config: &PPOCRV6MediumDetConfig, device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let backbone = PPLCNetV4Backbone::new(
            &(&vs.root() / "model" / "backbone"),
            &config.backbone_config,
        )?;
        let neck = Neck::new(&(&vs.root() / "model" / "neck"), config)?;
        let head = Head::new(&(&vs.root() / "head"), config)?;
        Ok(Self {
            vs,
            backbone,
            neck,
            head,
        })
    }

    pub(crate) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub(crate) fn forward(&self, pixel_values: &Tensor) -> koharu_torch::Result<Tensor> {
        let feature_maps = self
            .backbone
            .forward(&pixel_values.to_kind(self.vs.kind())?)?;
        self.head.forward(&self.neck.forward(&feature_maps)?)
    }
}

fn upsample_nearest(hidden_states: &Tensor, scale_factor: i64) -> koharu_torch::Result<Tensor> {
    let size = hidden_states.size();
    hidden_states.upsample_nearest2d(
        [size[2] * scale_factor, size[3] * scale_factor],
        None::<f64>,
        None::<f64>,
    )
}

fn conv2d(
    path: &nn::Path<'_>,
    in_channels: i64,
    out_channels: i64,
    kernel_size: [i64; 2],
    stride: [i64; 2],
    padding: [i64; 2],
    bias: bool,
) -> koharu_torch::Result<nn::Conv2D> {
    nn::conv(
        path,
        in_channels,
        out_channels,
        kernel_size,
        nn::ConvConfigND {
            stride,
            padding,
            bias,
            ..Default::default()
        },
    )
}

fn spec(values: &[Spatial]) -> ([i64; 2], [i64; 2], [i64; 2]) {
    (values[0].pair(), values[1].pair(), values[2].pair())
}

#[derive(Debug)]
struct ConvBatchnormLayer {
    convolution: nn::Conv2D,
    norm: nn::BatchNorm,
}

impl ConvBatchnormLayer {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        padding: i64,
        bias: bool,
    ) -> Result<Self> {
        Ok(Self {
            convolution: nn::conv2d(
                path / "convolution",
                in_channels,
                out_channels,
                kernel_size,
                nn::ConvConfig {
                    stride,
                    padding,
                    bias,
                    ..Default::default()
                },
            )?,
            norm: nn::batch_norm2d(path / "norm", out_channels, Default::default())?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        self.norm
            .forward_t(&self.convolution.forward(hidden_states)?, false)?
            .relu()
    }
}

#[derive(Debug)]
struct IntraclassBlock {
    conv_reduce_channel: nn::Conv2D,
    vertical_long_to_small_conv_longratio: nn::Conv2D,
    vertical_long_to_small_conv_midratio: nn::Conv2D,
    vertical_long_to_small_conv_shortratio: nn::Conv2D,
    horizontal_small_to_long_conv_longratio: nn::Conv2D,
    horizontal_small_to_long_conv_midratio: nn::Conv2D,
    horizontal_small_to_long_conv_shortratio: nn::Conv2D,
    symmetric_conv_long_longratio: nn::Conv2D,
    symmetric_conv_long_midratio: nn::Conv2D,
    symmetric_conv_long_shortratio: nn::Conv2D,
    conv_final: ConvBatchnormLayer,
}

impl IntraclassBlock {
    fn new(
        path: &nn::Path<'_>,
        config: &IntraclassBlockConfig,
        in_channels: i64,
        reduce_factor: i64,
    ) -> Result<Self> {
        let reduced_channels = in_channels / reduce_factor;
        let make = |name: &str, values: &[Spatial]| {
            let (kernel, stride, padding) = spec(values);
            conv2d(
                &(path / name),
                reduced_channels,
                reduced_channels,
                kernel,
                stride,
                padding,
                true,
            )
        };
        let (reduce_kernel, reduce_stride, reduce_padding) = spec(&config.reduce_channel);
        let (return_kernel, return_stride, return_padding) = spec(&config.return_channel);
        assert_eq!(return_kernel[0], return_kernel[1]);
        assert_eq!(return_stride[0], return_stride[1]);
        assert_eq!(return_padding[0], return_padding[1]);
        Ok(Self {
            conv_reduce_channel: conv2d(
                &(path / "conv_reduce_channel"),
                in_channels,
                reduced_channels,
                reduce_kernel,
                reduce_stride,
                reduce_padding,
                true,
            )?,
            vertical_long_to_small_conv_longratio: make(
                "vertical_long_to_small_conv_longratio",
                &config.vertical_long_to_small_conv_longratio,
            )?,
            vertical_long_to_small_conv_midratio: make(
                "vertical_long_to_small_conv_midratio",
                &config.vertical_long_to_small_conv_midratio,
            )?,
            vertical_long_to_small_conv_shortratio: make(
                "vertical_long_to_small_conv_shortratio",
                &config.vertical_long_to_small_conv_shortratio,
            )?,
            horizontal_small_to_long_conv_longratio: make(
                "horizontal_small_to_long_conv_longratio",
                &config.horizontal_small_to_long_conv_longratio,
            )?,
            horizontal_small_to_long_conv_midratio: make(
                "horizontal_small_to_long_conv_midratio",
                &config.horizontal_small_to_long_conv_midratio,
            )?,
            horizontal_small_to_long_conv_shortratio: make(
                "horizontal_small_to_long_conv_shortratio",
                &config.horizontal_small_to_long_conv_shortratio,
            )?,
            symmetric_conv_long_longratio: make(
                "symmetric_conv_long_longratio",
                &config.symmetric_conv_long_longratio,
            )?,
            symmetric_conv_long_midratio: make(
                "symmetric_conv_long_midratio",
                &config.symmetric_conv_long_midratio,
            )?,
            symmetric_conv_long_shortratio: make(
                "symmetric_conv_long_shortratio",
                &config.symmetric_conv_long_shortratio,
            )?,
            conv_final: ConvBatchnormLayer::new(
                &(path / "conv_final"),
                reduced_channels,
                in_channels,
                return_kernel[0],
                return_stride[0],
                return_padding[0],
                true,
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let residual = hidden_states;
        let hidden_states = self.conv_reduce_channel.forward(hidden_states)?;
        let hidden_states = ((self.symmetric_conv_long_longratio.forward(&hidden_states)?
            + self
                .vertical_long_to_small_conv_longratio
                .forward(&hidden_states)?)?
            + self
                .horizontal_small_to_long_conv_longratio
                .forward(&hidden_states)?)?;
        let hidden_states = ((self.symmetric_conv_long_midratio.forward(&hidden_states)?
            + self
                .vertical_long_to_small_conv_midratio
                .forward(&hidden_states)?)?
            + self
                .horizontal_small_to_long_conv_midratio
                .forward(&hidden_states)?)?;
        let hidden_states = ((self
            .symmetric_conv_long_shortratio
            .forward(&hidden_states)?
            + self
                .vertical_long_to_small_conv_shortratio
                .forward(&hidden_states)?)?
            + self
                .horizontal_small_to_long_conv_shortratio
                .forward(&hidden_states)?)?;
        residual + self.conv_final.forward(&hidden_states)?
    }
}

#[derive(Debug)]
struct Neck {
    input_channel_adjustment_convolution: Vec<nn::Conv2D>,
    input_feature_projection_convolution: Vec<nn::Conv2D>,
    path_aggregation_head_convolution: Vec<nn::Conv2D>,
    path_aggregation_lateral_convolution: Vec<nn::Conv2D>,
    intraclass_blocks: Vec<IntraclassBlock>,
    scale_factor_list: Vec<i64>,
}

impl Neck {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumDetConfig) -> Result<Self> {
        assert_eq!(config.interpolate_mode, "nearest");
        let backbone_channels = config.backbone_config.stage_out_channels();
        let mut input_channel_adjustment_convolution = Vec::new();
        let mut input_feature_projection_convolution = Vec::new();
        let mut path_aggregation_head_convolution = Vec::new();
        let mut path_aggregation_lateral_convolution = Vec::new();
        for (index, &channels) in backbone_channels.iter().enumerate() {
            input_channel_adjustment_convolution.push(nn::conv2d(
                path / "input_channel_adjustment_convolution" / index,
                channels,
                config.neck_out_channels,
                1,
                nn::ConvConfig {
                    bias: false,
                    ..Default::default()
                },
            )?);
            input_feature_projection_convolution.push(nn::conv2d(
                path / "input_feature_projection_convolution" / index,
                config.neck_out_channels,
                config.neck_out_channels / 4,
                9,
                nn::ConvConfig {
                    padding: 4,
                    ..Default::default()
                },
            )?);
            if index > 0 {
                path_aggregation_head_convolution.push(nn::conv2d(
                    path / "path_aggregation_head_convolution" / (index - 1),
                    config.neck_out_channels / 4,
                    config.neck_out_channels / 4,
                    3,
                    nn::ConvConfig {
                        stride: 2,
                        padding: 1,
                        bias: false,
                        ..Default::default()
                    },
                )?);
            }
            path_aggregation_lateral_convolution.push(nn::conv2d(
                path / "path_aggregation_lateral_convolution" / index,
                config.neck_out_channels / 4,
                config.neck_out_channels / 4,
                9,
                nn::ConvConfig {
                    padding: 4,
                    ..Default::default()
                },
            )?);
        }
        let intraclass_blocks = (0..config.intraclass_block_number)
            .map(|index| {
                IntraclassBlock::new(
                    &(path / "intraclass_blocks" / index),
                    &config.intraclass_block_config,
                    config.neck_out_channels / 4,
                    config.reduce_factor,
                )
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            input_channel_adjustment_convolution,
            input_feature_projection_convolution,
            path_aggregation_head_convolution,
            path_aggregation_lateral_convolution,
            intraclass_blocks,
            scale_factor_list: config.scale_factor_list.clone(),
        })
    }

    fn forward(&self, feature_maps: &[Tensor]) -> koharu_torch::Result<Tensor> {
        let channel_adjusted = feature_maps
            .iter()
            .zip(&self.input_channel_adjustment_convolution)
            .map(|(feature, convolution)| convolution.forward(feature))
            .collect::<koharu_torch::Result<Vec<_>>>()?;

        let mut top_down = channel_adjusted
            .iter()
            .map(Tensor::alias)
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        for index in (0..top_down.len() - 1).rev() {
            top_down[index] =
                (&channel_adjusted[index] + upsample_nearest(&top_down[index + 1], 2)?)?;
        }

        let projected = top_down
            .iter()
            .enumerate()
            .map(|(index, feature)| {
                let feature = if index + 1 == top_down.len() {
                    &channel_adjusted[index]
                } else {
                    feature
                };
                self.input_feature_projection_convolution[index].forward(feature)
            })
            .collect::<koharu_torch::Result<Vec<_>>>()?;

        let mut bottom_up = projected
            .iter()
            .map(Tensor::alias)
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        for index in 1..bottom_up.len() {
            bottom_up[index] = (&projected[index]
                + self.path_aggregation_head_convolution[index - 1]
                    .forward(&bottom_up[index - 1])?)?;
        }

        let lateral_refined = (0..projected.len())
            .map(|index| {
                let feature = if index == 0 {
                    &projected[0]
                } else {
                    &bottom_up[index]
                };
                self.path_aggregation_lateral_convolution[index].forward(feature)
            })
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        let intraclass_refined = lateral_refined
            .iter()
            .zip(&self.intraclass_blocks)
            .map(|(feature, block)| block.forward(feature))
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        let mut upsampled = intraclass_refined
            .iter()
            .zip(&self.scale_factor_list)
            .map(|(feature, &scale)| {
                if scale > 1 {
                    upsample_nearest(feature, scale)
                } else {
                    feature.alias()
                }
            })
            .collect::<koharu_torch::Result<Vec<_>>>()?;
        upsampled.reverse();
        Tensor::cat(&upsampled, 1)
    }
}

#[derive(Debug)]
struct Head {
    conv_down: ConvBatchnormLayer,
    conv_up: nn::ConvTranspose2D,
    conv_up_norm: nn::BatchNorm,
    conv_final: nn::ConvTranspose2D,
}

impl Head {
    fn new(path: &nn::Path<'_>, config: &PPOCRV6MediumDetConfig) -> Result<Self> {
        let in_channels = config.neck_out_channels;
        Ok(Self {
            conv_down: ConvBatchnormLayer::new(
                &(path / "conv_down"),
                in_channels,
                in_channels / 4,
                config.kernel_list[0],
                1,
                config.kernel_list[0] / 2,
                false,
            )?,
            conv_up: nn::conv_transpose2d(
                path / "conv_up" / "convolution",
                in_channels / 4,
                in_channels / 4,
                config.kernel_list[1],
                nn::ConvTransposeConfig {
                    stride: 2,
                    ..Default::default()
                },
            )?,
            conv_up_norm: nn::batch_norm2d(
                path / "conv_up" / "norm",
                in_channels / 4,
                Default::default(),
            )?,
            conv_final: nn::conv_transpose2d(
                path / "conv_final",
                in_channels / 4,
                1,
                config.kernel_list[2],
                nn::ConvTransposeConfig {
                    stride: 2,
                    ..Default::default()
                },
            )?,
        })
    }

    fn forward(&self, hidden_states: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = self.conv_down.forward(hidden_states)?;
        let hidden_states = self.conv_up.forward(&hidden_states)?;
        let hidden_states = self.conv_up_norm.forward_t(&hidden_states, false)?.relu()?;
        self.conv_final.forward(&hidden_states)?.sigmoid()
    }
}
