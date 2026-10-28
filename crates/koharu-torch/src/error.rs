use std::io;

#[derive(Debug, thiserror::Error)]
pub enum TchError {
    /// An error libtorch threw.
    #[error("libtorch: {0}")]
    Torch(String),

    #[error("conversion error: {0}")]
    Convert(String),

    #[error("tensor kind error: {0}")]
    Kind(String),

    #[error("invalid shape: {0}")]
    Shape(String),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error("{path}: {err}")]
    SafeTensors {
        path: String,
        err: safetensors::SafeTensorError,
    },

    /// A weight the model has and the file doesn't: the port's names don't
    /// match the checkpoint's.
    #[error("{path} has no `{name}`")]
    MissingWeight { name: String, path: String },

    /// Weights the file has and the model doesn't, for the same reason.
    #[error("{path} has weights the model doesn't: {names}")]
    UnexpectedWeights { names: String, path: String },
}
