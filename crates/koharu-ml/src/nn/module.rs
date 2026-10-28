//! The forward pass.

use tch::{Result, Tensor};

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
