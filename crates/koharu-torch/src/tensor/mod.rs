//! A Torch tensor.

mod convert;
pub mod index;

pub use super::wrappers::tensor::{inference_mode, InferenceMode, Tensor};
pub use index::{IndexOp, NewAxis, TensorIndexer};
