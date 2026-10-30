//! Convolutions.

use std::borrow::Borrow;

use super::Path;
use koharu_torch::{Result, Tensor};

#[derive(Debug, Clone, Copy)]
pub enum PaddingMode {
    Zeros,
    Reflect,
}

impl PaddingMode {
    // libtorch's names for the modes; zeros pads with a constant.
    fn name(self) -> &'static str {
        match self {
            PaddingMode::Zeros => "constant",
            PaddingMode::Reflect => "reflect",
        }
    }

    pub fn pad(self, xs: &Tensor, reversed_padding_repeated_twice: &[i64]) -> Result<Tensor> {
        xs.f_pad(reversed_padding_repeated_twice, self.name(), None)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConvConfigND<ND> {
    pub stride: ND,
    pub padding: ND,
    pub dilation: ND,
    pub groups: i64,
    pub bias: bool,
    pub padding_mode: PaddingMode,
}

pub type ConvConfig = ConvConfigND<i64>;

impl Default for ConvConfig {
    fn default() -> Self {
        ConvConfig {
            stride: 1,
            padding: 0,
            dilation: 1,
            groups: 1,
            bias: true,
            padding_mode: PaddingMode::Zeros,
        }
    }
}

impl Default for ConvConfigND<[i64; 2]> {
    fn default() -> Self {
        ConvConfigND::<[i64; 2]> {
            stride: [1, 1],
            padding: [0, 0],
            dilation: [1, 1],
            groups: 1,
            bias: true,
            padding_mode: PaddingMode::Zeros,
        }
    }
}

#[derive(Debug)]
pub struct Conv<ND> {
    pub ws: Tensor,
    pub bs: Option<Tensor>,
    reversed_padding_repeated_twice: Vec<i64>,
    config: ConvConfigND<ND>,
}

pub type Conv2D = Conv<[i64; 2]>;

pub fn conv<'a, ND: AsRef<[i64]>, T: Borrow<Path<'a>>>(
    vs: T,
    in_dim: i64,
    out_dim: i64,
    ksizes: ND,
    config: ConvConfigND<ND>,
) -> Result<Conv<ND>> {
    let vs = vs.borrow();
    let bs = if config.bias {
        Some(vs.var("bias", &[out_dim])?)
    } else {
        None
    };
    let mut weight_size = vec![out_dim, in_dim / config.groups];
    weight_size.extend(ksizes.as_ref().iter());
    let ws = vs.var("weight", &weight_size)?;
    let padding = config.padding.as_ref().iter().rev();
    let reversed_padding_repeated_twice = padding.clone().chain(padding).copied().collect();
    Ok(Conv {
        ws,
        bs,
        config,
        reversed_padding_repeated_twice,
    })
}

trait Create: AsRef<[i64]> + Sized {
    fn make_array(i: i64) -> Self;

    fn conv<'a, T: Borrow<Path<'a>>>(
        vs: T,
        in_dim: i64,
        out_dim: i64,
        ksize: i64,
        config: ConvConfig,
    ) -> Result<Conv<Self>> {
        let config = ConvConfigND::<Self> {
            stride: Self::make_array(config.stride),
            padding: Self::make_array(config.padding),
            dilation: Self::make_array(config.dilation),
            groups: config.groups,
            bias: config.bias,
            padding_mode: config.padding_mode,
        };
        conv(vs, in_dim, out_dim, Self::make_array(ksize), config)
    }
}

impl Create for [i64; 2] {
    fn make_array(i: i64) -> Self {
        [i, i]
    }
}

pub fn conv2d<'a, T: Borrow<Path<'a>>>(
    vs: T,
    i: i64,
    o: i64,
    k: i64,
    c: ConvConfig,
) -> Result<Conv2D> {
    <[i64; 2]>::conv(vs, i, o, k, c)
}

macro_rules! impl_forward {
    ($conv:ty, $op:ident, $zero:expr) => {
        impl super::Module for $conv {
            fn forward(&self, xs: &Tensor) -> Result<Tensor> {
                let (xs, padding) = match self.config.padding_mode {
                    PaddingMode::Zeros => (xs.f_alias()?, self.config.padding),
                    mode => (mode.pad(xs, &self.reversed_padding_repeated_twice)?, $zero),
                };
                xs.$op(
                    &self.ws,
                    self.bs.as_ref(),
                    self.config.stride,
                    padding,
                    self.config.dilation,
                    self.config.groups,
                )
            }
        }
    };
}

impl_forward!(Conv2D, f_conv2d, [0, 0]);
