mod error;
pub use error::TchError;
pub type Result<T> = std::result::Result<T, error::TchError>;

pub(crate) mod wrappers;
pub use wrappers::device::{AllocatorMemory, Cuda, Device};
pub use wrappers::generator::Generator;
pub use wrappers::kind::{self, Kind};
pub use wrappers::layout::{Layout, MemoryFormat, QScheme};
pub use wrappers::scalar::Scalar;
pub use wrappers::utils::{IntList, IntListOption};

mod tensor;
pub use tensor::{index, inference_mode, IndexOp, InferenceMode, NewAxis, Tensor, TensorIndexer};
