use std::{io, path::PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("there's no home folder to keep Koharu's settings in")]
    NoHome,
    #[error("couldn't read {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("couldn't save {}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("{} is invalid: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
    #[error(
        "there's no OS credential store to keep secrets in; on Linux, that's a Secret Service such as GNOME Keyring, KWallet, or KeePassXC ({0})"
    )]
    NoCredentialStore(String),
    #[error("the OS credential store refused: {0}")]
    CredentialStore(String),
    #[error("{} can't be read: {reason}", path.display())]
    SecretsUnreadable { path: PathBuf, reason: String },
    #[error("the secret `{0}` can't be empty")]
    EmptySecret(String),
}
