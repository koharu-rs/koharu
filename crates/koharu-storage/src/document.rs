use std::{
    fs::{self, File},
    io::Write as _,
    path::{Path, PathBuf},
};

use atomicwrites::{AtomicFile, OverwriteBehavior};
use loro::LoroDoc;
use memmap2::Mmap;

use crate::{
    Result,
    lock::{lock, lock_path},
};

/// An open, exclusively locked document.
#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    _lock: File,
}

impl Document {
    /// Creates a document holding `snapshot`, a Loro snapshot export, and
    /// fails if `path` exists.
    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub fn create(path: impl AsRef<Path>, snapshot: &[u8]) -> Result<Self> {
        let path = path.as_ref().to_owned();
        let lock = lock(&path)?;
        AtomicFile::new(&path, OverwriteBehavior::DisallowOverwrite)
            .write(|file| file.write_all(snapshot))?;
        Ok(Self { path, _lock: lock })
    }

    /// Opens a document and loads it. Loro copies the whole file into memory
    /// but decodes each container when it is first read, so page images stay
    /// encoded until a page is used.
    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, LoroDoc)> {
        let path = path.as_ref().to_owned();
        let lock = lock(&path)?;
        let file = File::open(&path)?;
        // SAFETY: Loro copies the snapshot out of the map before this returns,
        // and the map is dropped right after. The lock keeps other Koharu
        // windows from replacing the file meanwhile, but it is advisory: another
        // program truncating the file during the import would fault. Mapping
        // saves holding a second full copy while Loro makes its own.
        let map = unsafe { Mmap::map(&file)? };
        let doc = LoroDoc::from_snapshot(&map)?;
        Ok((Self { path, _lock: lock }, doc))
    }

    /// Deletes a closed document, then its lock file.
    pub fn remove(path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let lock = lock(path)?;
        fs::remove_file(path)?;
        drop(lock);
        fs::remove_file(lock_path(path))?;
        Ok(())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replaces the document with `snapshot`, a Loro snapshot export. It goes
    /// to a synced temporary file that is renamed over the document, never
    /// written in place.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn save(&self, snapshot: &[u8]) -> Result<()> {
        AtomicFile::new(&self.path, OverwriteBehavior::AllowOverwrite)
            .write(|file| file.write_all(snapshot))?;
        Ok(())
    }
}
