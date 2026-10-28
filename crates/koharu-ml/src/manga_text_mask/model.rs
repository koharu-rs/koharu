//! Inference-only port of the Manga Text Segmentation 2025 network used by the mask generator.
//!
//! The public checkpoint is constructed by this exact script:
//! https://huggingface.co/a-b-c-x-y-z/Manga-Text-Segmentation-2025/blob/2dde9eeb03e81692c1562059451c2bf30e1a13da/inference.py
//!
//! Its model is segmentation-models-pytorch's Unet++ decoder:
//! https://github.com/qubvel-org/segmentation_models.pytorch/blob/420ce84b0c2df0286fa9bb2bd1499eea625c9b33/segmentation_models_pytorch/decoders/unetplusplus/decoder.py
//! backed by timm's EfficientNetV2-RW-M:
//! https://github.com/huggingface/pytorch-image-models/blob/e44f14d7d2f557b9f3add82ee4f1ed2beefbb30d/timm/models/efficientnet.py#L812-L846

use std::path::Path;

use anyhow::Result;
use koharu_torch::{
    Device, Kind, Tensor,
    nn::{self, Module, ModuleT},
};

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    encoder: EfficientNetEncoder,
    decoder: UnetPlusPlusDecoder,
    segmentation_head: nn::Conv2D,
}

impl Model {
    pub(super) fn new(device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();
        let encoder = EfficientNetEncoder::new(&(&root / "encoder" / "model"))?;
        let decoder = UnetPlusPlusDecoder::new(&(&root / "decoder"))?;
        let segmentation_head = conv2d(&(&root / "segmentation_head" / 0), 16, 1, 3, 1, 1, true)?;
        Ok(Self {
            vs,
            encoder,
            decoder,
            segmentation_head,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub(super) fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let features = self.encoder.forward(&input.to_kind(self.vs.kind())?)?;
        self.segmentation_head
            .forward(&self.decoder.forward(&features)?)
    }
}

#[derive(Debug)]
struct EfficientNetEncoder {
    conv_stem: nn::Conv2D,
    bn1: nn::BatchNorm,
    blocks: Vec<EfficientNetStage>,
}

impl EfficientNetEncoder {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        let conv_stem = conv2d(&(path / "conv_stem"), 3, 32, 3, 2, 1, false)?;
        let bn1 = batch_norm(&(path / "bn1"), 32)?;

        // `efficientnetv2_rw_m` scales the RW-S definition with a 1.2 channel
        // multiplier and per-stage depth multipliers [1.2; 4] + [1.6; 2].
        // Keep these concrete values beside construction: they determine both
        // checkpoint paths and the feature maps selected by timm `features_only`.
        let stages = [
            Stage::edge(3, 32, 32, 1, 1),
            Stage::edge(5, 32, 56, 2, 4),
            Stage::edge(5, 56, 80, 2, 4),
            Stage::inverted(8, 80, 152, 2, 4),
            Stage::inverted(15, 152, 192, 1, 6),
            Stage::inverted(24, 192, 328, 2, 6),
        ];
        let blocks = stages
            .into_iter()
            .enumerate()
            .map(|(stage_index, stage)| stage.build(&(path / "blocks" / stage_index)))
            .collect::<Result<_>>()?;

        Ok(Self {
            conv_stem,
            bn1,
            blocks,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<EncoderFeatures> {
        let mut hidden_states = silu(self.bn1.forward_t(&self.conv_stem.forward(input)?, false)?)?;
        let mut selected = Vec::with_capacity(5);
        for (stage_index, stage) in self.blocks.iter().enumerate() {
            hidden_states = stage.forward(hidden_states)?;
            // timm coalesces the two stride-16 stages and exposes the latter.
            if matches!(stage_index, 0 | 1 | 2 | 4 | 5) {
                selected.push(hidden_states.alias()?);
            }
        }
        Ok(EncoderFeatures {
            stage0: selected.remove(0),
            stage1: selected.remove(0),
            stage2: selected.remove(0),
            stage4: selected.remove(0),
            stage5: selected.remove(0),
        })
    }
}

#[derive(Debug)]
struct EncoderFeatures {
    stage0: Tensor,
    stage1: Tensor,
    stage2: Tensor,
    stage4: Tensor,
    stage5: Tensor,
}

#[derive(Debug, Clone, Copy)]
enum StageKind {
    Edge,
    Inverted,
}

#[derive(Debug, Clone, Copy)]
struct Stage {
    kind: StageKind,
    repeats: usize,
    input_channels: i64,
    output_channels: i64,
    stride: i64,
    expansion: i64,
}

impl Stage {
    const fn edge(
        repeats: usize,
        input_channels: i64,
        output_channels: i64,
        stride: i64,
        expansion: i64,
    ) -> Self {
        Self {
            kind: StageKind::Edge,
            repeats,
            input_channels,
            output_channels,
            stride,
            expansion,
        }
    }

    const fn inverted(
        repeats: usize,
        input_channels: i64,
        output_channels: i64,
        stride: i64,
        expansion: i64,
    ) -> Self {
        Self {
            kind: StageKind::Inverted,
            repeats,
            input_channels,
            output_channels,
            stride,
            expansion,
        }
    }

    fn build(self, path: &nn::Path<'_>) -> Result<EfficientNetStage> {
        Ok(match self.kind {
            StageKind::Edge => EfficientNetStage::Edge(
                (0..self.repeats)
                    .map(|index| {
                        EdgeResidual::new(
                            &(path / index),
                            if index == 0 {
                                self.input_channels
                            } else {
                                self.output_channels
                            },
                            self.output_channels,
                            if index == 0 { self.stride } else { 1 },
                            self.expansion,
                        )
                    })
                    .collect::<Result<_>>()?,
            ),
            StageKind::Inverted => EfficientNetStage::Inverted(
                (0..self.repeats)
                    .map(|index| {
                        InvertedResidual::new(
                            &(path / index),
                            if index == 0 {
                                self.input_channels
                            } else {
                                self.output_channels
                            },
                            self.output_channels,
                            if index == 0 { self.stride } else { 1 },
                            self.expansion,
                        )
                    })
                    .collect::<Result<_>>()?,
            ),
        })
    }
}

#[derive(Debug)]
enum EfficientNetStage {
    Edge(Vec<EdgeResidual>),
    Inverted(Vec<InvertedResidual>),
}

impl EfficientNetStage {
    fn forward(&self, input: Tensor) -> koharu_torch::Result<Tensor> {
        match self {
            Self::Edge(blocks) => blocks
                .iter()
                .try_fold(input, |input, block| block.forward(&input)),
            Self::Inverted(blocks) => blocks
                .iter()
                .try_fold(input, |input, block| block.forward(&input)),
        }
    }
}

/// timm `EdgeResidual`, also called FusedMBConv.
/// https://github.com/huggingface/pytorch-image-models/blob/e44f14d7d2f557b9f3add82ee4f1ed2beefbb30d/timm/models/_efficientnet_blocks.py#L628-L699
#[derive(Debug)]
struct EdgeResidual {
    conv_exp: nn::Conv2D,
    bn1: nn::BatchNorm,
    conv_pwl: nn::Conv2D,
    bn2: nn::BatchNorm,
    has_skip: bool,
}

impl EdgeResidual {
    fn new(
        path: &nn::Path<'_>,
        input_channels: i64,
        output_channels: i64,
        stride: i64,
        expansion: i64,
    ) -> Result<Self> {
        let expanded_channels = input_channels * expansion;
        Ok(Self {
            conv_exp: conv2d(
                &(path / "conv_exp"),
                input_channels,
                expanded_channels,
                3,
                stride,
                1,
                false,
            )?,
            bn1: batch_norm(&(path / "bn1"), expanded_channels)?,
            conv_pwl: conv2d(
                &(path / "conv_pwl"),
                expanded_channels,
                output_channels,
                1,
                1,
                0,
                false,
            )?,
            bn2: batch_norm(&(path / "bn2"), output_channels)?,
            has_skip: stride == 1 && input_channels == output_channels,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = silu(self.bn1.forward_t(&self.conv_exp.forward(input)?, false)?)?;
        let hidden_states = self
            .bn2
            .forward_t(&self.conv_pwl.forward(&hidden_states)?, false)?;
        if self.has_skip {
            hidden_states + input
        } else {
            Ok(hidden_states)
        }
    }
}

/// timm `InvertedResidual` MBConv with squeeze/excitation.
/// https://github.com/huggingface/pytorch-image-models/blob/e44f14d7d2f557b9f3add82ee4f1ed2beefbb30d/timm/models/_efficientnet_blocks.py#L201-L298
#[derive(Debug)]
struct InvertedResidual {
    conv_pw: nn::Conv2D,
    bn1: nn::BatchNorm,
    conv_dw: nn::Conv2D,
    bn2: nn::BatchNorm,
    se: SqueezeExcite,
    conv_pwl: nn::Conv2D,
    bn3: nn::BatchNorm,
    has_skip: bool,
}

impl InvertedResidual {
    fn new(
        path: &nn::Path<'_>,
        input_channels: i64,
        output_channels: i64,
        stride: i64,
        expansion: i64,
    ) -> Result<Self> {
        let expanded_channels = input_channels * expansion;
        Ok(Self {
            conv_pw: conv2d(
                &(path / "conv_pw"),
                input_channels,
                expanded_channels,
                1,
                1,
                0,
                false,
            )?,
            bn1: batch_norm(&(path / "bn1"), expanded_channels)?,
            conv_dw: nn::conv2d(
                path / "conv_dw",
                expanded_channels,
                expanded_channels,
                3,
                nn::ConvConfig {
                    stride,
                    padding: 1,
                    groups: expanded_channels,
                    bias: false,
                    ..Default::default()
                },
            )?,
            bn2: batch_norm(&(path / "bn2"), expanded_channels)?,
            // timm's builder derives SE width from the pre-expansion input.
            se: SqueezeExcite::new(&(path / "se"), expanded_channels, input_channels / 4)?,
            conv_pwl: conv2d(
                &(path / "conv_pwl"),
                expanded_channels,
                output_channels,
                1,
                1,
                0,
                false,
            )?,
            bn3: batch_norm(&(path / "bn3"), output_channels)?,
            has_skip: stride == 1 && input_channels == output_channels,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let hidden_states = silu(self.bn1.forward_t(&self.conv_pw.forward(input)?, false)?)?;
        let hidden_states = silu(
            self.bn2
                .forward_t(&self.conv_dw.forward(&hidden_states)?, false)?,
        )?;
        let hidden_states = self.se.forward(&hidden_states)?;
        let hidden_states = self
            .bn3
            .forward_t(&self.conv_pwl.forward(&hidden_states)?, false)?;
        if self.has_skip {
            hidden_states + input
        } else {
            Ok(hidden_states)
        }
    }
}

#[derive(Debug)]
struct SqueezeExcite {
    conv_reduce: nn::Conv2D,
    conv_expand: nn::Conv2D,
}

impl SqueezeExcite {
    fn new(path: &nn::Path<'_>, input_channels: i64, reduced_channels: i64) -> Result<Self> {
        Ok(Self {
            conv_reduce: conv2d(
                &(path / "conv_reduce"),
                input_channels,
                reduced_channels,
                1,
                1,
                0,
                true,
            )?,
            conv_expand: conv2d(
                &(path / "conv_expand"),
                reduced_channels,
                input_channels,
                1,
                1,
                0,
                true,
            )?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let scale = input.mean_dim([2_i64, 3].as_slice(), true, None)?;
        let scale = silu(self.conv_reduce.forward(&scale)?)?;
        input * self.conv_expand.forward(&scale)?.sigmoid()?
    }
}

#[derive(Debug)]
struct UnetPlusPlusDecoder {
    x_0_0: DecoderBlock,
    x_0_1: DecoderBlock,
    x_1_1: DecoderBlock,
    x_0_2: DecoderBlock,
    x_1_2: DecoderBlock,
    x_2_2: DecoderBlock,
    x_0_3: DecoderBlock,
    x_1_3: DecoderBlock,
    x_2_3: DecoderBlock,
    x_3_3: DecoderBlock,
    x_0_4: DecoderBlock,
}

impl UnetPlusPlusDecoder {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        let blocks = path / "blocks";
        Ok(Self {
            x_0_0: DecoderBlock::new(&(&blocks / "x_0_0"), 328, 192, 256)?,
            x_0_1: DecoderBlock::new(&(&blocks / "x_0_1"), 256, 160, 128)?,
            x_1_1: DecoderBlock::new(&(&blocks / "x_1_1"), 192, 80, 80)?,
            x_0_2: DecoderBlock::new(&(&blocks / "x_0_2"), 128, 168, 64)?,
            x_1_2: DecoderBlock::new(&(&blocks / "x_1_2"), 80, 112, 56)?,
            x_2_2: DecoderBlock::new(&(&blocks / "x_2_2"), 80, 56, 56)?,
            x_0_3: DecoderBlock::new(&(&blocks / "x_0_3"), 64, 128, 32)?,
            x_1_3: DecoderBlock::new(&(&blocks / "x_1_3"), 56, 96, 32)?,
            x_2_3: DecoderBlock::new(&(&blocks / "x_2_3"), 56, 64, 32)?,
            x_3_3: DecoderBlock::new(&(&blocks / "x_3_3"), 56, 32, 32)?,
            x_0_4: DecoderBlock::new(&(&blocks / "x_0_4"), 32, 0, 16)?,
        })
    }

    fn forward(&self, features: &EncoderFeatures) -> koharu_torch::Result<Tensor> {
        let x_0_0 = self
            .x_0_0
            .forward(&features.stage5, Some(&features.stage4))?;
        let x_1_1 = self
            .x_1_1
            .forward(&features.stage4, Some(&features.stage2))?;
        let x_2_2 = self
            .x_2_2
            .forward(&features.stage2, Some(&features.stage1))?;
        let x_3_3 = self
            .x_3_3
            .forward(&features.stage1, Some(&features.stage0))?;

        let skip = Tensor::cat(&[x_1_1.alias()?, features.stage2.alias()?], 1)?;
        let x_0_1 = self.x_0_1.forward(&x_0_0, Some(&skip))?;
        let skip = Tensor::cat(&[x_2_2.alias()?, features.stage1.alias()?], 1)?;
        let x_1_2 = self.x_1_2.forward(&x_1_1, Some(&skip))?;
        let skip = Tensor::cat(&[x_3_3.alias()?, features.stage0.alias()?], 1)?;
        let x_2_3 = self.x_2_3.forward(&x_2_2, Some(&skip))?;

        let skip = Tensor::cat(
            &[x_1_2.alias()?, x_2_2.alias()?, features.stage1.alias()?],
            1,
        )?;
        let x_0_2 = self.x_0_2.forward(&x_0_1, Some(&skip))?;
        let skip = Tensor::cat(
            &[x_2_3.alias()?, x_3_3.alias()?, features.stage0.alias()?],
            1,
        )?;
        let x_1_3 = self.x_1_3.forward(&x_1_2, Some(&skip))?;

        let skip = Tensor::cat(&[x_1_3, x_2_3, x_3_3, features.stage0.alias()?], 1)?;
        let x_0_3 = self.x_0_3.forward(&x_0_2, Some(&skip))?;
        self.x_0_4.forward(&x_0_3, None)
    }
}

#[derive(Debug)]
struct DecoderBlock {
    conv1: ConvGroupNormRelu,
    attention1: Scse,
    conv2: ConvGroupNormRelu,
    attention2: Scse,
}

impl DecoderBlock {
    fn new(
        path: &nn::Path<'_>,
        input_channels: i64,
        skip_channels: i64,
        output_channels: i64,
    ) -> Result<Self> {
        let combined_channels = input_channels + skip_channels;
        Ok(Self {
            conv1: ConvGroupNormRelu::new(&(path / "conv1"), combined_channels, output_channels)?,
            attention1: Scse::new(&(path / "attention1" / "attention"), combined_channels)?,
            conv2: ConvGroupNormRelu::new(&(path / "conv2"), output_channels, output_channels)?,
            attention2: Scse::new(&(path / "attention2" / "attention"), output_channels)?,
        })
    }

    fn forward(&self, input: &Tensor, skip: Option<&Tensor>) -> koharu_torch::Result<Tensor> {
        let size = input.size();
        let mut hidden_states =
            input.upsample_nearest2d([size[2] * 2, size[3] * 2], None::<f64>, None::<f64>)?;
        if let Some(skip) = skip {
            hidden_states = self
                .attention1
                .forward(&Tensor::cat(&[hidden_states, skip.alias()?], 1)?)?;
        }
        let hidden_states = self.conv1.forward(&hidden_states)?;
        let hidden_states = self.conv2.forward(&hidden_states)?;
        self.attention2.forward(&hidden_states)
    }
}

#[derive(Debug)]
struct ConvGroupNormRelu {
    conv: nn::Conv2D,
    norm: nn::GroupNorm,
}

impl ConvGroupNormRelu {
    fn new(path: &nn::Path<'_>, input_channels: i64, output_channels: i64) -> Result<Self> {
        Ok(Self {
            conv: conv2d(&(path / 0), input_channels, output_channels, 3, 1, 1, false)?,
            // The original script recursively replaces every decoder BatchNorm
            // with GroupNorm. Every decoder width is divisible by eight.
            norm: nn::group_norm(path / 1, 8, output_channels, Default::default())?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        self.norm.forward(&self.conv.forward(input)?)?.relu()
    }
}

/// segmentation-models-pytorch SCSE attention.
/// https://github.com/qubvel-org/segmentation_models.pytorch/blob/420ce84b0c2df0286fa9bb2bd1499eea625c9b33/segmentation_models_pytorch/base/modules.py#L85-L101
#[derive(Debug)]
struct Scse {
    channel_reduce: nn::Conv2D,
    channel_expand: nn::Conv2D,
    spatial: nn::Conv2D,
}

impl Scse {
    fn new(path: &nn::Path<'_>, channels: i64) -> Result<Self> {
        Ok(Self {
            channel_reduce: conv2d(&(path / "cSE" / 1), channels, channels / 16, 1, 1, 0, true)?,
            channel_expand: conv2d(&(path / "cSE" / 3), channels / 16, channels, 1, 1, 0, true)?,
            spatial: conv2d(&(path / "sSE" / 0), channels, 1, 1, 1, 0, true)?,
        })
    }

    fn forward(&self, input: &Tensor) -> koharu_torch::Result<Tensor> {
        let channel = input.adaptive_avg_pool2d([1, 1])?;
        let channel = self.channel_reduce.forward(&channel)?.relu()?;
        let channel = self.channel_expand.forward(&channel)?.sigmoid()?;
        let spatial = self.spatial.forward(input)?.sigmoid()?;
        (input * channel)? + (input * spatial)?
    }
}

fn conv2d(
    path: &nn::Path<'_>,
    input_channels: i64,
    output_channels: i64,
    kernel_size: i64,
    stride: i64,
    padding: i64,
    bias: bool,
) -> Result<nn::Conv2D> {
    Ok(nn::conv2d(
        path,
        input_channels,
        output_channels,
        kernel_size,
        nn::ConvConfig {
            stride,
            padding,
            bias,
            ..Default::default()
        },
    )?)
}

fn batch_norm(path: &nn::Path<'_>, channels: i64) -> Result<nn::BatchNorm> {
    Ok(nn::batch_norm2d(path, channels, Default::default())?)
}

fn silu(input: Tensor) -> koharu_torch::Result<Tensor> {
    input.silu()
}
