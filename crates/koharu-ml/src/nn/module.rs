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

/// `tch`'s `apply` for these modules, which return a `Result`; `tch` names a
/// method's fallible form with `f_`.
pub trait Apply {
    fn f_apply<M: Module>(&self, m: &M) -> Result<Tensor>;
    fn f_apply_t<M: ModuleT>(&self, m: &M, train: bool) -> Result<Tensor>;
    fn f_apply_opt<M: Module>(&self, m: &Option<M>) -> Result<Tensor>;
}

impl Apply for Tensor {
    fn f_apply<M: Module>(&self, m: &M) -> Result<Tensor> {
        m.forward(self)
    }

    fn f_apply_t<M: ModuleT>(&self, m: &M, train: bool) -> Result<Tensor> {
        m.forward_t(self, train)
    }

    fn f_apply_opt<M: Module>(&self, m: &Option<M>) -> Result<Tensor> {
        match m {
            Some(m) => m.forward(self),
            None => self.f_alias(),
        }
    }
}
