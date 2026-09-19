use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use anyhow::Context as _;
use bytes::Bytes;
use memmap2::Mmap;
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::{BlobId, Error, Result, durability};

const MMAP_MIN_BYTES: u64 = 256 * 1024;

pub(crate) struct BlobStore {
    root: PathBuf,
    _lock: File,
    _temporary: Option<TempDir>,
    leases: Mutex<Vec<Weak<BTreeSet<BlobId>>>>,
}

impl BlobStore {
    pub(crate) fn new(root: PathBuf, lock: File, temporary: Option<TempDir>) -> Self {
        Self {
            root,
            _lock: lock,
            _temporary: temporary,
            leases: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn scope(
        self: &Arc<Self>,
        referenced: BTreeSet<BlobId>,
        available: BTreeMap<BlobId, Bytes>,
    ) -> Result<Blobs> {
        if let Some(id) = available.keys().find(|id| !referenced.contains(id)) {
            return Err(Error::invalid(format!(
                "available blob {id} is not referenced by the state"
            )));
        }
        for (id, bytes) in &available {
            if BlobId::for_bytes(bytes) != *id {
                return Err(Error::invalid(format!("blob {id} has mismatched bytes")));
            }
        }
        let lease = Arc::new(referenced);
        self.leases.lock().push(Arc::downgrade(&lease));
        Ok(Blobs {
            inner: Arc::new(BlobScope {
                store: self.clone(),
                lease,
                available: Arc::new(available),
            }),
        })
    }

    pub(crate) fn verify_references(&self, ids: &BTreeSet<BlobId>) -> Result<()> {
        for id in ids {
            if !self.path(*id).try_exists()? {
                return Err(Error::BlobNotFound(*id));
            }
        }
        Ok(())
    }

    pub(crate) fn persist(&self, blobs: &Blobs) -> Result<()> {
        for id in blobs.inner.lease.iter().copied() {
            let target = self.path(id);
            if target.try_exists()? {
                continue;
            }
            let bytes = blobs
                .inner
                .available
                .get(&id)
                .ok_or(Error::BlobNotFound(id))?;
            durability::publish(&target, bytes)?;
        }
        Ok(())
    }

    pub(crate) fn durable_scope(self: &Arc<Self>, ids: BTreeSet<BlobId>) -> Result<Blobs> {
        self.verify_references(&ids)?;
        self.scope(ids, BTreeMap::new())
    }

    pub(crate) fn collect(&self, saved: BTreeSet<BlobId>) -> Result<(usize, u64)> {
        let mut marked = saved;
        let mut leases = self.leases.lock();
        leases.retain(|lease| {
            let Some(lease) = lease.upgrade() else {
                return false;
            };
            marked.extend(lease.iter().copied());
            true
        });
        drop(leases);

        let mut removed = 0;
        let mut bytes = 0u64;
        let root = self.root.join("blobs");
        if !root.try_exists()? {
            return Ok((0, 0));
        }
        for shard in fs::read_dir(root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(shard.path())? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Ok(id) = name.parse::<BlobId>() else {
                    continue;
                };
                if marked.contains(&id) {
                    continue;
                }
                let size = entry.metadata()?.len();
                fs::remove_file(entry.path())?;
                removed += 1;
                bytes = bytes.saturating_add(size);
            }
        }
        Ok((removed, bytes))
    }

    fn path(&self, id: BlobId) -> PathBuf {
        let name = id.to_string();
        self.root.join("blobs").join(&name[..2]).join(name)
    }
}

struct BlobScope {
    store: Arc<BlobStore>,
    lease: Arc<BTreeSet<BlobId>>,
    available: Arc<BTreeMap<BlobId, Bytes>>,
}

/// The complete blob closure of one immutable state.
#[derive(Clone)]
pub struct Blobs {
    inner: Arc<BlobScope>,
}

impl std::fmt::Debug for Blobs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Blobs")
            .field("referenced", &self.inner.lease.len())
            .field("available", &self.inner.available.len())
            .finish()
    }
}

impl Blobs {
    #[must_use]
    pub fn contains(&self, id: BlobId) -> bool {
        self.inner.lease.contains(&id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lease.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lease.is_empty()
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = BlobId> + '_ {
        self.inner.lease.iter().copied()
    }

    pub async fn get(&self, id: BlobId) -> Result<Bytes> {
        if !self.contains(id) {
            return Err(Error::BlobNotFound(id));
        }
        if let Some(bytes) = self.inner.available.get(&id) {
            return Ok(bytes.clone());
        }
        let path = self.inner.store.path(id);
        let lease = self.inner.lease.clone();
        tokio::task::spawn_blocking(move || map_or_read(&path, lease))
            .await
            .context("blob read task failed")?
    }

    pub(crate) fn derive(
        &self,
        referenced: BTreeSet<BlobId>,
        available: impl IntoIterator<Item = (BlobId, Bytes)>,
    ) -> Result<Self> {
        let mut contents = self
            .inner
            .available
            .iter()
            .filter(|(id, _)| referenced.contains(id))
            .map(|(id, bytes)| (*id, bytes.clone()))
            .collect::<BTreeMap<_, _>>();
        contents.extend(available);
        self.inner.store.scope(referenced, contents)
    }

    pub(crate) fn belongs_to(&self, store: &Arc<BlobStore>) -> bool {
        Arc::ptr_eq(&self.inner.store, store)
    }
}

struct MappedBlob {
    map: Mmap,
    _lease: Arc<BTreeSet<BlobId>>,
}

impl AsRef<[u8]> for MappedBlob {
    fn as_ref(&self) -> &[u8] {
        &self.map
    }
}

fn map_or_read(path: &Path, lease: Arc<BTreeSet<BlobId>>) -> Result<Bytes> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(Bytes::new());
    }
    if length < MMAP_MIN_BYTES {
        return fs::read(path).map(Bytes::from).map_err(Into::into);
    }
    // SAFETY: published blob paths are immutable. Koharu never truncates or
    // overwrites them, and the retained lease excludes the file from GC until
    // the final owner-backed Bytes value is dropped.
    let map = unsafe { Mmap::map(&file)? };
    Ok(Bytes::from_owner(MappedBlob { map, _lease: lease }))
}
