//! AOTGenerator ported from BallonsTranslator.
//!
//! https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/inpaint/aot.py#L21-L262

use std::path::Path;

use crate::nn::{self, Module};
use anyhow::Result;
use tch::{Device, Kind, Tensor};

const RELU_NF_SCALE: f64 = 1.713_958_859_443_664_6;
const WEIGHT_STANDARDIZATION_EPS: f64 = 1e-4;

#[derive(Debug)]
pub(super) struct Model {
    vs: nn::VarStore,
    head: [GatedWSConvPadded; 3],
    body_conv: Vec<AOTBlock>,
    tail: Tail,
}

impl Model {
    pub(super) fn new(device: Device, kind: Kind) -> Result<Self> {
        let vs = nn::VarStore::new(device, kind);
        let root = vs.root();
        let head = [
            GatedWSConvPadded::new(&(&root / "head" / 0), 4, 32, 3, 1, 1)?,
            GatedWSConvPadded::new(&(&root / "head" / 2), 32, 64, 4, 2, 1)?,
            GatedWSConvPadded::new(&(&root / "head" / 4), 64, 128, 4, 2, 1)?,
        ];
        let body_conv = (0..10)
            .map(|index| AOTBlock::new(&(&root / "body_conv" / index), 128))
            .collect::<Result<_>>()?;
        let tail = Tail::new(&(&root / "tail"))?;
        Ok(Self {
            vs,
            head,
            body_conv,
            tail,
        })
    }

    pub(super) fn load(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.vs.load(path)?;
        Ok(())
    }

    pub(super) fn forward(&self, image: &Tensor, mask: &Tensor) -> Result<Tensor> {
        let image = image.f_to_kind(self.vs.kind())?;
        let mask = mask.f_to_kind(self.vs.kind())?;
        let x = Tensor::f_cat(&[mask, image], 1)?;
        let x = relu_nf(self.head[0].forward(&x)?)?;
        let x = relu_nf(self.head[1].forward(&x)?)?;
        let mut x = self.head[2].forward(&x)?;
        for block in &self.body_conv {
            x = block.forward(&x)?;
        }
        Ok(self.tail.forward(&x)?.f_clamp(-1.0, 1.0)?)
    }
}

#[derive(Debug)]
struct ScaledWSConv2d {
    conv: nn::Conv2D,
    gain: Tensor,
    stride: i64,
    dilation: i64,
}

impl ScaledWSConv2d {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        dilation: i64,
    ) -> Result<Self> {
        let conv = nn::conv2d(
            path,
            in_channels,
            out_channels,
            kernel_size,
            nn::ConvConfig {
                stride,
                dilation,
                ..Default::default()
            },
        )?;
        let gain = path.var("gain", &[out_channels, 1, 1, 1])?;
        Ok(Self {
            conv,
            gain,
            stride,
            dilation,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        input.f_conv2d(
            &standardized_weight(&self.conv.ws, &self.gain)?,
            self.conv.bs.as_ref(),
            [self.stride, self.stride],
            [0, 0],
            [self.dilation, self.dilation],
            1,
        )
    }
}

#[derive(Debug)]
struct ScaledWSTransposeConv2d {
    conv: nn::ConvTranspose2D,
    gain: Tensor,
    stride: i64,
    padding: i64,
}

impl ScaledWSTransposeConv2d {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
    ) -> Result<Self> {
        let padding = (kernel_size - 1) / 2;
        let conv = nn::conv_transpose2d(
            path,
            in_channels,
            out_channels,
            kernel_size,
            nn::ConvTransposeConfig {
                stride,
                padding,
                ..Default::default()
            },
        )?;
        let gain = path.var("gain", &[in_channels, 1, 1, 1])?;
        Ok(Self {
            conv,
            gain,
            stride,
            padding,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        input.f_conv_transpose2d(
            &standardized_weight(&self.conv.ws, &self.gain)?,
            self.conv.bs.as_ref(),
            [self.stride, self.stride],
            [self.padding, self.padding],
            [0, 0],
            1,
            [1, 1],
        )
    }
}

#[derive(Debug)]
struct GatedWSConvPadded {
    conv: ScaledWSConv2d,
    conv_gate: ScaledWSConv2d,
    padding: i64,
}

impl GatedWSConvPadded {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
        dilation: i64,
    ) -> Result<Self> {
        Ok(Self {
            conv: ScaledWSConv2d::new(
                &(path / "conv"),
                in_channels,
                out_channels,
                kernel_size,
                stride,
                dilation,
            )?,
            conv_gate: ScaledWSConv2d::new(
                &(path / "conv_gate"),
                in_channels,
                out_channels,
                kernel_size,
                stride,
                dilation,
            )?,
            padding: ((kernel_size - 1) * dilation) / 2,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let input =
            input.f_reflection_pad2d([self.padding, self.padding, self.padding, self.padding])?;
        ((self.conv.forward(&input)?).f_mul(&self.conv_gate.forward(&input)?.f_sigmoid()?)?)
            .f_mul_scalar(1.8)
    }
}

#[derive(Debug)]
struct GatedWSTransposeConvPadded {
    conv: ScaledWSTransposeConv2d,
    conv_gate: ScaledWSTransposeConv2d,
}

impl GatedWSTransposeConvPadded {
    fn new(
        path: &nn::Path<'_>,
        in_channels: i64,
        out_channels: i64,
        kernel_size: i64,
        stride: i64,
    ) -> Result<Self> {
        Ok(Self {
            conv: ScaledWSTransposeConv2d::new(
                &(path / "conv"),
                in_channels,
                out_channels,
                kernel_size,
                stride,
            )?,
            conv_gate: ScaledWSTransposeConv2d::new(
                &(path / "conv_gate"),
                in_channels,
                out_channels,
                kernel_size,
                stride,
            )?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        (self
            .conv
            .forward(input)?
            .f_mul(&self.conv_gate.forward(input)?.f_sigmoid()?)?)
        .f_mul_scalar(1.8)
    }
}

#[derive(Debug)]
struct PaddedConvRelu {
    conv: nn::Conv2D,
    padding: i64,
}

impl PaddedConvRelu {
    fn new(path: &nn::Path<'_>, channels: i64, dilation: i64) -> Result<Self> {
        Ok(Self {
            conv: nn::conv2d(
                path,
                channels,
                channels / 4,
                3,
                nn::ConvConfig {
                    dilation,
                    ..Default::default()
                },
            )?,
            padding: dilation,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        self.conv
            .forward(&input.f_reflection_pad2d([
                self.padding,
                self.padding,
                self.padding,
                self.padding,
            ])?)?
            .f_relu()
    }
}

#[derive(Debug)]
struct PaddedConv {
    conv: nn::Conv2D,
}

impl PaddedConv {
    fn new(path: &nn::Path<'_>, channels: i64) -> Result<Self> {
        Ok(Self {
            conv: nn::conv2d(path, channels, channels, 3, Default::default())?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        self.conv.forward(&input.f_reflection_pad2d([1, 1, 1, 1])?)
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
struct AOTBlock {
    blocks: [PaddedConvRelu; 4],
    fuse: PaddedConv,
    gate: PaddedConv,
}

impl AOTBlock {
    fn new(path: &nn::Path<'_>, channels: i64) -> Result<Self> {
        let [block00, block01, block02, block03] = std::array::from_fn(|index| {
            PaddedConvRelu::new(
                &(path / format!("block{index:02}") / 1),
                channels,
                [2, 4, 8, 16][index],
            )
        });
        Ok(Self {
            blocks: [block00?, block01?, block02?, block03?],
            fuse: PaddedConv::new(&(path / "fuse" / 1), channels)?,
            gate: PaddedConv::new(&(path / "gate" / 1), channels)?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let branches = self
            .blocks
            .iter()
            .map(|block| block.forward(input))
            .collect::<tch::Result<Vec<_>>>()?;
        let fused = self.fuse.forward(&Tensor::f_cat(&branches, 1)?)?;
        let mask = my_layer_norm(&self.gate.forward(input)?)?.f_sigmoid()?;
        (input.f_mul(&mask.f_ones_like()?.f_sub(&mask)?)?).f_add(&fused.f_mul(&mask)?)
    }
}

#[derive(Debug)]
struct Tail {
    conv0: GatedWSConvPadded,
    conv1: GatedWSConvPadded,
    up0: GatedWSTransposeConvPadded,
    up1: GatedWSTransposeConvPadded,
    output: GatedWSConvPadded,
}

impl Tail {
    fn new(path: &nn::Path<'_>) -> Result<Self> {
        Ok(Self {
            conv0: GatedWSConvPadded::new(&(path / 0), 128, 128, 3, 1, 1)?,
            conv1: GatedWSConvPadded::new(&(path / 2), 128, 128, 3, 1, 1)?,
            up0: GatedWSTransposeConvPadded::new(&(path / 4), 128, 64, 4, 2)?,
            up1: GatedWSTransposeConvPadded::new(&(path / 6), 64, 32, 4, 2)?,
            output: GatedWSConvPadded::new(&(path / 8), 32, 3, 3, 1, 1)?,
        })
    }

    fn forward(&self, input: &Tensor) -> tch::Result<Tensor> {
        let x = relu_nf(self.conv0.forward(input)?)?;
        let x = relu_nf(self.conv1.forward(&x)?)?;
        let x = relu_nf(self.up0.forward(&x)?)?;
        let x = relu_nf(self.up1.forward(&x)?)?;
        self.output.forward(&x)
    }
}

fn standardized_weight(weight: &Tensor, gain: &Tensor) -> tch::Result<Tensor> {
    let (variance, mean) = weight.f_var_mean_dim([1_i64, 2, 3].as_slice(), true, true)?;
    let fan_in = weight.size()[1..].iter().product::<i64>() as f64;
    let scale = (variance
        .f_mul_scalar(fan_in)?
        .f_clamp_min(WEIGHT_STANDARDIZATION_EPS)?
        .f_rsqrt()?)
    .f_mul(gain)?;
    (weight.f_mul(&scale)?).f_sub(&mean.f_mul(&scale)?)
}

fn relu_nf(input: Tensor) -> tch::Result<Tensor> {
    input.f_relu()?.f_mul_scalar(RELU_NF_SCALE)
}

fn my_layer_norm(input: &Tensor) -> tch::Result<Tensor> {
    let dimensions = [2_i64, 3];
    let mean = input.f_mean_dim(dimensions.as_slice(), true, None)?;
    let std = input
        .f_std_dim(dimensions.as_slice(), true, true)?
        .f_add_scalar(1e-9)?;
    (input
        .f_sub(&mean)?
        .f_mul_scalar(2.0)?
        .f_div(&std)?
        .f_sub_scalar(1.0)?)
    .f_mul_scalar(5.0)
}
