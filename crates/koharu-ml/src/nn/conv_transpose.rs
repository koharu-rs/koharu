//! Transposed convolutions.

use std::borrow::Borrow;

use super::Path;
use koharu_torch::{Result, Tensor};

#[derive(Debug, Clone, Copy)]
pub struct ConvTransposeConfigND<ND> {
    pub stride: ND,
    pub padding: ND,
    pub output_padding: ND,
    pub groups: i64,
    pub bias: bool,
    pub dilation: ND,
}

pub type ConvTransposeConfig = ConvTransposeConfigND<i64>;

impl Default for ConvTransposeConfig {
    fn default() -> Self {
        ConvTransposeConfigND {
            stride: 1,
            padding: 0,
            output_padding: 0,
            dilation: 1,
            groups: 1,
            bias: true,
        }
    }
}

#[derive(Debug)]
pub struct ConvTransposeND<ND> {
    pub ws: Tensor,
    pub bs: Option<Tensor>,
    config: ConvTransposeConfigND<ND>,
}

pub type ConvTranspose2D = ConvTransposeND<[i64; 2]>;

fn conv_transpose<'a, ND: AsRef<[i64]>, T: Borrow<Path<'a>>>(
    vs: T,
    in_dim: i64,
    out_dim: i64,
    ksizes: ND,
    config: ConvTransposeConfigND<ND>,
) -> Result<ConvTransposeND<ND>> {
    let vs = vs.borrow();
    let bs = if config.bias {
        Some(vs.var("bias", &[out_dim])?)
    } else {
        None
    };
    let mut weight_size = vec![in_dim, out_dim / config.groups];
    weight_size.extend(ksizes.as_ref().iter());
    let ws = vs.var("weight", &weight_size)?;
    Ok(ConvTransposeND { ws, bs, config })
}

trait Create: AsRef<[i64]> + Sized {
    fn make_array(i: i64) -> Self;

    fn conv_transpose<'a, T: Borrow<Path<'a>>>(
        vs: T,
        in_dim: i64,
        out_dim: i64,
        ksize: i64,
        config: ConvTransposeConfig,
    ) -> Result<ConvTransposeND<Self>> {
        let config = ConvTransposeConfigND::<Self> {
            stride: Self::make_array(config.stride),
            padding: Self::make_array(config.padding),
            output_padding: Self::make_array(config.output_padding),
            dilation: Self::make_array(config.dilation),
            groups: config.groups,
            bias: config.bias,
        };
        conv_transpose(vs, in_dim, out_dim, Self::make_array(ksize), config)
    }
}

impl Create for [i64; 2] {
    fn make_array(i: i64) -> Self {
        [i, i]
    }
}

pub fn conv_transpose2d<'a, T: Borrow<Path<'a>>>(
    vs: T,
    i: i64,
    o: i64,
    k: i64,
    c: ConvTransposeConfig,
) -> Result<ConvTranspose2D> {
    <[i64; 2]>::conv_transpose(vs, i, o, k, c)
}

macro_rules! impl_forward {
    ($conv:ty, $op:ident) => {
        impl super::Module for $conv {
            fn forward(&self, xs: &Tensor) -> Result<Tensor> {
                xs.$op(
                    &self.ws,
                    self.bs.as_ref(),
                    self.config.stride,
                    self.config.padding,
                    self.config.output_padding,
                    self.config.groups,
                    self.config.dilation,
                )
            }
        }
    };
}

impl_forward!(ConvTranspose2D, f_conv_transpose2d);
