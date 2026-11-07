use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs_err::{File, OpenOptions};

/// The lock file beside `target`.
pub(super) fn lock_file(target: &Path) -> Result<PathBuf> {
    let name = target
        .file_name()
        .with_context(|| format!("{} has no file name", target.display()))?;
    Ok(target.with_file_name(format!(".{}.lock", name.to_string_lossy())))
}

/// Locks `target` against every other use of it, in this process or another,
/// until the returned file closes.
pub(super) async fn lock(target: &Path) -> Result<File> {
    let path = lock_file(target)?;
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent folder", target.display()))?
        .to_owned();
    let file = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
        fs_err::create_dir_all(parent)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock()?;
        Ok(file)
    })
    .await??;
    Ok(file)
}
