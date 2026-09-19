use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context as _;
use bytes::Bytes;
use fs4::FileExt;
use parking_lot::RwLock;
use tempfile::TempDir;
use tokio::sync::Mutex;

use crate::{
    BlobId, Blobs, DocumentId, Error, Result, Revision,
    blobs::BlobStore,
    format::{self, Slot, StoredState},
};

#[derive(Clone, Debug)]
struct Head {
    slot: Slot,
    stored: Arc<StoredState>,
}

struct Inner {
    root: PathBuf,
    blobs: Arc<BlobStore>,
    head: RwLock<Head>,
    writer: Mutex<()>,
}

/// One open project. Clones share the same writer lock and durable head.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let head = self.inner.head.read();
        formatter
            .debug_struct("Session")
            .field("document", &head.stored.document)
            .field("revision", &head.stored.revision)
            .field("root", &self.inner.root)
            .finish_non_exhaustive()
    }
}

/// One immutable serialized scene state and its complete blob closure.
#[derive(Clone, Debug)]
pub struct State {
    document: DocumentId,
    revision: Revision,
    payload: Bytes,
    blobs: Blobs,
}

impl State {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn blobs(&self) -> &Blobs {
        &self.blobs
    }

    /// Builds another immutable state while retaining still-referenced pending
    /// content from this state. `available` contains bytes already produced by
    /// the edit; storage determines which of them actually need publication.
    pub fn update(
        &self,
        revision: Revision,
        payload: Bytes,
        referenced: impl IntoIterator<Item = BlobId>,
        available: impl IntoIterator<Item = (BlobId, Bytes)>,
    ) -> Result<Self> {
        let referenced = referenced.into_iter().collect::<BTreeSet<_>>();
        let blobs = self.blobs.derive(referenced, available)?;
        Ok(Self {
            document: self.document,
            revision,
            payload,
            blobs,
        })
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct GcReport {
    pub blobs: usize,
    pub bytes: u64,
}

impl Session {
    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display(), document = %document))]
    pub async fn create(
        path: impl AsRef<Path>,
        document: DocumentId,
        payload: Bytes,
    ) -> Result<Self> {
        let path = path.as_ref().to_owned();
        tokio::task::spawn_blocking(move || create_project(path, document, payload, None))
            .await
            .context("project creation task failed")?
    }

    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        tokio::task::spawn_blocking(move || open_project(path, None))
            .await
            .context("project open task failed")?
    }

    pub async fn memory(document: DocumentId, payload: Bytes) -> Result<Self> {
        tokio::task::spawn_blocking(move || {
            let temporary = tempfile::tempdir()?;
            create_project(
                temporary.path().to_owned(),
                document,
                payload,
                Some(temporary),
            )
        })
        .await
        .context("temporary project creation task failed")?
    }

    #[must_use]
    pub fn document_id(&self) -> DocumentId {
        self.inner.head.read().stored.document
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.inner.head.read().stored.revision
    }

    pub async fn load(&self) -> Result<State> {
        let stored = self.inner.head.read().stored.clone();
        let referenced = stored.blobs.iter().copied().collect();
        let blobs = self.inner.blobs.durable_scope(referenced)?;
        Ok(State {
            document: stored.document,
            revision: stored.revision,
            payload: Bytes::from(stored.payload.clone()),
            blobs,
        })
    }

    /// Publishes all missing blob contents and then the complete state. The
    /// returned state has a durable blob scope and drops pending byte owners.
    #[tracing::instrument(level = "info", skip_all, fields(document = %state.document, revision = %state.revision))]
    pub async fn save(&self, state: &State) -> Result<State> {
        if state.document != self.document_id() {
            return Err(Error::DocumentMismatch {
                state: state.document,
                session: self.document_id(),
            });
        }
        if !state.blobs.belongs_to(&self.inner.blobs) {
            return Err(Error::invalid("state blobs belong to another session"));
        }

        let _writer = self.inner.writer.lock().await;
        let current = self.inner.head.read().clone();
        if state.revision <= current.stored.revision {
            return Err(Error::RevisionConflict {
                current: current.stored.revision,
                proposed: state.revision,
            });
        }

        let root = self.inner.root.clone();
        let store = self.inner.blobs.clone();
        let blobs = state.blobs.clone();
        let document = state.document;
        let revision = state.revision;
        let payload = state.payload.clone();
        let slot = current.slot.other();
        let stored = tokio::task::spawn_blocking(move || {
            store.persist(&blobs)?;
            let stored = StoredState {
                document,
                revision,
                blobs: blobs.ids().collect(),
                payload: payload.to_vec(),
            };
            format::save(&root, slot, &stored)?;
            Ok::<_, Error>(stored)
        })
        .await
        .context("project save task failed")??;

        self.inner.head.write().clone_from(&Head {
            slot,
            stored: Arc::new(stored),
        });
        let durable = self
            .inner
            .blobs
            .scope(state.blobs.ids().collect(), BTreeMap::new())?;
        Ok(State {
            document,
            revision,
            payload: state.payload.clone(),
            blobs: durable,
        })
    }

    #[tracing::instrument(level = "info", skip_all)]
    pub async fn collect_garbage(&self) -> Result<GcReport> {
        let _writer = self.inner.writer.lock().await;
        let root = self.inner.root.clone();
        let store = self.inner.blobs.clone();
        tokio::task::spawn_blocking(move || {
            let mut saved = BTreeSet::new();
            for slot in [Slot::A, Slot::B] {
                if let Ok(Some(state)) = format::load(&root, slot) {
                    saved.extend(state.blobs);
                }
            }
            let (blobs, bytes) = store.collect(saved)?;
            Ok::<_, Error>(GcReport { blobs, bytes })
        })
        .await
        .context("storage garbage collection task failed")?
    }
}

fn create_project(
    root: PathBuf,
    document: DocumentId,
    payload: Bytes,
    temporary: Option<TempDir>,
) -> Result<Session> {
    fs::create_dir_all(root.join("blobs"))?;
    if format::path(&root, Slot::A).try_exists()? || format::path(&root, Slot::B).try_exists()? {
        return Err(Error::invalid("project already contains a state"));
    }
    let lock = lock_project(&root)?;
    let stored = StoredState {
        document,
        revision: Revision::ZERO,
        blobs: Vec::new(),
        payload: payload.to_vec(),
    };
    format::save(&root, Slot::A, &stored)?;
    Ok(Session {
        inner: Arc::new(Inner {
            blobs: Arc::new(BlobStore::new(root.clone(), lock, temporary)),
            root,
            head: RwLock::new(Head {
                slot: Slot::A,
                stored: Arc::new(stored),
            }),
            writer: Mutex::new(()),
        }),
    })
}

fn open_project(root: PathBuf, temporary: Option<TempDir>) -> Result<Session> {
    if !root.is_dir() {
        return Err(Error::NotAProject);
    }
    let lock = lock_project(&root)?;
    let store = Arc::new(BlobStore::new(root.clone(), lock, temporary));
    let mut candidates = Vec::new();
    let mut first_error = None;
    for slot in [Slot::A, Slot::B] {
        match format::load(&root, slot) {
            Ok(Some(state)) => candidates.push((slot, state)),
            Ok(None) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    candidates.sort_by_key(|(_, state)| std::cmp::Reverse(state.revision));
    let mut selected = None;
    for (slot, state) in candidates {
        let ids = state.blobs.iter().copied().collect();
        if store.verify_references(&ids).is_ok() {
            selected = Some((slot, state));
            break;
        }
    }
    let (slot, stored) = selected.ok_or_else(|| first_error.unwrap_or(Error::NotAProject))?;
    Ok(Session {
        inner: Arc::new(Inner {
            root,
            blobs: store,
            head: RwLock::new(Head {
                slot,
                stored: Arc::new(stored),
            }),
            writer: Mutex::new(()),
        }),
    })
}

fn lock_project(root: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("project.lock"))?;
    FileExt::try_lock(&lock).map_err(|error| match error {
        fs4::TryLockError::WouldBlock => Error::Locked,
        fs4::TryLockError::Error(error) => Error::Io(error),
    })?;
    Ok(lock)
}
