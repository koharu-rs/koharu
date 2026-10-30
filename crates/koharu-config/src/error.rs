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
    #[error("{} isn't valid TOML: {message}", path.display())]
    Toml { path: PathBuf, message: String },
    #[error("[{section}] in {}: {message}", path.display())]
    Section {
        path: PathBuf,
        section: &'static str,
        message: String,
    },
    #[error("[{section}] can't be written as a TOML table: {message}")]
    Serialize {
        section: &'static str,
        message: String,
    },
    #[error(
        "there's no OS credential store to keep secrets in; on Linux, that's a Secret Service such as GNOME Keyring, KWallet, or KeePassXC ({0})"
    )]
    NoCredentialStore(String),
    #[error("the OS credential store refused: {0}")]
    Credentials(#[from] keyring::Error),
    #[error("{} can't be opened: {reason}", path.display())]
    Secrets { path: PathBuf, reason: String },
    #[error("the secret `{0}` can't be empty")]
    EmptySecret(String),
}
