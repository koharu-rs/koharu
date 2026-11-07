mod error;
pub use error::Error;
pub type Result<T> = std::result::Result<T, error::Error>;

pub(crate) mod wrappers;
pub use wrappers::device::{AllocatorMemory, Cuda, Device};
pub use wrappers::generator::Generator;
pub use wrappers::kind::{self, Kind};
pub use wrappers::layout::{Layout, MemoryFormat, QScheme};
pub use wrappers::scalar::Scalar;
pub use wrappers::utils::{IntList, IntListOption};

mod tensor;
pub use tensor::{IndexOp, InferenceMode, NewAxis, Tensor, TensorIndexer, index, inference_mode};
