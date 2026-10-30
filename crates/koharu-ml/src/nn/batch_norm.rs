//! Batch normalization.

use std::borrow::Borrow;

use koharu_torch::{Kind, Result, TchError, Tensor};

#[derive(Debug, Clone, Copy)]
pub struct BatchNormConfig {
    pub cudnn_enabled: bool,
    pub eps: f64,
    pub momentum: f64,
    pub affine: bool,
}

impl Default for BatchNormConfig {
    fn default() -> Self {
        BatchNormConfig {
            cudnn_enabled: true,
            eps: 1e-5,
            momentum: 0.1,
            affine: true,
        }
    }
}

#[derive(Debug)]
pub struct BatchNorm {
    config: BatchNormConfig,
    pub running_mean: Tensor,
    pub running_var: Tensor,
    pub ws: Option<Tensor>,
    pub bs: Option<Tensor>,
    pub nd: usize,
}

fn batch_norm<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    nd: usize,
    out_dim: i64,
    config: BatchNormConfig,
) -> Result<BatchNorm> {
    let vs = vs.borrow();
    let (ws, bs) = if config.affine {
        (
            Some(vs.var("weight", &[out_dim])?),
            Some(vs.var("bias", &[out_dim])?),
        )
    } else {
        (None, None)
    };
    // Inference doesn't read it, but PyTorch's `_NormBase` registers it
    // beside the running statistics, so a checkpoint has it, and fills it
    // with 0 when a checkpoint without a version leaves it out:
    // https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/torch/nn/modules/batchnorm.py#L66-L84
    // https://github.com/pytorch/pytorch/blob/cf30153c4c131c8164ee7798e5022d810682e2cb/torch/nn/modules/batchnorm.py#L117-L141
    vs.optional_var_with_kind("num_batches_tracked", &[], Kind::Int64)?;
    Ok(BatchNorm {
        config,
        running_mean: vs.var("running_mean", &[out_dim])?,
        running_var: vs.var("running_var", &[out_dim])?,
        ws,
        bs,
        nd,
    })
}

pub fn batch_norm2d<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    out_dim: i64,
    config: BatchNormConfig,
) -> Result<BatchNorm> {
    batch_norm(vs, 2, out_dim, config)
}

impl super::ModuleT for BatchNorm {
    fn forward_t(&self, xs: &Tensor, train: bool) -> Result<Tensor> {
        let dim = xs.dim();
        let expected = if self.nd == 1 {
            dim == 2 || dim == 3
        } else {
            dim == self.nd + 2
        };
        if !expected {
            return Err(TchError::Shape(format!(
                "a {}d batch norm can't take a {:?} input",
                self.nd,
                xs.size()
            )));
        }
        // The process-wide switch the runtime sets once, which keeps MIOpen
        // off where it isn't reliable.
        let cudnn = koharu_torch::Cuda::user_enabled_cudnn();
        Tensor::f_batch_norm(
            xs,
            self.ws.as_ref(),
            self.bs.as_ref(),
            Some(&self.running_mean),
            Some(&self.running_var),
            train,
            self.config.momentum,
            self.config.eps,
            self.config.cudnn_enabled && cudnn,
        )
    }
}
