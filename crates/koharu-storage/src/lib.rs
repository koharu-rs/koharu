//! Koharu documents. A document is one `.loro` file: the Loro snapshot of a
//! Koharu project, page images included. Storage owns locking, loading,
//! and atomic saves; what the document means belongs to the scene.

use std::{
    fs::{self, File},
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use atomicwrites::{AtomicFile, OverwriteBehavior};
use fs4::FileExt;
use loro::{ExportMode, LoroDoc};
use memmap2::Mmap;

/// An open, exclusively locked document.
#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    _lock: File,
}

impl Document {
    /// Creates a document holding `doc`, and fails if `path` exists.
    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub fn create(path: impl AsRef<Path>, doc: &LoroDoc) -> Result<Self> {
        let path = path.as_ref().to_owned();
        let lock = lock(&path)?;
        let snapshot = doc.export(ExportMode::Snapshot)?;
        AtomicFile::new(&path, OverwriteBehavior::DisallowOverwrite)
            .write(|file| file.write_all(&snapshot))?;
        Ok(Self { path, _lock: lock })
    }

    /// Opens a document and loads it. Containers are decoded
    /// when first read, so page images stay encoded until a page is used.
    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, LoroDoc)> {
        let path = path.as_ref().to_owned();
        let lock = lock(&path)?;
        let file = File::open(&path)?;
        // SAFETY: the map lives only for this import, while the document lock
        // keeps Koharu from writing the file. Saves never write in place.
        let map = unsafe { Mmap::map(&file)? };
        let doc = LoroDoc::from_snapshot(&map)?;
        Ok((Self { path, _lock: lock }, doc))
    }

    /// Deletes a closed document and its lock file.
    pub fn remove(path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        drop(lock(path)?);
        fs::remove_file(path)?;
        fs::remove_file(path.with_added_extension("lock"))?;
        Ok(())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Saves `doc` with its history. The snapshot goes to a synced temporary
    /// file that is renamed over the document, never written in place.
    /// Containers that were never read are written from their loaded bytes,
    /// so a save does not decode the document's images.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn save(&self, doc: &LoroDoc) -> Result<()> {
        let snapshot = doc.export(ExportMode::Snapshot)?;
        AtomicFile::new(&self.path, OverwriteBehavior::AllowOverwrite)
            .write(|file| file.write_all(&snapshot))?;
        Ok(())
    }

    /// Saves `doc` without its history, dropping every image an edit has
    /// replaced. Loro decodes every container to do this, so it is meant for
    /// rare moments such as closing a document, not for every save.
    #[tracing::instrument(level = "info", skip_all)]
    pub fn compact(&self, doc: &LoroDoc) -> Result<()> {
        let frontiers = doc.oplog_frontiers();
        let snapshot = doc.export(ExportMode::shallow_snapshot(&frontiers))?;
        AtomicFile::new(&self.path, OverwriteBehavior::AllowOverwrite)
            .write(|file| file.write_all(&snapshot))?;
        Ok(())
    }
}

/// Locks `<path>.lock`. Saves rename a new file over the document, so a lock
/// on the document itself would stay with the replaced file. fs4 is called
/// through its trait because std's inherent `File::try_lock` would win.
fn lock(path: &Path) -> Result<File> {
    let lock = File::create(path.with_added_extension("lock"))?;
    FileExt::try_lock(&lock).with_context(|| format!("cannot lock {}", path.display()))?;
    Ok(lock)
}
