use std::ffi::NulError;

use thiserror::Error;

/// Main library error type.
#[derive(Error, Debug)]
pub enum Error {
    /// Conversion error.
    #[error("conversion error: {0}")]
    Convert(String),

    /// Invalid file format.
    #[error("invalid file format: {0}")]
    FileFormat(String),

    /// Missing tensor with name.
    #[error("cannot find the tensor named {0} in {1}")]
    TensorNameNotFound(String, String),

    /// Tensor kind error.
    #[error("tensor kind error: {0}")]
    Kind(String),

    /// Null pointer.
    #[error(transparent)]
    Nul(#[from] NulError),

    /// Invalid shape.
    #[error("invalid shape: {0}")]
    Shape(String),

    /// Unknown kind
    #[error("unknown kind: {0}")]
    UnknownKind(i32),

    /// Errors returned by the Torch C++ API.
    #[error("Internal torch error: {0}")]
    Torch(String),
}
