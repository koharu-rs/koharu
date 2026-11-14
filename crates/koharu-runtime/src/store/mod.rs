//! The store packages and models download into, one version folder each.

mod lock;
mod remove;

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context, Result, ensure};

use lock::lock;
pub(crate) use remove::remove_others;

/// The one store everything downloads into: the `store` folder beside the
/// executable in Windows release builds, where setup puts Koharu, and the OS
/// cache folder's `koharu/store` otherwise.
pub(crate) fn root() -> &'static Path {
    static ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
        if cfg!(all(windows, not(debug_assertions))) {
            std::env::current_exe()
                .expect("Koharu knows where its executable is")
                .with_file_name("store")
        } else {
            dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("koharu")
                .join("store")
        }
    });
    &ROOT
}

/// Puts `target`, a file or a folder, in the store once: `write` fills a
/// staging folder on the store's own volume and returns what it wrote there,
/// which moves into place once `valid` says it's complete. A lock on `scope`,
/// which is `target` or the version folder it's in, holds off every other
/// download into it and its removal, in this process or another.
pub(crate) async fn put<Valid, Write, Pending>(
    scope: &Path,
    target: PathBuf,
    valid: Valid,
    write: Write,
) -> Result<PathBuf>
where
    Valid: Fn(&Path) -> bool + Send + Sync,
    Write: FnOnce(PathBuf) -> Pending + Send,
    Pending: Future<Output = Result<PathBuf>> + Send,
{
    if valid(&target) {
        return Ok(target);
    }

    let _lock = lock(scope).await?;
    if valid(&target) {
        return Ok(target);
    }

    // Not valid, so it's what's left of an incomplete one.
    if fs_err::exists(&target)? {
        fs_err::remove_dir_all(&target)?;
    }
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent folder", target.display()))?;
    fs_err::create_dir_all(parent)?;
    let stage = tempfile::Builder::new()
        .prefix(".stage-")
        .tempdir_in(parent)?;
    let staged = write(stage.path().to_owned()).await?;
    ensure!(
        valid(&staged),
        "the download produced an incomplete package"
    );
    fs_err::rename(&staged, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use fs_err::OpenOptions;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_download_wins_for_a_shared_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("runtime");
        let attempts = Arc::new(AtomicUsize::new(0));
        let run = || {
            let target = target.clone();
            let attempts = attempts.clone();
            async move {
                put(
                    &target,
                    target.clone(),
                    |path| path.join("ready").is_file(),
                    move |stage| async move {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        fs_err::tokio::write(stage.join("ready"), b"ok").await?;
                        Ok(stage)
                    },
                )
                .await
            }
        };

        let (left, right) = tokio::join!(run(), run());
        assert_eq!(left.unwrap(), target);
        assert_eq!(right.unwrap(), target);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn one_download_wins_across_processes() {
        const TARGET: &str = "KOHARU_RUNTIME_STORE_TEST_TARGET";

        if let Some(target) = std::env::var_os(TARGET) {
            let target = PathBuf::from(target);
            let attempt = target.with_file_name("attempt");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(put(
                    &target,
                    target.clone(),
                    |path| path.join("ready").is_file(),
                    move |stage| async move {
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(attempt)?;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        fs_err::tokio::write(stage.join("ready"), b"ok").await?;
                        Ok(stage)
                    },
                ))
                .unwrap();
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("runtime");
        let executable = std::env::current_exe().unwrap();
        let spawn = || {
            std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "store::tests::one_download_wins_across_processes",
                ])
                .env(TARGET, &target)
                .spawn()
                .unwrap()
        };
        let mut first = spawn();
        let mut second = spawn();

        assert!(first.wait().unwrap().success());
        assert!(second.wait().unwrap().success());
        assert!(target.join("ready").is_file());
        assert!(root.path().join("attempt").is_file());
    }
}
