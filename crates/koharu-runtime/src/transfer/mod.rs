//! Transfers into the store, and the stateful download manager tracking them.

mod activity;
mod download;

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use futures::{Stream, StreamExt, stream};
use parking_lot::Mutex;
use reqwest::Client;
use serde::Serialize;
use tokio::sync::{OnceCell, broadcast, watch};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use crate::huggingface::{PinnedFile, Repository};
use activity::SpeedTracker;
pub(crate) use download::BACKOFF;

/// The user agent every transfer sends.
pub(crate) const USER_AGENT: &str = concat!("koharu/", env!("CARGO_PKG_VERSION"));

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const BROADCAST_CAPACITY: usize = 256;

/// Error returned when a download is cancelled via `CancellationToken` or
/// `DownloadManager::cancel_download`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("download cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub(crate) fn is_cancelled(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Cancelled>())
}

/// Lifecycle state of a tracked download entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, specta::Type)]
pub enum DownloadState {
    Running,
    Finished,
    Failed(String),
    Cancelled,
}

/// Snapshot of a single grouped download entry tracked by `DownloadManager`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, specta::Type)]
pub struct DownloadEntry {
    pub id: u64,
    pub model_id: String,
    pub model_name: String,
    pub stage: Option<String>,
    pub completed: u64,
    pub total: u64,
    pub bytes_per_sec: u64,
    pub files_done: usize,
    pub files_total: usize,
    pub state: DownloadState,
}

#[derive(Clone, Debug)]
enum Completion {
    Ok,
    Cancelled,
    Failed(String),
}

/// Handle returned by [`DownloadManager::start`], allowing callers to inspect
/// the download entry ID synchronously or await completion.
#[derive(Debug)]
pub struct DownloadHandle(DownloadHandleInner);

#[derive(Debug)]
enum DownloadHandleInner {
    Ready,
    Pending {
        id: u64,
        rx: watch::Receiver<Option<Completion>>,
    },
}

impl DownloadHandle {
    fn ready() -> Self {
        Self(DownloadHandleInner::Ready)
    }

    fn pending(id: u64, rx: watch::Receiver<Option<Completion>>) -> Self {
        Self(DownloadHandleInner::Pending { id, rx })
    }

    #[must_use]
    pub fn id(&self) -> Option<u64> {
        match &self.0 {
            DownloadHandleInner::Ready => None,
            DownloadHandleInner::Pending { id, .. } => Some(*id),
        }
    }

    pub async fn wait(self) -> Result<()> {
        match self.0 {
            DownloadHandleInner::Ready => Ok(()),
            DownloadHandleInner::Pending { mut rx, .. } => {
                let completion = rx
                    .wait_for(Option::is_some)
                    .await
                    .context("download manager task dropped unexpectedly")?;
                match completion.as_ref().expect("wait_for guarantees Some") {
                    Completion::Ok => Ok(()),
                    Completion::Cancelled => Err(Cancelled.into()),
                    Completion::Failed(message) => bail!("{message}"),
                }
            }
        }
    }
}

type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;
type PinnedFetcher = Arc<
    dyn Fn(
            PinnedFile,
            CancellationToken,
            ProgressCallback,
        ) -> Pin<Box<dyn Future<Output = Result<u64>> + Send>>
        + Send
        + Sync,
>;
type ExistsPredicate = Arc<dyn Fn(&PinnedFile) -> bool + Send + Sync>;

#[derive(Clone)]
enum DownloadSource {
    Pinned {
        files: Vec<PinnedFile>,
        fetcher: PinnedFetcher,
        exists: ExistsPredicate,
    },
    Repo,
    Direct,
}

struct ManagedEntry {
    entry: DownloadEntry,
    source: DownloadSource,
    cancel: CancellationToken,
    done: watch::Sender<Option<Completion>>,
    tracker: SpeedTracker,
}

struct ManagerState {
    next_id: u64,
    entries: BTreeMap<u64, ManagedEntry>,
    active_by_model: HashMap<String, u64>,
}

impl ManagerState {
    fn new() -> Self {
        Self {
            next_id: 1,
            entries: BTreeMap::new(),
            active_by_model: HashMap::new(),
        }
    }
}

/// Process-wide stateful download manager tracking per-model transfers,
/// deduplicating concurrent requests, computing EWMA speed, and supporting
/// cancellation, retry, and dismissal.
pub struct DownloadManager {
    inner: Arc<Mutex<ManagerState>>,
    tx: broadcast::Sender<DownloadEntry>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    /// Returns the process-wide singleton `DownloadManager`.
    #[must_use]
    pub fn global() -> &'static Self {
        static GLOBAL: LazyLock<DownloadManager> = LazyLock::new(DownloadManager::new);
        &GLOBAL
    }

    /// Creates an isolated `DownloadManager` instance (primarily for tests).
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(ManagerState::new())),
            tx,
        }
    }

    /// Starts (or joins an already running) grouped download for `files` and
    /// returns a [`DownloadHandle`] immediately without blocking.
    pub fn start(
        &self,
        model_id: impl Into<String>,
        model_name: impl Into<String>,
        stage: Option<&str>,
        files: &[PinnedFile],
    ) -> DownloadHandle {
        self.start_pinned_with(
            model_id,
            model_name,
            stage,
            files,
            default_pinned_fetcher(),
            default_exists_predicate(),
        )
    }

    /// Ensures every file in `files` is available locally, starting or joining
    /// a grouped download if any file is missing.
    pub async fn ensure(
        &self,
        model_id: impl Into<String>,
        model_name: impl Into<String>,
        stage: Option<&str>,
        files: &[PinnedFile],
    ) -> Result<()> {
        self.start(model_id, model_name, stage, files).wait().await
    }

    /// Cancels an active download by ID. Returns an error if `id` is unknown.
    pub fn cancel_download(&self, id: u64) -> Result<()> {
        let state = self.inner.lock();
        let managed = state
            .entries
            .get(&id)
            .with_context(|| format!("download {id} not found"))?;
        if managed.entry.state == DownloadState::Running {
            managed.cancel.cancel();
        }
        Ok(())
    }

    /// Retries a `Failed` or `Cancelled` pinned-file download in place.
    pub fn retry_download(&self, id: u64) -> Result<()> {
        let task_spec = {
            let mut state = self.inner.lock();
            let model_id = state
                .entries
                .get(&id)
                .with_context(|| format!("download {id} not found"))?
                .entry
                .model_id
                .clone();

            let current_state = state.entries[&id].entry.state.clone();
            match current_state {
                DownloadState::Running | DownloadState::Finished => return Ok(()),
                DownloadState::Failed(_) | DownloadState::Cancelled => {}
            }

            if let Some(&active_id) = state.active_by_model.get(&model_id)
                && active_id != id
            {
                bail!("a download for `{model_id}` is already running");
            }

            let managed = state.entries.get_mut(&id).expect("checked above");
            let (files, fetcher, exists) = match &managed.source {
                DownloadSource::Pinned {
                    files,
                    fetcher,
                    exists,
                } => (files.clone(), Arc::clone(fetcher), Arc::clone(exists)),
                DownloadSource::Repo | DownloadSource::Direct => {
                    bail!("only model downloads can be retried")
                }
            };

            let (base_completed, files_done, pending) = partition_pinned(&files, exists.as_ref());
            let total: u64 = files.iter().map(|f| f.bytes).sum();

            if pending.is_empty() {
                managed.entry.completed = total.max(base_completed);
                managed.entry.total = total;
                managed.entry.files_done = files.len();
                managed.entry.files_total = files.len();
                managed.entry.bytes_per_sec = 0;
                managed.entry.state = DownloadState::Finished;
                let snapshot = managed.entry.clone();
                let done = managed.done.clone();
                let _ = self.tx.send(snapshot);
                let _ = done.send(Some(Completion::Ok));
                return Ok(());
            }

            let cancel = CancellationToken::new();
            let (done, _) = watch::channel(None);
            managed.cancel = cancel.clone();
            managed.done = done;
            managed.tracker = SpeedTracker::new(base_completed);
            managed.entry.completed = base_completed;
            managed.entry.total = total;
            managed.entry.bytes_per_sec = 0;
            managed.entry.files_done = files_done;
            managed.entry.files_total = files.len();
            managed.entry.state = DownloadState::Running;
            let snapshot = managed.entry.clone();
            state.active_by_model.insert(model_id, id);
            let _ = self.tx.send(snapshot);

            (pending, base_completed, cancel, fetcher)
        };

        let (pending, base_completed, cancel, fetcher) = task_spec;
        self.spawn_pinned_task(id, pending, base_completed, cancel, fetcher);
        Ok(())
    }

    /// Removes a non-running download entry from the tracked list.
    pub fn dismiss_download(&self, id: u64) -> Result<()> {
        let mut state = self.inner.lock();
        let Some(managed) = state.entries.get(&id) else {
            return Ok(());
        };
        if managed.entry.state == DownloadState::Running {
            bail!("cannot dismiss a running download");
        }
        state.entries.remove(&id);
        Ok(())
    }

    /// Returns a snapshot of all tracked downloads ordered by ascending `id`.
    #[must_use]
    pub fn list_downloads(&self) -> Vec<DownloadEntry> {
        let state = self.inner.lock();
        state
            .entries
            .values()
            .map(|managed| managed.entry.clone())
            .collect()
    }

    /// Returns a stream yielding the current snapshot of all entries followed
    /// by live updates without gaps.
    pub fn downloads(&self) -> impl Stream<Item = DownloadEntry> + Unpin {
        let (snapshot, rx) = {
            let state = self.inner.lock();
            let snapshot: Vec<DownloadEntry> = state
                .entries
                .values()
                .map(|managed| managed.entry.clone())
                .collect();
            let rx = self.tx.subscribe();
            (snapshot, rx)
        };
        let live = BroadcastStream::new(rx).filter_map(|item| futures::future::ready(item.ok()));
        stream::iter(snapshot).chain(live)
    }

    pub(crate) async fn resolve_repository(
        &self,
        repo: Repository,
        names: &[&str],
        cancel: &CancellationToken,
    ) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        if names.iter().all(|name| repo.exists(name)) {
            return Ok(());
        }

        let model_id = format!("{}:{}:{}", repo.id(), repo.commit(), names.join(","));
        let model_name = if names.len() == 1 {
            names[0].rsplit('/').next().unwrap_or(names[0]).to_string()
        } else {
            repo.id()
                .rsplit('/')
                .next()
                .unwrap_or(repo.id())
                .to_string()
        };

        let handle = self.start_repo(model_id, model_name, repo, names);
        let id = handle.id();
        let outcome = {
            let wait_fut = handle.wait();
            tokio::pin!(wait_fut);
            tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                result = &mut wait_fut => Some(result),
            }
        };

        match outcome {
            Some(result) => result,
            None => {
                if let Some(id) = id {
                    self.cancel_if_unshared(id);
                }
                Err(Cancelled.into())
            }
        }
    }

    pub(crate) async fn fetch_direct(&self, url: &str, destination: &Path) -> Result<()> {
        let name = destination
            .file_name()
            .context("a transfer's destination has no name")?
            .to_string_lossy()
            .into_owned();
        let model_id = format!("runtime:{name}");
        let (id, cancel) = self.register_direct(model_id, name);

        let inner = Arc::clone(&self.inner);
        let tx = self.tx.clone();
        let progress = Arc::new(move |completed: u64, total: u64| {
            update_direct_progress(&inner, &tx, id, completed, total);
        });

        let result = async {
            let client = client().await?;
            download::download(client, url, destination, &cancel, progress.as_ref()).await
        }
        .await;

        if result.is_err() {
            let _ = fs_err::tokio::remove_file(destination).await;
        }

        self.finish_entry(id, &result, true);
        result
    }

    fn start_pinned_with(
        &self,
        model_id: impl Into<String>,
        model_name: impl Into<String>,
        stage: Option<&str>,
        files: &[PinnedFile],
        fetcher: PinnedFetcher,
        exists: ExistsPredicate,
    ) -> DownloadHandle {
        let model_id = model_id.into();
        let model_name = model_name.into();
        let (base_completed, files_done, pending) = partition_pinned(files, exists.as_ref());
        if pending.is_empty() {
            return DownloadHandle::ready();
        }

        let total: u64 = files.iter().map(|f| f.bytes).sum();
        let (id, rx, cancel) = {
            let mut state = self.inner.lock();
            if let Some(&existing_id) = state.active_by_model.get(&model_id)
                && let Some(existing) = state.entries.get(&existing_id)
                && existing.entry.state == DownloadState::Running
                && !existing.cancel.is_cancelled()
            {
                return DownloadHandle::pending(existing_id, existing.done.subscribe());
            }

            let id = reusable_id(&state, &model_id).unwrap_or_else(|| {
                let id = state.next_id;
                state.next_id += 1;
                id
            });
            let cancel = CancellationToken::new();
            let (done, rx) = watch::channel(None);
            let entry = DownloadEntry {
                id,
                model_id: model_id.clone(),
                model_name,
                stage: stage.map(ToOwned::to_owned),
                completed: base_completed,
                total,
                bytes_per_sec: 0,
                files_done,
                files_total: files.len(),
                state: DownloadState::Running,
            };
            let managed = ManagedEntry {
                entry: entry.clone(),
                source: DownloadSource::Pinned {
                    files: files.to_vec(),
                    fetcher: Arc::clone(&fetcher),
                    exists,
                },
                cancel: cancel.clone(),
                done,
                tracker: SpeedTracker::new(base_completed),
            };
            state.entries.insert(id, managed);
            state.active_by_model.insert(model_id, id);
            let _ = self.tx.send(entry);
            (id, rx, cancel)
        };

        self.spawn_pinned_task(id, pending, base_completed, cancel, fetcher);
        DownloadHandle::pending(id, rx)
    }

    fn start_repo(
        &self,
        model_id: String,
        model_name: String,
        repo: Repository,
        names: &[&str],
    ) -> DownloadHandle {
        let pending_names: Vec<String> = names
            .iter()
            .filter(|name| !repo.exists(name))
            .map(|name| (*name).to_owned())
            .collect();
        if pending_names.is_empty() {
            return DownloadHandle::ready();
        }
        let files_done = names.len().saturating_sub(pending_names.len());
        let files_total = names.len();

        let (id, rx, cancel) = {
            let mut state = self.inner.lock();
            if let Some(&existing_id) = state.active_by_model.get(&model_id)
                && let Some(existing) = state.entries.get(&existing_id)
                && existing.entry.state == DownloadState::Running
                && !existing.cancel.is_cancelled()
            {
                return DownloadHandle::pending(existing_id, existing.done.subscribe());
            }

            let id = reusable_id(&state, &model_id).unwrap_or_else(|| {
                let id = state.next_id;
                state.next_id += 1;
                id
            });
            let cancel = CancellationToken::new();
            let (done, rx) = watch::channel(None);
            let entry = DownloadEntry {
                id,
                model_id: model_id.clone(),
                model_name,
                stage: None,
                completed: 0,
                total: 0,
                bytes_per_sec: 0,
                files_done,
                files_total,
                state: DownloadState::Running,
            };
            let managed = ManagedEntry {
                entry: entry.clone(),
                source: DownloadSource::Repo,
                cancel: cancel.clone(),
                done,
                tracker: SpeedTracker::new(0),
            };
            state.entries.insert(id, managed);
            state.active_by_model.insert(model_id, id);
            let _ = self.tx.send(entry);
            (id, rx, cancel)
        };

        let inner = Arc::clone(&self.inner);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut base_completed = 0u64;
            let mut completed_files = files_done;
            let mut outcome: Result<()> = Ok(());

            for name in pending_names {
                if cancel.is_cancelled() {
                    outcome = Err(Cancelled.into());
                    break;
                }
                let inner_cb = Arc::clone(&inner);
                let tx_cb = tx.clone();
                let progress: ProgressCallback = Arc::new(move |file_completed, file_total| {
                    update_repo_progress(
                        &inner_cb,
                        &tx_cb,
                        id,
                        base_completed,
                        file_completed,
                        file_total,
                    );
                });
                match repo.resolve_one(&name, &cancel, progress).await {
                    Ok(path) => {
                        let file_bytes = fs_err::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        base_completed = base_completed.saturating_add(file_bytes);
                        completed_files += 1;
                        mark_file_done(&inner, &tx, id, base_completed, completed_files, false);
                    }
                    Err(error) => {
                        outcome = Err(error);
                        break;
                    }
                }
            }

            finish_entry_with(&inner, &tx, id, &outcome, false);
        });

        DownloadHandle::pending(id, rx)
    }

    fn spawn_pinned_task(
        &self,
        id: u64,
        pending: Vec<PinnedFile>,
        mut base_completed: u64,
        cancel: CancellationToken,
        fetcher: PinnedFetcher,
    ) {
        let inner = Arc::clone(&self.inner);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut completed_files = {
                let state = inner.lock();
                state
                    .entries
                    .get(&id)
                    .map(|m| m.entry.files_done)
                    .unwrap_or(0)
            };
            let mut outcome: Result<()> = Ok(());

            for file in pending {
                if cancel.is_cancelled() {
                    outcome = Err(Cancelled.into());
                    break;
                }
                let inner_cb = Arc::clone(&inner);
                let tx_cb = tx.clone();
                let declared_bytes = file.bytes;
                let progress: ProgressCallback = Arc::new(move |file_completed, file_total| {
                    update_pinned_progress(
                        &inner_cb,
                        &tx_cb,
                        id,
                        base_completed,
                        declared_bytes,
                        file_completed,
                        file_total,
                    );
                });
                match fetcher(file, cancel.clone(), progress).await {
                    Ok(actual_bytes) => {
                        let delta = if file.bytes > 0 {
                            file.bytes
                        } else {
                            actual_bytes
                        };
                        base_completed = base_completed.saturating_add(delta);
                        completed_files += 1;
                        mark_file_done(&inner, &tx, id, base_completed, completed_files, false);
                    }
                    Err(error) => {
                        outcome = Err(error);
                        break;
                    }
                }
            }

            finish_entry_with(&inner, &tx, id, &outcome, true);
        });
    }

    fn register_direct(&self, model_id: String, model_name: String) -> (u64, CancellationToken) {
        let mut state = self.inner.lock();
        let id = reusable_id(&state, &model_id).unwrap_or_else(|| {
            let id = state.next_id;
            state.next_id += 1;
            id
        });
        let cancel = CancellationToken::new();
        let (done, _) = watch::channel(None);
        let entry = DownloadEntry {
            id,
            model_id: model_id.clone(),
            model_name,
            stage: Some("runtime".to_string()),
            completed: 0,
            total: 0,
            bytes_per_sec: 0,
            files_done: 0,
            files_total: 1,
            state: DownloadState::Running,
        };
        let managed = ManagedEntry {
            entry: entry.clone(),
            source: DownloadSource::Direct,
            cancel: cancel.clone(),
            done,
            tracker: SpeedTracker::new(0),
        };
        state.entries.insert(id, managed);
        state.active_by_model.insert(model_id, id);
        let _ = self.tx.send(entry);
        (id, cancel)
    }

    fn finish_entry(&self, id: u64, result: &Result<()>, preserve_declared_total: bool) {
        finish_entry_with(&self.inner, &self.tx, id, result, preserve_declared_total);
    }

    fn cancel_if_unshared(&self, id: u64) {
        let state = self.inner.lock();
        if let Some(managed) = state.entries.get(&id)
            && managed.entry.state == DownloadState::Running
            && managed.done.receiver_count() == 0
        {
            managed.cancel.cancel();
        }
    }
}

fn default_pinned_fetcher() -> PinnedFetcher {
    Arc::new(|file, cancel, progress| {
        Box::pin(async move {
            let path = file
                .repo
                .resolve_one(file.filename, &cancel, progress)
                .await?;
            let actual = fs_err::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Ok(actual)
        })
    })
}

fn default_exists_predicate() -> ExistsPredicate {
    Arc::new(PinnedFile::exists)
}

fn partition_pinned(
    files: &[PinnedFile],
    exists: &dyn Fn(&PinnedFile) -> bool,
) -> (u64, usize, Vec<PinnedFile>) {
    let mut base_completed = 0u64;
    let mut files_done = 0usize;
    let mut pending = Vec::new();
    for file in files {
        if exists(file) {
            base_completed = base_completed.saturating_add(file.bytes);
            files_done += 1;
        } else {
            pending.push(*file);
        }
    }
    (base_completed, files_done, pending)
}

fn reusable_id(state: &ManagerState, model_id: &str) -> Option<u64> {
    state
        .entries
        .iter()
        .find(|(_, managed)| {
            managed.entry.model_id == model_id
                && matches!(
                    managed.entry.state,
                    DownloadState::Failed(_) | DownloadState::Cancelled
                )
        })
        .map(|(&id, _)| id)
}

fn update_pinned_progress(
    inner: &Arc<Mutex<ManagerState>>,
    tx: &broadcast::Sender<DownloadEntry>,
    id: u64,
    base_completed: u64,
    declared_bytes: u64,
    file_completed: u64,
    file_total: u64,
) {
    let mut state = inner.lock();
    let Some(managed) = state.entries.get_mut(&id) else {
        return;
    };
    let clamped = if declared_bytes > 0 {
        file_completed.min(declared_bytes)
    } else {
        file_completed
    };
    let completed = base_completed.saturating_add(clamped);
    managed.entry.completed = completed;
    if managed.entry.total == 0 && file_total > 0 {
        managed.entry.total = base_completed.saturating_add(file_total);
    }
    let (rate, should_emit) = managed.tracker.record(completed, Instant::now());
    managed.entry.bytes_per_sec = rate;
    if should_emit {
        let _ = tx.send(managed.entry.clone());
    }
}

fn update_repo_progress(
    inner: &Arc<Mutex<ManagerState>>,
    tx: &broadcast::Sender<DownloadEntry>,
    id: u64,
    base_completed: u64,
    file_completed: u64,
    file_total: u64,
) {
    let mut state = inner.lock();
    let Some(managed) = state.entries.get_mut(&id) else {
        return;
    };
    let completed = base_completed.saturating_add(file_completed);
    managed.entry.completed = completed;
    if file_total > 0 {
        managed.entry.total = managed
            .entry
            .total
            .max(base_completed.saturating_add(file_total));
    }
    let (rate, should_emit) = managed.tracker.record(completed, Instant::now());
    managed.entry.bytes_per_sec = rate;
    if should_emit {
        let _ = tx.send(managed.entry.clone());
    }
}

fn update_direct_progress(
    inner: &Arc<Mutex<ManagerState>>,
    tx: &broadcast::Sender<DownloadEntry>,
    id: u64,
    completed: u64,
    total: u64,
) {
    let mut state = inner.lock();
    let Some(managed) = state.entries.get_mut(&id) else {
        return;
    };
    managed.entry.completed = completed;
    if total > 0 {
        managed.entry.total = total;
    }
    let (rate, should_emit) = managed.tracker.record(completed, Instant::now());
    managed.entry.bytes_per_sec = rate;
    if should_emit {
        let _ = tx.send(managed.entry.clone());
    }
}

fn mark_file_done(
    inner: &Arc<Mutex<ManagerState>>,
    tx: &broadcast::Sender<DownloadEntry>,
    id: u64,
    completed: u64,
    files_done: usize,
    is_direct: bool,
) {
    let mut state = inner.lock();
    let Some(managed) = state.entries.get_mut(&id) else {
        return;
    };
    managed.entry.completed = completed;
    managed.entry.files_done = files_done;
    if is_direct {
        managed.entry.total = managed.entry.total.max(completed);
    }
    let (rate, _) = managed.tracker.record(completed, Instant::now());
    managed.entry.bytes_per_sec = rate;
    let _ = tx.send(managed.entry.clone());
}

fn finish_entry_with(
    inner: &Arc<Mutex<ManagerState>>,
    tx: &broadcast::Sender<DownloadEntry>,
    id: u64,
    result: &Result<()>,
    preserve_declared_total: bool,
) {
    let (snapshot, done, completion) = {
        let mut state = inner.lock();
        let Some(managed) = state.entries.get_mut(&id) else {
            return;
        };
        let model_id = managed.entry.model_id.clone();
        let cancel_requested = managed.cancel.is_cancelled();
        managed.entry.bytes_per_sec = 0;

        let completion = match result {
            Ok(()) => {
                managed.entry.files_done = managed.entry.files_total;
                if preserve_declared_total && managed.entry.total > 0 {
                    managed.entry.completed = managed.entry.total;
                } else {
                    managed.entry.total = managed.entry.total.max(managed.entry.completed);
                    managed.entry.completed = managed.entry.total;
                }
                managed.entry.state = DownloadState::Finished;
                Completion::Ok
            }
            Err(error) if is_cancelled(error) || cancel_requested => {
                managed.entry.state = DownloadState::Cancelled;
                Completion::Cancelled
            }
            Err(error) => {
                let message = format!("{error:#}");
                managed.entry.state = DownloadState::Failed(message.clone());
                Completion::Failed(message)
            }
        };

        let snapshot = managed.entry.clone();
        let done = managed.done.clone();
        if state.active_by_model.get(&model_id) == Some(&id) {
            state.active_by_model.remove(&model_id);
        }
        (snapshot, done, completion)
    };

    let _ = tx.send(snapshot);
    let _ = done.send(Some(completion));
}

/// Every download's snapshot and subsequent state transitions; a receiver that
/// falls behind skips the oldest states it missed.
pub fn downloads() -> impl Stream<Item = DownloadEntry> + Unpin {
    DownloadManager::global().downloads()
}

/// Fetches `url` to `destination`, reporting progress under stage `"runtime"`
/// and removing `destination` if the transfer fails or is cancelled.
pub(crate) async fn fetch(url: &str, destination: &Path) -> Result<()> {
    DownloadManager::global()
        .fetch_direct(url, destination)
        .await
}

/// The client every transfer goes through, the Hub's included. A transfer
/// takes as long as the file does, so a response is never cut off for
/// arriving slowly.
pub(crate) async fn client() -> Result<&'static Client> {
    static CLIENT: OnceCell<Client> = OnceCell::const_new();
    CLIENT
        .get_or_try_init(|| async {
            Client::builder()
                .user_agent(USER_AGENT)
                .connect_timeout(CONNECT_TIMEOUT)
                .http2_adaptive_window(true)
                .tcp_keepalive(Duration::from_secs(60))
                .build()
                .context("couldn't build the HTTP client")
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Notify,
    };

    use super::*;
    use crate::store;

    const TEST_REPO: Repository = Repository::model(
        "test/mock-model",
        "0123456789abcdef0123456789abcdef01234567",
    );

    #[tokio::test]
    async fn grouping_initializes_total_tracks_ewma_and_replays_stream() {
        let manager = DownloadManager::new();
        let files = [
            PinnedFile::new(TEST_REPO, "a.bin", 1_000),
            PinnedFile::new(TEST_REPO, "b.bin", 2_000),
        ];

        let stream = manager.downloads();
        let fetcher: PinnedFetcher = Arc::new(|file, _cancel, progress| {
            Box::pin(async move {
                let half = file.bytes / 2;
                progress(half, file.bytes);
                tokio::time::sleep(Duration::from_millis(120)).await;
                progress(file.bytes, file.bytes);
                Ok(file.bytes)
            })
        });
        let exists: ExistsPredicate = Arc::new(|_| false);

        let handle = manager.start_pinned_with(
            "mock-model",
            "Mock Model",
            Some("detect"),
            &files,
            fetcher,
            exists,
        );
        let id = handle.id().expect("expected pending download");
        handle.wait().await.unwrap();

        let entries = manager.list_downloads();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].total, 3_000);
        assert_eq!(entries[0].completed, 3_000);
        assert_eq!(entries[0].files_done, 2);
        assert_eq!(entries[0].files_total, 2);
        assert_eq!(entries[0].bytes_per_sec, 0);
        assert_eq!(entries[0].state, DownloadState::Finished);

        // Stream should have emitted Running updates (with nonzero bytes_per_sec) and Finished.
        let mut collected = Vec::new();
        let mut stream = stream;
        while let Ok(Some(item)) =
            tokio::time::timeout(Duration::from_millis(30), stream.next()).await
        {
            collected.push(item);
        }
        assert!(collected.iter().any(|e| e.state == DownloadState::Running));
        assert!(collected.iter().any(|e| e.bytes_per_sec > 0));
        assert_eq!(collected.last().unwrap().state, DownloadState::Finished);
    }

    #[tokio::test]
    async fn concurrent_dedup_shares_id_and_cancel_notifies_all_waiters() {
        let manager = DownloadManager::new();
        let files = [PinnedFile::new(TEST_REPO, "weights.bin", 4_096)];
        let invocations = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());

        let invocations_clone = Arc::clone(&invocations);
        let started_clone = Arc::clone(&started);
        let fetcher: PinnedFetcher = Arc::new(move |_file, cancel, _progress| {
            let invocations = Arc::clone(&invocations_clone);
            let started = Arc::clone(&started_clone);
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::Relaxed);
                started.notify_waiters();
                cancel.cancelled().await;
                Err(Cancelled.into())
            })
        });
        let exists: ExistsPredicate = Arc::new(|_| false);

        let h1 = manager.start_pinned_with(
            "dedup-model",
            "Dedup Model",
            Some("translate"),
            &files,
            Arc::clone(&fetcher),
            Arc::clone(&exists),
        );
        let h2 = manager.start_pinned_with(
            "dedup-model",
            "Dedup Model",
            Some("translate"),
            &files,
            fetcher,
            exists,
        );

        let id1 = h1.id().unwrap();
        let id2 = h2.id().unwrap();
        assert_eq!(id1, id2);

        started.notified().await;
        assert_eq!(invocations.load(Ordering::Relaxed), 1);

        // Dropping one handle does not cancel the shared download.
        drop(h1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(manager.list_downloads()[0].state, DownloadState::Running);

        manager.cancel_download(id2).unwrap();
        let err = h2.wait().await.unwrap_err();
        assert!(is_cancelled(&err));
        assert_eq!(manager.list_downloads()[0].state, DownloadState::Cancelled);
    }

    #[tokio::test]
    async fn retry_and_dismiss_lifecycle() {
        let manager = DownloadManager::new();
        let files = [
            PinnedFile::new(TEST_REPO, "first.bin", 100),
            PinnedFile::new(TEST_REPO, "second.bin", 200),
        ];
        let fail_second = Arc::new(AtomicBool::new(true));
        let first_done = Arc::new(AtomicBool::new(false));

        let fail_clone = Arc::clone(&fail_second);
        let first_done_clone = Arc::clone(&first_done);
        let fetcher: PinnedFetcher = Arc::new(move |file, _cancel, _progress| {
            let fail_second = Arc::clone(&fail_clone);
            let first_done = Arc::clone(&first_done_clone);
            Box::pin(async move {
                if file.filename == "first.bin" {
                    first_done.store(true, Ordering::Relaxed);
                    return Ok(file.bytes);
                }
                if fail_second.load(Ordering::Relaxed) {
                    bail!("transient failure");
                }
                Ok(file.bytes)
            })
        });
        let first_done_exists = Arc::clone(&first_done);
        let exists: ExistsPredicate = Arc::new(move |file| {
            file.filename == "first.bin" && first_done_exists.load(Ordering::Relaxed)
        });

        let handle = manager.start_pinned_with(
            "retry-model",
            "Retry Model",
            Some("inpaint"),
            &files,
            fetcher,
            exists,
        );
        let id = handle.id().unwrap();
        assert!(handle.wait().await.is_err());

        let entry = &manager.list_downloads()[0];
        assert_eq!(
            entry.state,
            DownloadState::Failed("transient failure".into())
        );
        assert_eq!(entry.files_done, 1);

        fail_second.store(false, Ordering::Relaxed);
        manager.retry_download(id).unwrap();

        // Wait until retry reaches Finished.
        for _ in 0..50 {
            if manager.list_downloads()[0].state == DownloadState::Finished {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let finished = &manager.list_downloads()[0];
        assert_eq!(finished.id, id);
        assert_eq!(finished.state, DownloadState::Finished);
        assert_eq!(finished.completed, 300);
        assert_eq!(finished.files_done, 2);

        // Dismiss removes finished entry.
        manager.dismiss_download(id).unwrap();
        assert!(manager.list_downloads().is_empty());
    }

    #[tokio::test]
    async fn dismiss_running_download_errors() {
        let manager = DownloadManager::new();
        let files = [PinnedFile::new(TEST_REPO, "hang.bin", 100)];
        let fetcher: PinnedFetcher = Arc::new(|_file, cancel, _progress| {
            Box::pin(async move {
                cancel.cancelled().await;
                Err(Cancelled.into())
            })
        });
        let exists: ExistsPredicate = Arc::new(|_| false);

        let handle = manager.start_pinned_with(
            "running-model",
            "Running Model",
            None,
            &files,
            fetcher,
            exists,
        );
        let id = handle.id().unwrap();
        assert!(manager.dismiss_download(id).is_err());
        manager.cancel_download(id).unwrap();
        let _ = handle.wait().await;
        manager.dismiss_download(id).unwrap();
    }

    #[tokio::test]
    async fn direct_fetch_via_local_server_records_runtime_stage_and_cleans_stage_dir_on_cancel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut s1, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s1.read(&mut buf).await;
            let _ = s1
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhi")
                .await;
            let (mut s2, _) = listener.accept().await.unwrap();
            let _ = s2.read(&mut buf).await;
            let _ = s2
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhi")
                .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("pkg.tar.gz");
        let manager = Arc::new(DownloadManager::new());
        let url = format!("http://{addr}/pkg.tar.gz");

        let manager_clone = Arc::clone(&manager);
        let target_clone = target.clone();
        let put_fut = store::put(
            &target_clone,
            target_clone.clone(),
            |p| p.is_file(),
            move |stage| {
                let manager = Arc::clone(&manager_clone);
                let url = url.clone();
                async move {
                    let archive = stage.join("pkg.tar.gz");
                    manager.fetch_direct(&url, &archive).await?;
                    Ok(archive)
                }
            },
        );
        tokio::pin!(put_fut);

        // Wait until the direct download is registered as Running with stage "runtime".
        let mut id = None;
        for _ in 0..50 {
            if let Some(entry) = manager.list_downloads().first() {
                assert_eq!(entry.stage.as_deref(), Some("runtime"));
                id = Some(entry.id);
                break;
            }
            tokio::select! {
                _ = &mut put_fut => panic!("put finished unexpectedly"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
        let id = id.expect("expected direct download entry");
        manager.cancel_download(id).unwrap();
        let res = put_fut.await;
        assert!(is_cancelled(&res.unwrap_err()));
        assert!(!target.exists());

        // Verify no `.stage-*` directory remains inside `root`.
        let leftovers = stage_dirs(root.path());
        assert!(leftovers.is_empty(), "leftover stage dirs: {leftovers:?}");
    }

    fn stage_dirs(dir: &Path) -> Vec<std::path::PathBuf> {
        let Ok(entries) = fs_err::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(".stage-"))
            })
            .collect()
    }

    async fn spawn_mock_hf_server(payload: Vec<u8>) -> (std::net::SocketAddr, Arc<Notify>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let release = Arc::new(Notify::new());
        let release_srv = Arc::clone(&release);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let payload = payload.clone();
                let release = Arc::clone(&release_srv);
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 1024];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        req.extend_from_slice(&buf[..n]);
                        if req.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let req_line = String::from_utf8_lossy(&req);
                    let first_line = req_line.lines().next().unwrap_or("");
                    let mut parts = first_line.split_whitespace();
                    let method = parts.next().unwrap_or("");
                    let path = parts.next().unwrap_or("");

                    if path == "/_release" {
                        release.notify_waiters();
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                        return;
                    }

                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    if method == "HEAD" {
                        let _ = socket.write_all(header.as_bytes()).await;
                        return;
                    }

                    if socket.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    let split = payload.len().min(16);
                    if socket.write_all(&payload[..split]).await.is_err() {
                        return;
                    }
                    let _ = socket.flush().await;

                    release.notified().await;
                    let _ = socket.write_all(&payload[split..]).await;
                    let _ = socket.flush().await;
                });
            }
        });

        (addr, release)
    }

    async fn run_isolated_repo_cancel_child(exact_name: &str, payload: Vec<u8>) {
        let (addr, _release) = spawn_mock_hf_server(payload).await;
        let cache = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let output = tokio::process::Command::new(&executable)
            .args(["--exact", exact_name])
            .env("KOHARU_RUNTIME_REPO_CANCEL_CHILD", "1")
            .env("XDG_CACHE_HOME", cache.path())
            .env("HF_ENDPOINT", format!("http://{addr}"))
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "child test `{exact_name}` failed ({:?}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    async fn wait_for_entry_bytes(min_completed: u64) -> DownloadEntry {
        for _ in 0..200 {
            if let Some(entry) = DownloadManager::global()
                .list_downloads()
                .into_iter()
                .next()
                && entry.state == DownloadState::Running
                && entry.completed >= min_completed
            {
                return entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for active download progress >= {min_completed}; entries={:?}",
            DownloadManager::global().list_downloads()
        );
    }

    async fn wait_for_entry_state(expected: DownloadState) -> DownloadEntry {
        for _ in 0..200 {
            if let Some(entry) = DownloadManager::global()
                .list_downloads()
                .into_iter()
                .next()
                && entry.state == expected
            {
                return entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for state {expected:?}; entries={:?}",
            DownloadManager::global().list_downloads()
        );
    }

    #[tokio::test]
    async fn resolve_cancellable_single_caller_cancels_and_cleans_stage_dir() {
        const EXACT: &str =
            "transfer::tests::resolve_cancellable_single_caller_cancels_and_cleans_stage_dir";
        if std::env::var_os("KOHARU_RUNTIME_REPO_CANCEL_CHILD").is_none() {
            run_isolated_repo_cancel_child(EXACT, vec![7u8; 1024]).await;
            return;
        }

        const REPO: Repository = Repository::model(
            "test/single-stall",
            "0123456789abcdef0123456789abcdef01234567",
        );
        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();
        let task = tokio::spawn(async move {
            REPO.resolve_cancellable(["weights.bin"], &cancel_task)
                .await
        });

        let running = wait_for_entry_bytes(16).await;
        let snapshot_dir = REPO.path("weights.bin").parent().unwrap().to_path_buf();
        assert!(
            !stage_dirs(&snapshot_dir).is_empty(),
            "expected active .stage-* directory while download is stalled"
        );
        assert_eq!(running.state, DownloadState::Running);

        cancel.cancel();
        let err = task.await.unwrap().unwrap_err();
        assert!(is_cancelled(&err), "expected Cancelled, got {err:#}");

        let cancelled = wait_for_entry_state(DownloadState::Cancelled).await;
        assert_eq!(cancelled.id, running.id);
        assert!(!REPO.exists("weights.bin"));
        assert!(
            stage_dirs(&snapshot_dir).is_empty(),
            "leftover stage dirs: {:?}",
            stage_dirs(&snapshot_dir)
        );
    }

    #[tokio::test]
    async fn resolve_cancellable_shared_cancel_one_preserves_other() {
        const EXACT: &str =
            "transfer::tests::resolve_cancellable_shared_cancel_one_preserves_other";
        if std::env::var_os("KOHARU_RUNTIME_REPO_CANCEL_CHILD").is_none() {
            run_isolated_repo_cancel_child(EXACT, vec![42u8; 256]).await;
            return;
        }

        const REPO: Repository = Repository::model(
            "test/shared-one-cancel",
            "1123456789abcdef0123456789abcdef01234567",
        );
        let cancel_a = CancellationToken::new();
        let cancel_b = CancellationToken::new();
        let cancel_a_task = cancel_a.clone();
        let cancel_b_task = cancel_b.clone();

        let task_a = tokio::spawn(async move {
            REPO.resolve_cancellable(["shared.bin"], &cancel_a_task)
                .await
        });
        let task_b = tokio::spawn(async move {
            REPO.resolve_cancellable(["shared.bin"], &cancel_b_task)
                .await
        });

        let running = wait_for_entry_bytes(16).await;
        assert_eq!(DownloadManager::global().list_downloads().len(), 1);
        assert_eq!(running.state, DownloadState::Running);

        cancel_a.cancel();
        let err_a = task_a.await.unwrap().unwrap_err();
        assert!(is_cancelled(&err_a), "expected Cancelled, got {err_a:#}");

        tokio::time::sleep(Duration::from_millis(30)).await;
        let entries = DownloadManager::global().list_downloads();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].state, DownloadState::Running);

        let endpoint = std::env::var("HF_ENDPOINT").unwrap();
        let resp = client()
            .await
            .unwrap()
            .get(format!("{endpoint}/_release"))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        let [path] = task_b.await.unwrap().unwrap();
        assert_eq!(fs_err::read(&path).unwrap(), vec![42u8; 256]);
        let finished = wait_for_entry_state(DownloadState::Finished).await;
        assert_eq!(finished.id, running.id);
        let snapshot_dir = path.parent().unwrap();
        assert!(
            stage_dirs(snapshot_dir).is_empty(),
            "leftover stage dirs: {:?}",
            stage_dirs(snapshot_dir)
        );
    }

    #[tokio::test]
    async fn resolve_cancellable_shared_cancel_both_aborts_and_cleans_stage_dir() {
        const EXACT: &str =
            "transfer::tests::resolve_cancellable_shared_cancel_both_aborts_and_cleans_stage_dir";
        if std::env::var_os("KOHARU_RUNTIME_REPO_CANCEL_CHILD").is_none() {
            run_isolated_repo_cancel_child(EXACT, vec![9u8; 1024]).await;
            return;
        }

        const REPO: Repository = Repository::model(
            "test/shared-both-cancel",
            "2123456789abcdef0123456789abcdef01234567",
        );
        let cancel_a = CancellationToken::new();
        let cancel_b = CancellationToken::new();
        let cancel_a_task = cancel_a.clone();
        let cancel_b_task = cancel_b.clone();

        let task_a = tokio::spawn(async move {
            REPO.resolve_cancellable(["shared_stall.bin"], &cancel_a_task)
                .await
        });
        let task_b = tokio::spawn(async move {
            REPO.resolve_cancellable(["shared_stall.bin"], &cancel_b_task)
                .await
        });

        let running = wait_for_entry_bytes(16).await;
        let snapshot_dir = REPO
            .path("shared_stall.bin")
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(!stage_dirs(&snapshot_dir).is_empty());

        cancel_a.cancel();
        let err_a = task_a.await.unwrap().unwrap_err();
        assert!(is_cancelled(&err_a), "expected Cancelled, got {err_a:#}");

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            DownloadManager::global().list_downloads()[0].state,
            DownloadState::Running
        );

        cancel_b.cancel();
        let err_b = task_b.await.unwrap().unwrap_err();
        assert!(is_cancelled(&err_b), "expected Cancelled, got {err_b:#}");

        let cancelled = wait_for_entry_state(DownloadState::Cancelled).await;
        assert_eq!(cancelled.id, running.id);
        assert!(!REPO.exists("shared_stall.bin"));
        assert!(
            stage_dirs(&snapshot_dir).is_empty(),
            "leftover stage dirs: {:?}",
            stage_dirs(&snapshot_dir)
        );
    }
}
