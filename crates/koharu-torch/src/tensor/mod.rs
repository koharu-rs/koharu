//! A Torch tensor.

mod convert;
pub mod index;

pub use super::wrappers::tensor::{InferenceMode, Tensor, inference_mode};
pub use index::{IndexOp, NewAxis, TensorIndexer};
