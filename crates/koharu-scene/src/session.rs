use std::{collections::BTreeMap, path::Path, sync::Arc};

use bytes::Bytes;

use crate::{
    Change, Error, Patch, ProjectId, Result, Snapshot,
    patch::{Operation, apply_operations},
    state::{State, StoredState},
};

struct HistoryEntry {
    inverse: Arc<[Operation]>,
    // A revision's inverse may restore blobs that the current state no longer
    // references. Retain their lease for as long as that revision is undoable.
    _blobs: koharu_storage::Blobs,
}

/// One open project and its in-memory undo history.
///
/// Every successful commit publishes a complete scene snapshot. Undo commands
/// are retained only for the lifetime of this session; they are UI history, not
/// part of the durable project format.
pub struct Session {
    storage: koharu_storage::Session,
    current: Snapshot,
    history: BTreeMap<crate::Revision, HistoryEntry>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("project", &self.project_id())
            .field("revision", &self.current.revision())
            .field("undoable", &self.history.len())
            .finish_non_exhaustive()
    }
}

impl Session {
    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub async fn create(path: impl AsRef<Path>) -> Result<Self> {
        let document = koharu_storage::DocumentId::new();
        let state = State::empty(document);
        let storage = koharu_storage::Session::create(
            path,
            document,
            Bytes::from(encode_checkpoint(&state)?),
        )
        .await?;
        Self::assemble(storage, state).await
    }

    #[tracing::instrument(level = "info", skip_all, fields(path = %path.as_ref().display()))]
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let storage = koharu_storage::Session::open(path).await?;
        Self::recover(storage).await
    }

    pub async fn memory() -> Result<Self> {
        let document = koharu_storage::DocumentId::new();
        let state = State::empty(document);
        let storage =
            koharu_storage::Session::memory(document, Bytes::from(encode_checkpoint(&state)?))
                .await?;
        Self::assemble(storage, state).await
    }

    #[must_use]
    pub fn project_id(&self) -> ProjectId {
        ProjectId(self.current.state.document)
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.current.clone()
    }

    #[tracing::instrument(level = "info", skip_all, fields(project = %self.project_id(), base = %patch.base_revision))]
    pub async fn commit(&mut self, patch: Patch) -> Result<Commit> {
        if patch.project != self.current.state.document {
            return Err(Error::invalid("patch belongs to another project"));
        }
        if patch.base_revision != self.current.revision()
            || !Arc::ptr_eq(&patch.base_state, &self.current.state)
        {
            return Err(Error::Storage(koharu_storage::Error::RevisionConflict {
                current: self.current.revision(),
                proposed: patch.base_revision,
            }));
        }
        if patch.is_empty() {
            return Ok(Commit {
                revision: self.current.revision(),
                changes: Change::empty(self.current.revision()),
                snapshot: self.current.clone(),
            });
        }

        let next_revision = self
            .current
            .revision()
            .next()
            .ok_or_else(|| Error::invalid("project revision overflow"))?;
        let mut state = (*patch.state).clone();
        state.revision = next_revision;
        let proposed = self.current.storage.update(
            next_revision,
            Bytes::from(encode_checkpoint(&state)?),
            state.referenced_blobs(),
            patch.attachments.iter().cloned(),
        )?;
        let stored = self.storage.save(&proposed).await?;

        let inverse = patch
            .operations
            .iter()
            .rev()
            .map(Operation::reversed)
            .collect::<Vec<_>>();
        self.history.insert(
            next_revision,
            HistoryEntry {
                inverse: inverse.into(),
                _blobs: self.current.storage.blobs().clone(),
            },
        );

        let state = Arc::new(state);
        let snapshot = Snapshot::new(state, stored)?;
        let changes =
            Change::from_operations(patch.base_revision, next_revision, &patch.operations);
        self.current = snapshot.clone();
        Ok(Commit {
            revision: next_revision,
            changes,
            snapshot,
        })
    }

    pub async fn undo(&mut self, revision: crate::Revision) -> Result<Commit> {
        self.undo_many([revision]).await
    }

    #[tracing::instrument(level = "info", skip_all)]
    pub async fn undo_many(
        &mut self,
        revisions: impl IntoIterator<Item = crate::Revision>,
    ) -> Result<Commit> {
        let mut revisions = revisions.into_iter().collect::<Vec<_>>();
        revisions.sort_unstable_by(|left, right| right.cmp(left));
        revisions.dedup();
        if revisions.is_empty() {
            return Err(Error::invalid("undo requires at least one revision"));
        }
        let mut operations = Vec::new();
        for revision in &revisions {
            let entry = self.history.get(revision).ok_or_else(|| {
                Error::invalid(format!(
                    "revision {revision} is not undoable in this session"
                ))
            })?;
            operations.extend(entry.inverse.iter().cloned());
        }
        let state = apply_operations(&self.current.state, &operations)?;
        let label: Arc<str> = if revisions.len() == 1 {
            format!("Undo revision {}", revisions[0]).into()
        } else {
            format!("Undo {} revisions", revisions.len()).into()
        };
        let patch = Patch::new(
            &self.current,
            state,
            Vec::new(),
            operations,
            Vec::new(),
            Some(label),
        )?;
        self.commit(patch).await
    }

    pub async fn collect_garbage(&self) -> Result<koharu_storage::GcReport> {
        self.storage.collect_garbage().await.map_err(Into::into)
    }

    /// Adopts an arbitrary snapshot of this session as the new current state.
    ///
    /// Used by snapshot restore: the result is a real revision (the document
    /// moves forward in time) whose `Change` is deliberately empty — consumers
    /// must fully re-render instead of applying a diff.
    pub async fn restore(&mut self, snapshot: &Snapshot) -> Result<Commit> {
        if snapshot.state.document != self.current.state.document {
            return Err(Error::invalid("snapshot belongs to another project"));
        }
        let next_revision = self
            .current
            .revision()
            .next()
            .ok_or_else(|| Error::invalid("project revision overflow"))?;
        let mut state = (*snapshot.state).clone();
        state.revision = next_revision;
        let proposed = self.current.storage.update(
            next_revision,
            Bytes::from(encode_checkpoint(&state)?),
            state.referenced_blobs(),
            Vec::new(),
        )?;
        let stored = self.storage.save(&proposed).await?;
        let snapshot = Snapshot::new(Arc::new(state), stored)?;
        self.current = snapshot.clone();
        Ok(Commit {
            revision: next_revision,
            changes: Change::empty(next_revision),
            snapshot,
        })
    }

    /// Drops the undo history for the given revisions, releasing each entry's
    /// blob lease. Returns the number of revisions actually removed.
    ///
    /// The caller guarantees the dropped revisions are unreachable for undo;
    /// the session does not enforce this.
    pub fn drop_history(&mut self, revisions: impl IntoIterator<Item = crate::Revision>) -> usize {
        revisions
            .into_iter()
            .filter(|revision| self.history.remove(revision).is_some())
            .count()
    }

    async fn assemble(storage: koharu_storage::Session, state: State) -> Result<Self> {
        let stored = storage.load().await?;
        let current = Snapshot::new(Arc::new(state), stored)?;
        Ok(Self {
            storage,
            current,
            history: BTreeMap::new(),
        })
    }

    async fn recover(storage: koharu_storage::Session) -> Result<Self> {
        let stored = storage.load().await?;
        let checkpoint: StoredState = revision::from_slice(stored.payload())?;
        let state = State::from_checkpoint(stored.document_id(), stored.revision(), checkpoint)?;
        state.validate()?;
        let current = Snapshot::new(Arc::new(state), stored)?;
        Ok(Self {
            storage,
            current,
            history: BTreeMap::new(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub revision: crate::Revision,
    pub changes: Change,
    pub snapshot: Snapshot,
}

fn encode_checkpoint(state: &State) -> Result<Vec<u8>> {
    revision::to_vec(&state.to_checkpoint()).map_err(Into::into)
}
