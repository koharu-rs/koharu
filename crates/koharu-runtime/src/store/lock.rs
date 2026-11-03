use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use anyhow::{Context, Result};

/// Locks `target` against every other install of it, in this process or
/// another, until the returned file closes.
pub(super) async fn lock(target: &Path) -> Result<File> {
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent directory", target.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("failed to create {}", parent.display()))?;

    let name = target
        .file_name()
        .with_context(|| format!("{} has no file name", target.display()))?
        .to_string_lossy();
    let path = target.with_file_name(format!(".{name}.lock"));
    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.lock()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(file)
    })
    .await
    .context("runtime store lock task failed")?
}
