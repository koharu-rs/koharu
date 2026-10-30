//! Layer normalization.

use std::borrow::Borrow;

use koharu_torch::{Result, Tensor};

#[derive(Debug, Clone, Copy)]
pub struct LayerNormConfig {
    pub cudnn_enabled: bool,
    pub eps: f64,
    pub elementwise_affine: bool,
}

impl Default for LayerNormConfig {
    fn default() -> Self {
        LayerNormConfig {
            cudnn_enabled: true,
            eps: 1e-5,
            elementwise_affine: true,
        }
    }
}

#[derive(Debug)]
pub struct LayerNorm {
    config: LayerNormConfig,
    pub ws: Option<Tensor>,
    pub bs: Option<Tensor>,
    pub normalized_shape: Vec<i64>,
}

pub fn layer_norm<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    normalized_shape: Vec<i64>,
    config: LayerNormConfig,
) -> Result<LayerNorm> {
    let vs = vs.borrow();
    let (ws, bs) = if config.elementwise_affine {
        (
            Some(vs.var("weight", &normalized_shape)?),
            Some(vs.var("bias", &normalized_shape)?),
        )
    } else {
        (None, None)
    };
    Ok(LayerNorm {
        config,
        ws,
        bs,
        normalized_shape,
    })
}

impl super::Module for LayerNorm {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        Tensor::f_layer_norm(
            xs,
            self.normalized_shape.as_slice(),
            self.ws.as_ref(),
            self.bs.as_ref(),
            self.config.eps,
            self.config.cudnn_enabled,
        )
    }
}
