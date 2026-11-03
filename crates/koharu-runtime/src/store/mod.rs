//! The store packages and models install into, one version folder each.

mod cleanup;
mod lock;

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context, Result, ensure};
use tempfile::TempDir;

pub(crate) use cleanup::remove_others;
use lock::lock;

/// The one store everything installs into: the `store` folder beside the
/// executable in Windows release builds, where the installer puts Koharu,
/// and the OS cache folder's `koharu/store` otherwise.
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

/// Installs `target`, a file or a folder, once: `write` fills a staging
/// folder on the store's own volume and returns what it wrote there, which
/// moves into place once `valid` says it's complete.
pub(crate) async fn install<Valid, Write, Pending>(
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

    let _lock = lock(&target).await?;
    if valid(&target) {
        return Ok(target);
    }

    let stage = stage(&target)?;
    let staged = write(stage.path().to_owned()).await?;
    ensure!(valid(&staged), "installer produced an incomplete package");
    publish(&staged, target, valid)
}

/// A staging folder beside `target`, on the store's own volume, once an
/// incomplete `target` is removed.
fn stage(target: &Path) -> Result<TempDir> {
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent directory", target.display()))?;
    if target
        .try_exists()
        .with_context(|| format!("failed to inspect {}", target.display()))?
    {
        std::fs::remove_dir_all(target)
            .with_context(|| format!("failed to replace incomplete {}", target.display()))?;
    }
    tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(parent)
        .with_context(|| format!("failed to stage {}", target.display()))
}

/// Moves `staged` into place as `target`; a valid `target` that another
/// install published first wins.
fn publish(staged: &Path, target: PathBuf, valid: impl Fn(&Path) -> bool) -> Result<PathBuf> {
    match std::fs::rename(staged, &target) {
        Ok(()) => Ok(target),
        Err(_) if valid(&target) => Ok(target),
        Err(error) => Err(error).with_context(|| format!("failed to publish {}", target.display())),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::OpenOptions,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_installation_wins_for_a_shared_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("runtime");
        let attempts = Arc::new(AtomicUsize::new(0));
        let run = || {
            let target = target.clone();
            let attempts = attempts.clone();
            async move {
                install(
                    target,
                    |path| path.join("ready").is_file(),
                    move |stage| async move {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        tokio::fs::write(stage.join("ready"), b"ok").await?;
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

    #[tokio::test]
    async fn a_valid_target_that_appears_during_installation_wins() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("runtime");
        let winner = target.clone();

        let installed = install(
            target.clone(),
            |path| path.join("ready").is_file(),
            move |stage| async move {
                tokio::fs::write(stage.join("ready"), b"staged").await?;
                tokio::fs::create_dir_all(&winner).await?;
                tokio::fs::write(winner.join("ready"), b"winner").await?;
                Ok(stage)
            },
        )
        .await
        .unwrap();

        assert_eq!(installed, target);
        assert_eq!(std::fs::read(target.join("ready")).unwrap(), b"winner");
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".install-")
        }));
    }

    #[test]
    fn one_installation_wins_across_processes() {
        const TARGET: &str = "KOHARU_RUNTIME_STORE_TEST_TARGET";

        if let Some(target) = std::env::var_os(TARGET) {
            let target = PathBuf::from(target);
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(install(
                    target.clone(),
                    |path| path.join("ready").is_file(),
                    move |stage| async move {
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(target.with_file_name("install-attempt"))?;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        tokio::fs::write(stage.join("ready"), b"ok").await?;
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
                    "store::tests::one_installation_wins_across_processes",
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
        assert!(root.path().join("install-attempt").is_file());
    }
}
