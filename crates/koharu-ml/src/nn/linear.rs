//! A fully connected layer.

use std::borrow::Borrow;

use koharu_torch::{Result, Tensor};

#[derive(Debug, Clone, Copy)]
pub struct LinearConfig {
    pub bias: bool,
}

impl Default for LinearConfig {
    fn default() -> Self {
        LinearConfig { bias: true }
    }
}

#[derive(Debug)]
pub struct Linear {
    pub ws: Tensor,
    pub bs: Option<Tensor>,
}

pub fn linear<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    in_dim: i64,
    out_dim: i64,
    c: LinearConfig,
) -> Result<Linear> {
    let vs = vs.borrow();
    let bs = if c.bias {
        Some(vs.var("bias", &[out_dim])?)
    } else {
        None
    };
    Ok(Linear {
        ws: vs.var("weight", &[out_dim, in_dim])?,
        bs,
    })
}

impl super::Module for Linear {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        xs.f_linear(&self.ws, self.bs.as_ref())
    }
}
