//! libtorch for Koharu's models: a fork of `tch` 0.26 whose libtorch loads
//! when the runtime starts. Ops return `Result`; the README is the design.

// The shim-calling macros come first, for every module after.
#[macro_use]
mod wrappers;
mod error;
pub mod nn;
mod tensor;

pub use error::TchError;
pub type Result<T, E = TchError> = std::result::Result<T, E>;

pub use tensor::{IndexOp, NewAxis, Shape, TensorIndexer};
pub use wrappers::device::Device;
pub use wrappers::kind::{self, Kind};
pub use wrappers::layout::Layout;
pub use wrappers::scalar::Scalar;
pub use wrappers::tensor::{NoGradGuard, Reduction, Tensor, no_grad, no_grad_guard};
pub use wrappers::utils::manual_seed;
