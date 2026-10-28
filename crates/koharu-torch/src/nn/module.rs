//! The forward pass.

use crate::{Result, Tensor};

pub trait Module: std::fmt::Debug + Send {
    fn forward(&self, xs: &Tensor) -> Result<Tensor>;
}

/// A module whose forward pass differs in training, such as batch
/// normalization's; Koharu passes `false`.
pub trait ModuleT: std::fmt::Debug + Send {
    fn forward_t(&self, xs: &Tensor, train: bool) -> Result<Tensor>;
}

impl<T: Module> ModuleT for T {
    fn forward_t(&self, xs: &Tensor, _train: bool) -> Result<Tensor> {
        self.forward(xs)
    }
}

impl Tensor {
    pub fn apply<M: Module>(&self, m: &M) -> Result<Tensor> {
        m.forward(self)
    }

    pub fn apply_t<M: ModuleT>(&self, m: &M, train: bool) -> Result<Tensor> {
        m.forward_t(self, train)
    }

    pub fn apply_opt<M: Module>(&self, m: &Option<M>) -> Result<Tensor> {
        match m {
            Some(m) => m.forward(self),
            None => self.alias(),
        }
    }

    pub fn apply_opt_t<M: ModuleT>(&self, m: &Option<M>, train: bool) -> Result<Tensor> {
        match m {
            Some(m) => m.forward_t(self, train),
            None => self.alias(),
        }
    }
}
