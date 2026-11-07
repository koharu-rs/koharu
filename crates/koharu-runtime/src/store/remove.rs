use std::path::{Path, PathBuf};

use anyhow::Result;

use super::lock::{lock, lock_file};

/// Removes every other version beside each of `versions`, the version folders
/// a start uses. A failure only logs, since models can still run: a version
/// another running Koharu holds open stays, and a later start removes it.
pub(crate) async fn remove_others(versions: Vec<PathBuf>) {
    for version in &versions {
        for other in beside(version) {
            if versions.contains(&other) {
                continue;
            }
            match remove(&other).await {
                Ok(()) => tracing::info!("removed {}", other.display()),
                Err(error) => tracing::warn!("couldn't remove {}: {error:#}", other.display()),
            }
        }
    }
}

/// The folders beside `version`, without the store's own hidden entries, such
/// as a download's stage.
fn beside(version: &Path) -> Vec<PathBuf> {
    let Some(Ok(entries)) = version.parent().map(fs_err::read_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && !entry.file_name().to_string_lossy().starts_with('.')
        })
        .map(|entry| entry.path())
        .collect()
}

async fn remove(folder: &Path) -> Result<()> {
    let _lock = lock(folder).await?;
    let path = folder.to_owned();
    tokio::task::spawn_blocking(move || fs_err::remove_dir_all(path)).await??;
    fs_err::remove_file(lock_file(folder)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::put;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_download_holds_off_removing_its_version() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        let (kept, old) = (package.join("kept"), package.join("old"));
        fs_err::create_dir_all(&kept).unwrap();
        let (started, start) = tokio::sync::oneshot::channel();

        let download = put(
            &old,
            old.join("archive"),
            |path| path.join("ready").is_file(),
            move |stage| async move {
                started.send(()).unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                tokio::fs::write(stage.join("ready"), b"ok").await?;
                Ok(stage)
            },
        );
        let remove = async {
            start.await.unwrap();
            remove_others(vec![kept.clone()]).await;
        };
        let (placed, ()) = tokio::join!(download, remove);

        placed.unwrap();
        assert!(!old.exists());
    }

    #[tokio::test]
    async fn only_other_versions_go() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        for folder in ["kept", "kept too", "old", ".stage-1"] {
            fs_err::create_dir_all(package.join(folder).join("library")).unwrap();
        }
        fs_err::write(package.join(".kept.lock"), b"").unwrap();
        fs_err::write(package.join("a file"), b"").unwrap();
        let other_package = root.path().join("other package");
        fs_err::create_dir_all(other_package.join("old")).unwrap();

        remove_others(vec![package.join("kept"), package.join("kept too")]).await;

        let mut names = fs_err::read_dir(&package)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [".kept.lock", ".stage-1", "a file", "kept", "kept too"]
        );
        assert!(other_package.join("old").is_dir());
    }
}
