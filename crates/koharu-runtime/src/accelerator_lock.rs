use std::{
    fmt,
    fs::TryLockError,
    path::{Path, PathBuf},
};

use anyhow::Result;
use fs_err::{File, OpenOptions};

const LOCK_FILE: &str = ".accelerator.lock";

/// Error returned when another Koharu process already holds the GPU
/// accelerator lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceleratorLocked;

impl fmt::Display for AcceleratorLocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Another Koharu process is currently using the GPU accelerator")
    }
}

impl std::error::Error for AcceleratorLocked {}

/// Exclusive cross-process advisory lock over the shared GPU accelerator
/// and runtime store at `<store_root>/.accelerator.lock`. Releases on drop.
#[derive(Debug)]
pub struct AcceleratorLock {
    file: File,
    path: PathBuf,
}

impl AcceleratorLock {
    /// Acquires the lock at `<store_root>/.accelerator.lock`, failing fast
    /// with [`AcceleratorLocked`] if another process holds it.
    pub fn acquire() -> Result<Self> {
        Self::acquire_in(crate::store::root())
    }

    /// Acquires the lock inside `root` (`<root>/.accelerator.lock`).
    /// Primarily used by tests to isolate lock files from the user's store.
    pub fn acquire_in(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        fs_err::create_dir_all(root)?;
        let path = root.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file, path }),
            Err(TryLockError::WouldBlock) => Err(AcceleratorLocked.into()),
            Err(TryLockError::Error(error)) => {
                Err(anyhow::Error::new(error).context(format!("failed to lock {}", path.display())))
            }
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AcceleratorLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_in_blocks_second_holder_until_dropped() {
        let root = tempfile::tempdir().unwrap();
        let lock = AcceleratorLock::acquire_in(root.path()).expect("first acquisition succeeds");
        assert_eq!(lock.path(), root.path().join(".accelerator.lock"));
        assert!(lock.path().is_file());

        let err = AcceleratorLock::acquire_in(root.path()).expect_err("second acquisition blocks");
        assert_eq!(
            err.downcast_ref::<AcceleratorLocked>(),
            Some(&AcceleratorLocked)
        );
        assert_eq!(
            err.to_string(),
            "Another Koharu process is currently using the GPU accelerator"
        );

        drop(lock);
        let reacquired =
            AcceleratorLock::acquire_in(root.path()).expect("reacquisition succeeds after drop");
        assert_eq!(reacquired.path(), root.path().join(".accelerator.lock"));
    }

    #[test]
    fn acquire_in_contends_across_processes() {
        const DIR_ENV: &str = "KOHARU_RUNTIME_ACCEL_LOCK_DIR";
        const EXPECT_ENV: &str = "KOHARU_RUNTIME_ACCEL_LOCK_EXPECT";

        if let Some(dir) = std::env::var_os(DIR_ENV) {
            let expect = std::env::var(EXPECT_ENV).unwrap();
            let result = AcceleratorLock::acquire_in(PathBuf::from(dir));
            match expect.as_str() {
                "blocked" => {
                    let err = result.expect_err("child must contend while parent holds lock");
                    assert_eq!(
                        err.downcast_ref::<AcceleratorLocked>(),
                        Some(&AcceleratorLocked)
                    );
                }
                "acquired" => {
                    result.expect("child must acquire lock after parent drops it");
                }
                other => panic!("unexpected mode {other}"),
            }
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let spawn = |expect: &str| {
            std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "accelerator_lock::tests::acquire_in_contends_across_processes",
                ])
                .env(DIR_ENV, root.path())
                .env(EXPECT_ENV, expect)
                .output()
                .unwrap()
        };

        let guard = AcceleratorLock::acquire_in(root.path()).unwrap();
        let blocked = spawn("blocked");
        assert!(
            blocked.status.success(),
            "blocked child failed: {}",
            String::from_utf8_lossy(&blocked.stderr)
        );

        drop(guard);
        let acquired = spawn("acquired");
        assert!(
            acquired.status.success(),
            "acquired child failed: {}",
            String::from_utf8_lossy(&acquired.stderr)
        );
    }
}
