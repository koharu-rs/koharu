//! In-memory linear action history, Photoshop-style.
//!
//! Owns the per-project "History panel" model: one timeline of named states
//! with a cursor. Undo and redo are cursor movements (`jump`); a new edit
//! discards every state right of the cursor. Each state pins a full scene
//! `Snapshot` (cheap `Arc` clone) plus every session revision it ever
//! produced so discarded states can release their blob leases through
//! `Session::drop_history`.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use koharu_scene::{Commit, EntityId, Revision, Session, Snapshot};
use serde::Serialize;
use specta::Type;

/// How long repeated edits of the same target merge into one history state.
const COALESCE_WINDOW: Duration = Duration::from_secs(3);

/// Default number of retained states (Photoshop "History States"). Bounds the
/// blob leases that superseded in-memory revisions would otherwise hold for
/// the whole session.
pub(crate) const DEFAULT_CAPACITY: usize = 50;

/// Semantic name of a history state. Serialized as a snake_case tag; the
/// client localizes it through `history.<tag>` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HistoryName {
    Open,
    ImportPages,
    RenamePage,
    DeletePages,
    MovePage,
    AddText,
    SourceText,
    Translation,
    Typography,
    Geometry,
    Transform,
    ToggleLayer,
    Opacity,
    DeleteLayers,
    MoveLayer,
    Brush,
    Erase,
    PipelineStage,
    SnapshotRestore,
}

/// Identity of a mergeable micro edit (same command on the same target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MergeKey {
    pub(crate) name: HistoryName,
    pub(crate) target: EntityId,
}

/// One immutable step on the history timeline.
pub(crate) struct HistoryState {
    name: HistoryName,
    detail: Option<String>,
    /// Revisions committed when the state was recorded. Crossing the state
    /// backwards applies their inverses; the list stays contiguous in the
    /// session history even after coalescing (merges only happen at the tip).
    forward: Vec<Revision>,
    /// Commits previously used to cross this state forward (LIFO).
    redo: Vec<Revision>,
    /// Every revision owned by this state: `forward` plus every undo/redo
    /// traversal commit. Fed to `Session::drop_history` on discard.
    related: Vec<Revision>,
    /// True when the state was produced by `Session::restore` (snapshot
    /// restore): there is no inverse to replay, so stepping back across it
    /// must adopt the previous state's pinned snapshot instead.
    restore_only: bool,
    snapshot: Snapshot,
    active_page: Option<EntityId>,
    merge: Option<MergeKey>,
    at: Instant,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct HistoryEntryInfo {
    pub index: u32,
    pub name: HistoryName,
    pub detail: Option<String>,
    pub current: bool,
    /// True for states right of the cursor (Photoshop dims them; a new edit
    /// discards them).
    pub undone: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct SnapshotInfo {
    pub id: u32,
    pub name: String,
}

pub(crate) struct SnapshotEntry {
    id: u32,
    name: String,
    snapshot: Snapshot,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct HistoryView {
    pub entries: Vec<HistoryEntryInfo>,
    pub snapshots: Vec<SnapshotInfo>,
    pub cursor: u32,
}

pub(crate) struct History {
    states: Vec<HistoryState>,
    /// Named snapshots. Not subject to capacity washout and explicitly
    /// user-managed, mirroring Photoshop snapshots.
    snapshots: Vec<SnapshotEntry>,
    next_snapshot: u32,
    cursor: usize,
    capacity: usize,
}

impl History {
    /// The anchor state `Open` is the state at open/create time, so undo can
    /// walk back to exactly what was on disk.
    pub(crate) fn new(snapshot: Snapshot, active_page: Option<EntityId>, capacity: usize) -> Self {
        Self {
            states: vec![HistoryState {
                name: HistoryName::Open,
                detail: None,
                forward: Vec::new(),
                redo: Vec::new(),
                related: Vec::new(),
                restore_only: false,
                snapshot,
                active_page,
                merge: None,
                at: Instant::now(),
            }],
            snapshots: Vec::new(),
            next_snapshot: 1,
            cursor: 0,
            capacity: capacity.max(1),
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.cursor + 1 < self.states.len()
    }

    pub(crate) fn view(&self) -> HistoryView {
        HistoryView {
            entries: self
                .states
                .iter()
                .enumerate()
                .map(|(index, state)| HistoryEntryInfo {
                    index: index as u32,
                    name: state.name,
                    detail: state.detail.clone(),
                    current: index == self.cursor,
                    undone: index > self.cursor,
                })
                .collect(),
            snapshots: self
                .snapshots
                .iter()
                .map(|entry| SnapshotInfo {
                    id: entry.id,
                    name: entry.name.clone(),
                })
                .collect(),
            cursor: self.cursor as u32,
        }
    }

    /// Pins the current scene snapshot under a stable id. Snapshots survive
    /// capacity washout and `clear`.
    pub(crate) fn create_snapshot(&mut self, snapshot: Snapshot, name: Option<String>) -> u32 {
        let id = self.next_snapshot;
        self.next_snapshot += 1;
        self.snapshots.push(SnapshotEntry {
            id,
            name: name.unwrap_or_else(|| format!("Snapshot {id}")),
            snapshot,
        });
        id
    }

    pub(crate) fn snapshot(&self, id: u32) -> Option<(&Snapshot, &str)> {
        self.snapshots
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (&entry.snapshot, entry.name.as_str()))
    }

    pub(crate) fn delete_snapshot(&mut self, id: u32) -> bool {
        let before = self.snapshots.len();
        self.snapshots.retain(|entry| entry.id != id);
        self.snapshots.len() != before
    }

    /// Records a freshly committed change as one state. Returns false for
    /// empty commits so callers can skip publishing.
    pub(crate) fn record(
        &mut self,
        session: &mut Session,
        name: HistoryName,
        detail: Option<String>,
        merge: Option<EntityId>,
        commit: &Commit,
        active_page: Option<EntityId>,
    ) -> bool {
        if commit.changes.to == commit.changes.from {
            return false;
        }
        let revision = commit.revision;
        if let Some(target) = merge
            && self.cursor + 1 == self.states.len()
        {
            let key = MergeKey { name, target };
            let current = &mut self.states[self.cursor];
            if current.merge == Some(key) && current.at.elapsed() < COALESCE_WINDOW {
                current.forward.push(revision);
                current.related.push(revision);
                current.snapshot = commit.snapshot.clone();
                current.active_page = active_page;
                current.at = Instant::now();
                return true;
            }
        }
        self.truncate_forward(session);
        self.states.push(HistoryState {
            name,
            detail,
            forward: vec![revision],
            redo: Vec::new(),
            related: vec![revision],
            restore_only: false,
            snapshot: commit.snapshot.clone(),
            active_page,
            merge: merge.map(|target| MergeKey { name, target }),
            at: Instant::now(),
        });
        self.cursor += 1;
        self.wash_out(session);
        true
    }

    /// Records a `Session::restore` commit (snapshot restore). The state has
    /// no replayable inverse, so stepping back across it adopts the previous
    /// state's pinned snapshot.
    pub(crate) fn record_restore(
        &mut self,
        session: &mut Session,
        detail: Option<String>,
        commit: &Commit,
        active_page: Option<EntityId>,
    ) {
        self.truncate_forward(session);
        self.states.push(HistoryState {
            name: HistoryName::SnapshotRestore,
            detail,
            forward: Vec::new(),
            redo: Vec::new(),
            related: Vec::new(),
            restore_only: true,
            snapshot: commit.snapshot.clone(),
            active_page,
            merge: None,
            at: Instant::now(),
        });
        self.cursor += 1;
        self.wash_out(session);
    }

    /// Discard everything right of the cursor, releasing session leases.
    fn truncate_forward(&mut self, session: &mut Session) {
        if self.cursor + 1 < self.states.len() {
            let discarded: Vec<HistoryState> = self.states.drain(self.cursor + 1..).collect();
            session.drop_history(
                discarded
                    .iter()
                    .flat_map(|state| state.related.iter().copied()),
            );
        }
    }

    /// Wash out the oldest states beyond capacity, anchor included; their
    /// inverses become unreachable, so the session drops their leases.
    fn wash_out(&mut self, session: &mut Session) {
        while self.states.len() > self.capacity {
            let removed = self.states.remove(0);
            session.drop_history(removed.related);
            self.cursor -= 1;
        }
    }

    /// Moves the cursor straight to `target` by adopting the state's pinned
    /// snapshot. Robust against interleaved commits (no inverse preconditions
    /// involved), but the commit carries an empty change set, so consumers do
    /// a full render instead of applying a diff.
    pub(crate) async fn jump(
        &mut self,
        session: &mut Session,
        target: usize,
    ) -> Result<Option<(Commit, Option<EntityId>)>> {
        if target >= self.states.len() {
            bail!("history target {} is out of range", target);
        }
        if target == self.cursor {
            return Ok(None);
        }
        let snapshot = self.states[target].snapshot.clone();
        let commit = session.restore(&snapshot).await?;
        self.cursor = target;
        let page = self.states[self.cursor].active_page;
        Ok(Some((commit, page)))
    }

    /// One undo step through the state's inverse operations, so the canvas
    /// re-renders from a diff instead of a full page render.
    pub(crate) async fn step_back(
        &mut self,
        session: &mut Session,
    ) -> Result<Option<(Commit, Option<EntityId>)>> {
        if self.cursor == 0 {
            return Ok(None);
        }
        // Restores have no inverse chain; cross them by adopting the
        // previous state's pinned snapshot.
        if self.states[self.cursor].restore_only {
            return self.jump(session, self.cursor - 1).await;
        }
        let forward = self.states[self.cursor].forward.clone();
        let commit = session.undo_many(forward.iter().copied()).await?;
        let state = &mut self.states[self.cursor];
        state.related.push(commit.revision);
        state.redo.push(commit.revision);
        self.cursor -= 1;
        let page = self.states[self.cursor].active_page;
        Ok(Some((commit, page)))
    }

    /// One redo step. Falls back to adopting the pinned snapshot when the
    /// redo commit was never materialized (the state was crossed by `jump`).
    pub(crate) async fn step_forward(
        &mut self,
        session: &mut Session,
    ) -> Result<Option<(Commit, Option<EntityId>)>> {
        if self.cursor + 1 == self.states.len() {
            return Ok(None);
        }
        self.cursor += 1;
        let revision = self.states[self.cursor].redo.pop();
        let commit = match revision {
            Some(revision) => {
                let commit = session.undo_many([revision]).await?;
                self.states[self.cursor].related.push(commit.revision);
                commit
            }
            None => {
                session
                    .restore(&self.states[self.cursor].snapshot.clone())
                    .await?
            }
        };
        let page = self.states[self.cursor].active_page;
        Ok(Some((commit, page)))
    }

    /// Drops every state except the current one, which becomes the anchor.
    pub(crate) fn clear(&mut self, session: &mut Session) {
        let mut current = self.states.swap_remove(self.cursor);
        let discarded = std::mem::take(&mut self.states);
        session.drop_history(
            current.related.drain(..).chain(
                discarded
                    .iter()
                    .flat_map(|state| state.related.iter().copied()),
            ),
        );
        current.forward.clear();
        current.redo.clear();
        current.restore_only = false;
        current.merge = None;
        self.states = vec![current];
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{At, PageDraft, Snapshot};

    use super::*;

    async fn add_page(session: &mut Session, label: &str) -> Commit {
        let label = label.to_owned();
        let patch = unary_patch(&session.snapshot(), label);
        session.commit(patch).await.unwrap()
    }

    fn unary_patch(snapshot: &Snapshot, label: String) -> koharu_scene::Patch {
        snapshot
            .patch(move |edit| {
                edit.add_page(PageDraft::new(label, 100.0, 100.0), At::End)?;
                Ok(())
            })
            .unwrap()
    }

    fn pages(session: &Session) -> usize {
        session.snapshot().pages().count()
    }

    #[tokio::test]
    async fn named_states_jump_both_ways() {
        let mut session = Session::memory().await.unwrap();
        let mut history = History::new(session.snapshot(), None, 10);
        let commit = add_page(&mut session, "a").await;
        assert!(history.record(
            &mut session,
            HistoryName::RenamePage,
            None,
            None,
            &commit,
            None
        ));
        let second = add_page(&mut session, "b").await;
        history.record(&mut session, HistoryName::Brush, None, None, &second, None);

        let view = history.view();
        assert_eq!(view.entries.len(), 3);
        assert_eq!(view.entries[0].name, HistoryName::Open);
        assert_eq!(view.entries[1].name, HistoryName::RenamePage);
        assert_eq!(view.entries[2].name, HistoryName::Brush);
        assert!(view.entries[2].current);
        assert!(history.can_undo() && !history.can_redo());

        let (commit, _) = history.jump(&mut session, 0).await.unwrap().unwrap();
        let _ = commit;
        assert_eq!(pages(&session), 0);
        let view = history.view();
        assert!(view.entries[1].undone && view.entries[2].undone);
        assert!(!history.can_undo() && history.can_redo());

        history.jump(&mut session, 2).await.unwrap();
        assert_eq!(pages(&session), 2);
        assert!(!history.can_redo());
    }

    #[tokio::test]
    async fn new_edit_truncates_undone_states() {
        let mut session = Session::memory().await.unwrap();
        let mut history = History::new(session.snapshot(), None, 10);
        for label in ["a", "b"] {
            let commit = add_page(&mut session, label).await;
            history.record(&mut session, HistoryName::Brush, None, None, &commit, None);
        }
        history.jump(&mut session, 0).await.unwrap();
        let commit = add_page(&mut session, "c").await;
        history.record(&mut session, HistoryName::Brush, None, None, &commit, None);

        let view = history.view();
        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.cursor, 1);
        assert!(!history.can_redo());
        history.jump(&mut session, 0).await.unwrap();
        assert_eq!(pages(&session), 0);
    }

    #[tokio::test]
    async fn repeated_micro_edits_coalesce_into_one_state() {
        let mut session = Session::memory().await.unwrap();
        let mut history = History::new(session.snapshot(), None, 10);
        let target = add_page(&mut session, "target").await;
        history.record(
            &mut session,
            HistoryName::AddText,
            None,
            None,
            &target,
            None,
        );
        let target_entity = session.snapshot().pages().next().unwrap().id();
        for label in ["x1", "x2"] {
            let commit = add_page(&mut session, label).await;
            history.record(
                &mut session,
                HistoryName::SourceText,
                None,
                Some(target_entity),
                &commit,
                None,
            );
        }

        let view = history.view();
        assert_eq!(
            view.entries.len(),
            3,
            "micro edits must merge with the open one"
        );
        history.jump(&mut session, 1).await.unwrap();
        assert_eq!(pages(&session), 1, "one undo state covers both micro edits");
    }

    #[tokio::test]
    async fn capacity_washes_out_oldest_states() {
        let mut session = Session::memory().await.unwrap();
        let mut history = History::new(session.snapshot(), None, 3);
        for index in 0..5 {
            let commit = add_page(&mut session, &index.to_string()).await;
            history.record(&mut session, HistoryName::Brush, None, None, &commit, None);
        }
        let view = history.view();
        assert_eq!(view.entries.len(), 3);
        assert_eq!(view.cursor, 2);

        history.jump(&mut session, 0).await.unwrap();
        // Only states a and b were washed out; the retained floor still
        // contains their pages, and undo stops there.
        assert_eq!(pages(&session), 3);
        assert!(!history.can_undo());
    }

    #[tokio::test]
    async fn clear_releases_revisions_and_resets_current_as_anchor() {
        let mut session = Session::memory().await.unwrap();
        let mut history = History::new(session.snapshot(), None, 10);
        let anchor = add_page(&mut session, "target").await;
        history.record(
            &mut session,
            HistoryName::AddText,
            None,
            None,
            &anchor,
            None,
        );
        let target = session.snapshot().pages().next().unwrap().id();
        let current = add_page(&mut session, "a").await;
        history.record(
            &mut session,
            HistoryName::SourceText,
            None,
            Some(target),
            &current,
            None,
        );

        history.clear(&mut session);

        let view = history.view();
        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.cursor, 0);
        assert!(!history.can_undo() && !history.can_redo());
        assert_eq!(pages(&session), 2);
        assert!(
            session.undo(current.revision).await.is_err(),
            "the new anchor's inverse is unreachable after clearing"
        );

        let next = add_page(&mut session, "b").await;
        history.record(
            &mut session,
            HistoryName::SourceText,
            None,
            Some(target),
            &next,
            None,
        );
        assert!(
            history.can_undo(),
            "a mergeable edit after clearing must not merge into the anchor"
        );
    }

    #[tokio::test]
    async fn snapshot_restore_is_itself_undoable() {
        let mut session = Session::memory().await.unwrap();
        let mut history = History::new(session.snapshot(), None, 10);
        let commit = add_page(&mut session, "a").await;
        history.record(&mut session, HistoryName::Brush, None, None, &commit, None);
        let snapshot_id = history.create_snapshot(session.snapshot(), None);
        let commit = add_page(&mut session, "b").await;
        history.record(&mut session, HistoryName::Brush, None, None, &commit, None);
        assert_eq!(pages(&session), 2);

        let (snapshot, name) = history
            .snapshot(snapshot_id)
            .map(|(snapshot, name)| (snapshot.clone(), name.to_owned()))
            .unwrap();
        let commit = session.restore(&snapshot).await.unwrap();
        history.record_restore(&mut session, Some(name), &commit, None);
        assert_eq!(pages(&session), 1);
        let view = history.view();
        assert_eq!(view.entries.len(), 4);
        assert_eq!(view.entries[3].name, HistoryName::SnapshotRestore);
        assert!(view.entries[3].current);

        history.step_back(&mut session).await.unwrap();
        assert_eq!(pages(&session), 2, "undo crosses snapshot restores");
        history.step_forward(&mut session).await.unwrap();
        assert_eq!(pages(&session), 1, "redo crosses snapshot restores");
    }
}
