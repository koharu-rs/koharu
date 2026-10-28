//! Group normalization.

use std::borrow::Borrow;

use crate::{Result, Tensor};

#[derive(Debug, Clone, Copy)]
pub struct GroupNormConfig {
    pub cudnn_enabled: bool,
    pub eps: f64,
    pub affine: bool,
}

impl Default for GroupNormConfig {
    fn default() -> Self {
        GroupNormConfig {
            cudnn_enabled: true,
            eps: 1e-5,
            affine: true,
        }
    }
}

#[derive(Debug)]
pub struct GroupNorm {
    config: GroupNormConfig,
    pub ws: Option<Tensor>,
    pub bs: Option<Tensor>,
    pub num_groups: i64,
    pub num_channels: i64,
}

pub fn group_norm<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    num_groups: i64,
    num_channels: i64,
    config: GroupNormConfig,
) -> Result<GroupNorm> {
    let vs = vs.borrow();
    let (ws, bs) = if config.affine {
        (
            Some(vs.var("weight", &[num_channels])?),
            Some(vs.var("bias", &[num_channels])?),
        )
    } else {
        (None, None)
    };
    Ok(GroupNorm {
        config,
        ws,
        bs,
        num_groups,
        num_channels,
    })
}

impl super::Module for GroupNorm {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        Tensor::group_norm(
            xs,
            self.num_groups,
            self.ws.as_ref(),
            self.bs.as_ref(),
            self.config.eps,
            self.config.cudnn_enabled,
        )
    }
}
