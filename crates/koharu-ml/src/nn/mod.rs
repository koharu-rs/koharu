//! Layers for inference, on the generated ops. Constructors declare each
//! weight under the checkpoint's name; `VarStore::load` fills them.

mod batch_norm;
mod conv;
mod conv_transpose;
mod layer_norm;
mod linear;
mod module;
mod sparse;
mod var_store;

pub use batch_norm::*;
pub use conv::*;
pub use conv_transpose::*;
pub use layer_norm::*;
pub use linear::*;
pub use module::{Module, ModuleT};
pub use sparse::*;
pub use var_store::{Path, VarStore};
