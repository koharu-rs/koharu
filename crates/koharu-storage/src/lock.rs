use std::{
    ffi::OsString,
    fs::{File, TryLockError},
    path::{Path, PathBuf},
};

use crate::{Error, Result};

/// Locks the hidden `.<name>.lock` beside the document. Saves rename a new file
/// over the document, so a lock on the document itself would stay with the
/// replaced file.
pub(crate) fn lock(path: &Path) -> Result<File> {
    let lock = File::create(lock_path(path))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(Error::Locked(path.to_owned())),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

pub(crate) fn lock_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".lock");
    path.with_file_name(name)
}
