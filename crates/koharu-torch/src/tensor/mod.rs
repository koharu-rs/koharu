//! A Torch tensor.

mod convert;
pub mod index;

pub use super::wrappers::tensor::{no_grad, no_grad_guard, NoGradGuard, Tensor};
pub use index::{IndexOp, NewAxis, TensorIndexer};
