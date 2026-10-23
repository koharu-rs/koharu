use std::{io, path::PathBuf};

use loro::LoroError;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} is open in another window", .0.display())]
    Locked(PathBuf),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Loro(#[from] LoroError),
}

impl From<atomicwrites::Error<io::Error>> for Error {
    fn from(error: atomicwrites::Error<io::Error>) -> Self {
        match error {
            atomicwrites::Error::Internal(error) | atomicwrites::Error::User(error) => {
                Self::Io(error)
            }
        }
    }
}
