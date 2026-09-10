use std::{
    fs::{File, OpenOptions},
    future::Future,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::{Context, Result, ensure};
use fs4::FileExt;
use serde::{Deserialize, Serialize};

static ROOT: OnceLock<PathBuf> = OnceLock::new();

#[derive(Default, Deserialize, Serialize)]
struct StoreConfig {
    packages_path: Option<PathBuf>,
}

/// Koharu's process-wide immutable package store.
pub struct Store;

impl Store {
    /// Returns the configured root, or the operating system cache by default.
    #[must_use]
    pub fn root() -> &'static Path {
        ROOT.get_or_init(Self::resolve_root)
    }

    fn resolve_root() -> PathBuf {
        match Self::configured_packages_path() {
            Some(path) if path.is_absolute() => path,
            Some(path) => {
                tracing::warn!(
                    "ignoring non-absolute runtime packages_path: {}",
                    path.display()
                );
                Self::default_root()
            }
            None => Self::default_root(),
        }
    }

    fn configured_packages_path() -> Option<PathBuf> {
        let section = koharu_config::load::<StoreConfig>("runtime").ok()?;
        let value = section.read().ok()?;
        value.packages_path.clone()
    }

    fn default_root() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("koharu")
            .join("packages")
    }

    pub(crate) async fn directory<Valid, Install, Pending>(
        target: PathBuf,
        valid: Valid,
        install: Install,
    ) -> Result<PathBuf>
    where
        Valid: Fn(&Path) -> bool + Send + Sync,
        Install: FnOnce(PathBuf) -> Pending + Send,
        Pending: Future<Output = Result<()>> + Send,
    {
        if valid(&target) {
            return Ok(target);
        }

        let _guard = Lock::acquire(&target).await?;
        if valid(&target) {
            return Ok(target);
        }

        let parent = target
            .parent()
            .with_context(|| format!("{} has no parent directory", target.display()))?;
        if target
            .try_exists()
            .with_context(|| format!("failed to inspect {}", target.display()))?
        {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("failed to replace incomplete {}", target.display()))?;
        }
        let stage = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(parent)
            .with_context(|| format!("failed to stage {}", target.display()))?;
        install(stage.path().to_owned()).await?;
        ensure!(
            valid(stage.path()),
            "installer produced an incomplete package"
        );

        match std::fs::rename(stage.path(), &target) {
            Ok(()) => Ok(target),
            Err(_) if valid(&target) => Ok(target),
            Err(error) => {
                Err(error).with_context(|| format!("failed to publish {}", target.display()))
            }
        }
    }

    pub(crate) async fn file<Install, Pending>(target: PathBuf, install: Install) -> Result<PathBuf>
    where
        Install: FnOnce(PathBuf) -> Pending + Send,
        Pending: Future<Output = Result<()>> + Send,
    {
        if target.is_file() {
            return Ok(target);
        }

        let _guard = Lock::acquire(&target).await?;
        if target.is_file() {
            return Ok(target);
        }

        let parent = target
            .parent()
            .with_context(|| format!("{} has no parent directory", target.display()))?;
        let stage = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to stage {}", target.display()))?
            .into_temp_path();
        install(stage.to_path_buf()).await?;
        ensure!(stage.is_file(), "installer did not create an artifact");

        if target.exists() {
            std::fs::remove_file(&target)
                .with_context(|| format!("failed to replace incomplete {}", target.display()))?;
        }
        stage
            .persist(&target)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to publish {}", target.display()))?;
        Ok(target)
    }
}

struct Lock(File);

impl Lock {
    async fn acquire(target: &Path) -> Result<Self> {
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
            FileExt::lock(&file).with_context(|| format!("failed to lock {}", path.display()))?;
            Ok(Self(file))
        })
        .await
        .context("runtime store lock task failed")?
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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
                Store::directory(
                    target,
                    |path| path.join("ready").is_file(),
                    move |stage| async move {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        tokio::fs::write(stage.join("ready"), b"ok").await?;
                        Ok(())
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

        let installed = Store::directory(
            target.clone(),
            |path| path.join("ready").is_file(),
            move |stage| async move {
                tokio::fs::write(stage.join("ready"), b"staged").await?;
                tokio::fs::create_dir_all(&winner).await?;
                tokio::fs::write(winner.join("ready"), b"winner").await?;
                Ok(())
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
                .block_on(Store::directory(
                    target.clone(),
                    |path| path.join("ready").is_file(),
                    move |stage| async move {
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(target.with_file_name("install-attempt"))?;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        tokio::fs::write(stage.join("ready"), b"ok").await?;
                        Ok(())
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
