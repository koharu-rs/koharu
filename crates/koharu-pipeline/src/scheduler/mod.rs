//! The task that owns the runs, so nothing is locked. It admits ready jobs
//! while a permit for a page in flight is free, each a Tokio task in one
//! `JoinSet`, and hears how each went from it, a panic included. Calls wait
//! for the GPU thread in its own line, by priority. The stages are known here
//! only by `Stage` and by the model `stages::key` names.

mod run;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use koharu_runtime::{DownloadManager, PinnedFile};
use koharu_scene::{LayerId, PageId, ProjectId};
use parking_lot::RwLock;
use petgraph::{
    Direction::{Incoming, Outgoing},
    stable_graph::NodeIndex,
};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::{self, JoinError, JoinSet},
    time::{self, Instant},
};

use self::run::{Job, build_graph};
pub(crate) use self::run::{Plan, Run};
use crate::{
    Event, Outcome, PipelineConfig, QueueOrderReason, QueueScope, QueuedRunSummary, RunId, Stage,
    event::Events,
    gpu::{Gpu, GpuSnapshot, Key, LockMode, Turn},
    stages::{self, Failed, Inputs, Member, NeededModel, Task, translate::Conversations},
};

/// Pages admitted and not yet written: one reading its input, one on the
/// GPU, one writing its result, so the CPU works beside the GPU while few
/// pages are held.
const AHEAD: usize = 3;
/// The most pages one recognition call reads.
const RECOGNIZE_PAGES: usize = 8;
/// Default chunk size handed to a combined translation task so `RerunNow` can
/// still preempt between chunks while the stage packs pages to the model's
/// token budget.
const AUTO_TRANSLATE_CHUNK: usize = 16;
const MAX_TRANSLATE_CHUNK: usize = 64;
/// How long a model no setting uses, one a run brought as Generative Fill's
/// does, stays after a run last ended, as Ollama keeps a model five minutes
/// after its last request, so filling again doesn't load it again.
const KEEP: Duration = Duration::from_secs(5 * 60);
/// FIFO aging credit per second of queue wait (`256 MiB/s`), preventing
/// starvation of large-VRAM batch requests.
const AGING_BYTES_PER_SEC: u64 = 256 * 1024 * 1024;

pub(crate) enum Command {
    Run(Run),
    Cancel(Option<RunId>),
    CancelJob {
        run: RunId,
        page: Option<PageId>,
        stage: Stage,
    },
    Unload,
    /// The settings changed: unload what nothing uses now.
    Settle,
    Close(ProjectId),
}

pub(crate) struct DownloadWaiter {
    pub(crate) id: Option<u64>,
    pub(crate) wait: Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>,
}

pub(crate) trait DownloadOps: Send + Sync {
    fn start(
        &self,
        model_id: &str,
        model_name: &str,
        stage: Stage,
        files: &[PinnedFile],
    ) -> DownloadWaiter;

    fn cancel_download(&self, id: u64);
}

struct RuntimeDownloads;

impl DownloadOps for RuntimeDownloads {
    fn start(
        &self,
        model_id: &str,
        model_name: &str,
        stage: Stage,
        files: &[PinnedFile],
    ) -> DownloadWaiter {
        let stage_name = stage.to_string();
        let handle =
            DownloadManager::global().start(model_id, model_name, Some(&stage_name), files);
        let id = handle.id();
        DownloadWaiter {
            id,
            wait: Box::pin(handle.wait()),
        }
    }

    fn cancel_download(&self, id: u64) {
        let _ = DownloadManager::global().cancel_download(id);
    }
}

struct WaitingRun {
    run: Run,
    pending: BTreeMap<u64, Stage>,
}

struct QueuedRun {
    run: Run,
    enqueued_at: Instant,
}

/// How a run is asked for, as BigQuery runs a job interactive or batch: an
/// interactive run, on blocks, an area, metadata, or `RerunNow`, goes ahead of
/// a batch run on pages.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Mode {
    Interactive,
    Batch,
}

/// A job's place in line; the smallest goes first. Within a run, stages are
/// batched across all pages in stage order (`Detect`, `Recognize`, `Inpaint`,
/// `Translate`) and page order within each stage.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Priority {
    mode: Mode,
    run: RunId,
    stage: Stage,
    page: usize,
}

impl Priority {
    #[cfg(test)]
    pub(crate) fn test_new(mode: Mode, run: RunId, page: usize, stage: Stage) -> Self {
        Self {
            mode,
            run,
            stage,
            page,
        }
    }
}

/// A ready job: its run's place among the runs, and its node.
#[derive(Clone, Copy)]
struct Candidate {
    run: usize,
    job: NodeIndex,
    stage: Stage,
    priority: Priority,
}

/// A running task's stage, its jobs, and the page it fails on (`None` for metadata).
struct Running {
    stage: Stage,
    jobs: Vec<(RunId, NodeIndex)>,
    page: Option<PageId>,
}

pub(crate) struct Scheduler {
    commands: mpsc::UnboundedReceiver<Command>,
    events: Events,
    gpu: Gpu,
    gpu_snapshot: GpuSnapshot,
    downloads: Arc<dyn DownloadOps>,
    files_needed_fn: fn(&BTreeSet<Stage>, &PipelineConfig) -> anyhow::Result<Vec<NeededModel>>,
    config: Arc<RwLock<PipelineConfig>>,
    /// Runs waiting off the GPU thread for missing model downloads.
    waiting_downloads: Vec<WaitingRun>,
    /// Batch `Normal` runs waiting for the single active batch slot.
    queued: Vec<QueuedRun>,
    /// At most one `Mode::Batch` run plus any `Mode::Interactive` / `RerunNow` runs.
    active: Vec<Run>,
    /// A permit for each page in flight, which its task holds.
    ahead: Arc<Semaphore>,
    tasks: JoinSet<anyhow::Result<Vec<(NodeIndex, String)>>>,
    download_tasks: JoinSet<(RunId, u64, anyhow::Result<()>)>,
    running: HashMap<task::Id, Running>,
    conversations: Conversations,
    /// Models still needed by each active run, read by `Models::prepare`.
    needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
    /// When the models no setting uses leave: `KEEP` after a run last ended.
    idle: Option<Instant>,
    last_queue_order: Vec<QueuedRunSummary>,
}

impl Scheduler {
    /// Starts the scheduler and the GPU thread. They end when every sender
    /// the pipeline holds drops.
    pub(crate) fn spawn_with_lock(
        config: Arc<RwLock<PipelineConfig>>,
        events: Events,
        lock: Option<Arc<koharu_runtime::AcceleratorLock>>,
    ) -> mpsc::UnboundedSender<Command> {
        let needed_by_run = Arc::new(RwLock::new(HashMap::new()));
        let lock_mode = match lock {
            Some(lock) => LockMode::Preacquired(lock),
            None => LockMode::AcquireIn(None),
        };
        let (gpu, gpu_snapshot) =
            Gpu::spawn_with(Handle::current(), Arc::clone(&needed_by_run), lock_mode);
        let (sender, scheduler) = Self::new_with(
            config,
            events,
            gpu,
            gpu_snapshot,
            needed_by_run,
            Arc::new(RuntimeDownloads),
            stages::files_needed,
        );
        tokio::spawn(scheduler.run());
        sender
    }

    pub(crate) fn new_with(
        config: Arc<RwLock<PipelineConfig>>,
        events: Events,
        gpu: Gpu,
        gpu_snapshot: GpuSnapshot,
        needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
        downloads: Arc<dyn DownloadOps>,
        files_needed_fn: fn(&BTreeSet<Stage>, &PipelineConfig) -> anyhow::Result<Vec<NeededModel>>,
    ) -> (mpsc::UnboundedSender<Command>, Self) {
        let (sender, commands) = mpsc::unbounded_channel();
        let scheduler = Self {
            commands,
            events,
            gpu,
            gpu_snapshot,
            downloads,
            files_needed_fn,
            config,
            waiting_downloads: Vec::new(),
            queued: Vec::new(),
            active: Vec::new(),
            ahead: Arc::new(Semaphore::new(AHEAD)),
            tasks: JoinSet::new(),
            download_tasks: JoinSet::new(),
            running: HashMap::new(),
            conversations: Conversations::default(),
            needed_by_run,
            idle: None,
            last_queue_order: Vec::new(),
        };
        (sender, scheduler)
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => self.handle(command),
                    None => break,
                },
                Some(joined) = self.download_tasks.join_next() => self.download_done(joined),
                Some(joined) = self.tasks.join_next_with_id() => self.done(joined),
                () = time::sleep_until(self.idle.unwrap_or_else(Instant::now)),
                    if self.idle.is_some() =>
                {
                    self.idle = None;
                    self.settle();
                }
            }
            self.admit();
        }
        // The pipeline is gone: what's waiting, queued, or running stops, and nothing is written.
        for run in &self.active {
            run.cancel.cancel();
        }
        for q in &self.queued {
            q.run.cancel.cancel();
        }
        for w in self.waiting_downloads.drain(..) {
            w.run.cancel.cancel();
            for dl_id in w.pending.keys() {
                self.downloads.cancel_download(*dl_id);
            }
        }
        while self.tasks.join_next().await.is_some() {}
    }

    fn emit(&self, event: Event) {
        self.events.send(event);
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Run(run) if run.graph.node_count() == 0 => self.emit(Event::Ended {
                run: run.id,
                outcome: Outcome::Done,
            }),
            Command::Run(run) => {
                let graph_stages = stages_requiring_models(&run);
                let needed = match (self.files_needed_fn)(&graph_stages, &run.config) {
                    Ok(needed) => needed,
                    Err(error) => {
                        let stage = graph_stages
                            .iter()
                            .copied()
                            .next_back()
                            .unwrap_or(Stage::Translate);
                        self.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Failed {
                                page: None,
                                stage,
                                reason: format!("{error:#}"),
                            },
                        });
                        return;
                    }
                };
                let mut pending = BTreeMap::new();
                let mut waiters = Vec::new();
                for model in needed {
                    let waiter = self.downloads.start(
                        &model.model_id,
                        &model.model_name,
                        model.stage,
                        &model.files,
                    );
                    if let Some(id) = waiter.id {
                        pending.entry(id).or_insert(model.stage);
                        waiters.push((id, waiter.wait));
                    }
                }
                if waiters.is_empty() {
                    self.enqueue_or_activate(run);
                } else {
                    let run_id = run.id;
                    let downloads: Vec<u64> = pending.keys().copied().collect();
                    self.waiting_downloads.push(WaitingRun { run, pending });
                    self.emit(Event::WaitingForDownload {
                        run: run_id,
                        downloads,
                    });
                    for (dl_id, wait) in waiters {
                        self.download_tasks
                            .spawn(async move { (run_id, dl_id, wait.await) });
                    }
                    self.emit_queue_updated();
                }
            }
            Command::Cancel(Some(id)) => self.cancel_one(id),
            Command::Cancel(None) => self.cancel_all(),
            Command::CancelJob { run, page, stage } => self.cancel_job(run, page, stage),
            Command::Unload => self
                .gpu
                .run(Turn::Upkeep, |models| models.retain(&HashSet::new())),
            Command::Settle => self.settle(),
            Command::Close(project) => {
                self.cancel_project(project);
                self.conversations.close(project);
            }
        }
    }

    fn download_done(&mut self, joined: Result<(RunId, u64, anyhow::Result<()>), JoinError>) {
        let Ok((run_id, dl_id, result)) = joined else {
            return;
        };
        let Some(pos) = self
            .waiting_downloads
            .iter()
            .position(|waiting| waiting.run.id == run_id)
        else {
            return;
        };
        match result {
            Ok(()) => {
                self.waiting_downloads[pos].pending.remove(&dl_id);
                if self.waiting_downloads[pos].pending.is_empty() {
                    let waiting = self.waiting_downloads.remove(pos);
                    self.enqueue_or_activate(waiting.run);
                } else {
                    self.emit_queue_updated();
                }
            }
            Err(error) => {
                let waiting = self.waiting_downloads.remove(pos);
                let failed_stage = waiting
                    .pending
                    .get(&dl_id)
                    .copied()
                    .unwrap_or(Stage::Detect);
                let outcome = if error.chain().any(|c| c.is::<koharu_runtime::Cancelled>()) {
                    Outcome::Stopped
                } else {
                    Outcome::Failed {
                        page: None,
                        stage: failed_stage,
                        reason: format!("{failed_stage} download failed: {error:#}"),
                    }
                };
                waiting.run.cancel.cancel();
                self.emit(Event::Ended {
                    run: run_id,
                    outcome,
                });
                for &other_dl_id in waiting.pending.keys() {
                    if other_dl_id != dl_id && !self.is_download_referenced(other_dl_id) {
                        self.downloads.cancel_download(other_dl_id);
                    }
                }
                self.emit_queue_updated();
            }
        }
    }

    fn is_download_referenced(&self, dl_id: u64) -> bool {
        self.waiting_downloads
            .iter()
            .any(|waiting| waiting.pending.contains_key(&dl_id))
    }

    fn enqueue_or_activate(&mut self, run: Run) {
        match run.mode {
            Mode::Interactive => {
                self.active.push(run);
                self.sync_needed();
                self.emit_queue_updated();
            }
            Mode::Batch => {
                self.queued.push(QueuedRun {
                    run,
                    enqueued_at: Instant::now(),
                });
                self.coalesce_queued();
                self.promote_next_batch();
                self.emit_queue_updated();
            }
        }
    }

    fn coalesce_queued(&mut self) {
        if self.queued.len() < 2 {
            return;
        }
        // Scan from the newly appended item (or any overlapping cluster) until fixed point.
        let mut idx = self.queued.len() - 1;
        while idx < self.queued.len() {
            let project = self.queued[idx].run.project;
            let config = Arc::clone(&self.queued[idx].run.config);
            let mut group = vec![idx];
            let mut group_items = run_work_items(&self.queued[idx].run);
            loop {
                let mut added = false;
                for (cand_idx, cand) in self.queued.iter().enumerate() {
                    if group.contains(&cand_idx)
                        || cand.run.project != project
                        || cand.run.config != config
                    {
                        continue;
                    }
                    let items = run_work_items(&cand.run);
                    if !items.is_disjoint(&group_items) {
                        group.push(cand_idx);
                        group_items.extend(items);
                        added = true;
                    }
                }
                if !added {
                    break;
                }
            }
            if group.len() > 1 {
                group.sort_by_key(|&i| self.queued[i].run.id);
                let target_idx = group[0];
                let target_id = self.queued[target_idx].run.id;

                let mut page_stages: HashMap<PageId, (BTreeSet<Stage>, Option<Arc<[LayerId]>>)> =
                    HashMap::new();
                let mut metadata = false;
                let mut is_project_scope = false;
                let mut all_stages = BTreeSet::new();

                for &member_idx in &group {
                    let run = &self.queued[member_idx].run;
                    if run.scope == QueueScope::Project {
                        is_project_scope = true;
                    }
                    for job in run.graph.node_weights() {
                        all_stages.insert(job.stage);
                        if let Some(page) = job.page {
                            let entry = page_stages
                                .entry(page)
                                .or_insert_with(|| (BTreeSet::new(), job.blocks.clone()));
                            entry.0.insert(job.stage);
                        } else {
                            metadata = true;
                        }
                    }
                }

                let project_pages: Vec<PageId> = self.queued[target_idx]
                    .run
                    .session
                    .lock()
                    .project()
                    .pages()
                    .map(|page| page.id)
                    .collect();
                let mut ordered_pages = Vec::with_capacity(page_stages.len());
                for page_id in project_pages {
                    if let Some((stages, blocks)) = page_stages.remove(&page_id) {
                        ordered_pages.push((page_id, stages, blocks));
                    }
                }
                for (page_id, (stages, blocks)) in page_stages {
                    ordered_pages.push((page_id, stages, blocks));
                }

                let target = &mut self.queued[target_idx].run;
                target.graph = build_graph(&ordered_pages, metadata, &target.cancel);
                target.stages = all_stages;
                target.scope = if is_project_scope {
                    QueueScope::Project
                } else {
                    QueueScope::Pages {
                        count: ordered_pages.len(),
                    }
                };

                let absorbed_ids: Vec<RunId> =
                    group[1..].iter().map(|&i| self.queued[i].run.id).collect();
                let mut remove_indices = group[1..].to_vec();
                remove_indices.sort_unstable_by(|a, b| b.cmp(a));
                for remove_idx in remove_indices {
                    self.queued.remove(remove_idx);
                }
                for absorbed_id in absorbed_ids {
                    self.emit(Event::Coalesced {
                        run: absorbed_id,
                        into: target_id,
                    });
                }
                break;
            }
            if idx == 0 {
                break;
            }
            idx -= 1;
        }
    }

    fn promote_next_batch(&mut self) {
        if self.active.iter().any(|run| run.mode == Mode::Batch) {
            return;
        }
        let ordered = self.score_queued();
        let Some((best_idx, _)) = ordered.first().copied() else {
            return;
        };
        let chosen = self.queued.remove(best_idx);
        self.active.push(chosen.run);
        self.sync_needed();
    }

    fn score_queued(&self) -> Vec<(usize, QueueOrderReason)> {
        if self.queued.is_empty() {
            return Vec::new();
        }
        let (resident, device_key, free_vram) = self.gpu_snapshot.snapshot();
        let vram = self.gpu_snapshot.vram();
        let now = Instant::now();

        let model_bytes = |key: &Key| -> u64 {
            let Ok(files) = stages::files_for_key(key) else {
                return 0;
            };
            if let Some(dk) = device_key.as_ref() {
                let fp_key = crate::vram::FootprintKey {
                    device: dk.clone(),
                    model_key: crate::vram::model_key(key.stage, &key.id, key.vision),
                    revision: crate::vram::revision_for_files(&files),
                };
                if let Some(fp) = vram.get(&fp_key) {
                    return if fp.load_bytes > 0 {
                        fp.load_bytes
                    } else {
                        fp.estimate(None)
                    };
                }
            }
            files.iter().map(|f| f.bytes).sum()
        };

        struct Scored {
            idx: usize,
            run_id: RunId,
            effective_cost: u64,
            raw_cost: u64,
            grouped: bool,
            aged: bool,
        }

        let mut scored: Vec<Scored> = self
            .queued
            .iter()
            .enumerate()
            .map(|(idx, q)| {
                let needed_keys: HashSet<Key> = q
                    .run
                    .graph
                    .node_weights()
                    .filter_map(|job| stages::key(job.stage, &q.run.config))
                    .collect();
                let grouped =
                    !needed_keys.is_empty() && needed_keys.iter().all(|key| resident.contains(key));
                let load_bytes: u64 = needed_keys
                    .iter()
                    .filter(|k| !resident.contains(*k))
                    .map(&model_bytes)
                    .sum();
                let eviction_bytes: u64 = if load_bytes == 0 {
                    0
                } else if free_vram.is_some_and(|free| {
                    free >= load_bytes.saturating_add(crate::vram::SAFETY_MARGIN)
                }) {
                    0
                } else {
                    resident
                        .iter()
                        .filter(|k| !needed_keys.contains(*k))
                        .map(&model_bytes)
                        .sum()
                };
                let wait_secs = now.saturating_duration_since(q.enqueued_at).as_secs_f64();
                let aging_bonus = (wait_secs * AGING_BYTES_PER_SEC as f64) as u64;
                let raw_cost = load_bytes.saturating_add(eviction_bytes);
                let effective_cost = raw_cost.saturating_sub(aging_bonus);
                Scored {
                    idx,
                    run_id: q.run.id,
                    effective_cost,
                    raw_cost,
                    grouped,
                    aged: aging_bonus > 0 && effective_cost < raw_cost,
                }
            })
            .collect();

        scored.sort_by_key(|s| (s.effective_cost, s.run_id));

        scored
            .iter()
            .enumerate()
            .map(|(pos, s)| {
                let jumped_earlier_run = scored[pos + 1..].iter().any(|o| o.run_id < s.run_id);
                let reason = if s.grouped {
                    QueueOrderReason::GroupedByLoadedModel
                } else if jumped_earlier_run
                    && s.raw_cost
                        < scored[pos + 1..]
                            .iter()
                            .filter(|o| o.run_id < s.run_id)
                            .map(|o| o.raw_cost)
                            .max()
                            .unwrap_or(0)
                {
                    QueueOrderReason::LowerVramCost
                } else if s.aged
                    && scored[pos + 1..]
                        .iter()
                        .any(|o| (o.raw_cost, o.run_id) < (s.raw_cost, s.run_id))
                {
                    QueueOrderReason::AgedPriority
                } else {
                    QueueOrderReason::Fifo
                };
                (s.idx, reason)
            })
            .collect()
    }

    fn emit_queue_updated(&mut self) {
        let scored = self.score_queued();
        let mut order: Vec<QueuedRunSummary> = scored
            .into_iter()
            .map(|(idx, reason)| {
                let run = &self.queued[idx].run;
                let jobs = run.ordered_jobs();
                QueuedRunSummary {
                    run: run.id,
                    steps: jobs.len(),
                    scope: run.scope,
                    stages: run.stages.clone(),
                    jobs,
                    priority: run.priority,
                    waiting_downloads: Vec::new(),
                    reason: Some(reason),
                }
            })
            .collect();

        let mut waiting_indices: Vec<usize> = (0..self.waiting_downloads.len()).collect();
        waiting_indices.sort_by_key(|&i| self.waiting_downloads[i].run.id);
        for idx in waiting_indices {
            let waiting = &self.waiting_downloads[idx];
            let jobs = waiting.run.ordered_jobs();
            order.push(QueuedRunSummary {
                run: waiting.run.id,
                steps: jobs.len(),
                scope: waiting.run.scope,
                stages: waiting.run.stages.clone(),
                jobs,
                priority: waiting.run.priority,
                waiting_downloads: waiting.pending.keys().copied().collect(),
                reason: Some(QueueOrderReason::WaitingForDownload),
            });
        }

        if order != self.last_queue_order {
            self.last_queue_order = order.clone();
            self.emit(Event::QueueUpdated { order });
        }
    }

    fn cancel_job(&mut self, id: RunId, page: Option<PageId>, stage: Stage) {
        if let Some(pos) = self
            .waiting_downloads
            .iter()
            .position(|waiting| waiting.run.id == id)
        {
            let node = self.waiting_downloads[pos]
                .run
                .graph
                .node_indices()
                .find(|&n| {
                    let job = &self.waiting_downloads[pos].run.graph[n];
                    job.page == page && job.stage == stage && !job.cancelled
                });
            let Some(node) = node else {
                return;
            };
            self.waiting_downloads[pos]
                .run
                .remove_job_preserving_chain(node);
            self.waiting_downloads[pos].run.stages = self.waiting_downloads[pos]
                .run
                .graph
                .node_weights()
                .map(|j| j.stage)
                .collect();
            self.emit(Event::JobCancelled {
                run: id,
                page,
                stage,
            });
            if self.waiting_downloads[pos].run.graph.node_count() == 0 {
                let waiting = self.waiting_downloads.remove(pos);
                waiting.run.cancel.cancel();
                self.emit(Event::Ended {
                    run: id,
                    outcome: Outcome::Stopped,
                });
                for dl_id in waiting.pending.keys() {
                    if !self.is_download_referenced(*dl_id) {
                        self.downloads.cancel_download(*dl_id);
                    }
                }
                self.emit_queue_updated();
                return;
            }
            let still_needed = stages_requiring_models(&self.waiting_downloads[pos].run);
            let mut dropped_downloads = Vec::new();
            self.waiting_downloads[pos]
                .pending
                .retain(|&dl_id, dl_stage| {
                    let keep = still_needed.contains(dl_stage);
                    if !keep {
                        dropped_downloads.push(dl_id);
                    }
                    keep
                });
            for dl_id in dropped_downloads {
                if !self.is_download_referenced(dl_id) {
                    self.downloads.cancel_download(dl_id);
                }
            }
            if self.waiting_downloads[pos].pending.is_empty() {
                let waiting = self.waiting_downloads.remove(pos);
                self.enqueue_or_activate(waiting.run);
            } else {
                self.emit_queue_updated();
            }
            return;
        }

        if let Some(pos) = self.queued.iter().position(|q| q.run.id == id) {
            let node = self.queued[pos].run.graph.node_indices().find(|&n| {
                let job = &self.queued[pos].run.graph[n];
                job.page == page && job.stage == stage && !job.cancelled
            });
            let Some(node) = node else {
                return;
            };
            self.queued[pos].run.remove_job_preserving_chain(node);
            self.queued[pos].run.stages = self.queued[pos]
                .run
                .graph
                .node_weights()
                .map(|j| j.stage)
                .collect();
            self.emit(Event::JobCancelled {
                run: id,
                page,
                stage,
            });
            if self.queued[pos].run.graph.node_count() == 0 {
                let q = self.queued.remove(pos);
                q.run.cancel.cancel();
                self.emit(Event::Ended {
                    run: id,
                    outcome: Outcome::Stopped,
                });
            }
            self.emit_queue_updated();
            return;
        }

        if let Some(pos) = self.active.iter().position(|r| r.id == id) {
            let node = self.active[pos].graph.node_indices().find(|&n| {
                let job = &self.active[pos].graph[n];
                job.page == page && job.stage == stage && !job.cancelled
            });
            let Some(node) = node else {
                return;
            };
            if self.active[pos].graph[node].running {
                let job = &mut self.active[pos].graph[node];
                job.cancelled = true;
                job.cancel.cancel();
                self.emit(Event::JobCancelled {
                    run: id,
                    page,
                    stage,
                });
                return;
            }
            self.active[pos].remove_job_preserving_chain(node);
            self.active[pos].stages = self.active[pos]
                .graph
                .node_weights()
                .map(|j| j.stage)
                .collect();
            self.emit(Event::JobCancelled {
                run: id,
                page,
                stage,
            });
            if self.active[pos].graph.node_count() == 0 {
                let run = self.active.remove(pos);
                let outcome = if run.finished_steps > 0 {
                    Outcome::Done
                } else {
                    Outcome::Stopped
                };
                self.emit(Event::Ended { run: id, outcome });
                self.sync_needed();
                self.promote_next_batch();
                self.emit_queue_updated();
                self.rest();
            } else {
                self.sync_needed();
                self.emit_queue_updated();
            }
        }
    }

    fn cancel_one(&mut self, id: RunId) {
        if let Some(pos) = self
            .waiting_downloads
            .iter()
            .position(|waiting| waiting.run.id == id)
        {
            let waiting = self.waiting_downloads.remove(pos);
            waiting.run.cancel.cancel();
            self.emit(Event::Ended {
                run: id,
                outcome: Outcome::Stopped,
            });
            for dl_id in waiting.pending.keys() {
                if !self.is_download_referenced(*dl_id) {
                    self.downloads.cancel_download(*dl_id);
                }
            }
            self.emit_queue_updated();
            return;
        }
        if let Some(pos) = self.queued.iter().position(|q| q.run.id == id) {
            let q = self.queued.remove(pos);
            q.run.cancel.cancel();
            self.emit(Event::Ended {
                run: id,
                outcome: Outcome::Stopped,
            });
            self.emit_queue_updated();
            return;
        }
        if let Some(pos) = self.active.iter().position(|r| r.id == id) {
            let run = self.active.remove(pos);
            run.cancel.cancel();
            self.emit(Event::Ended {
                run: id,
                outcome: Outcome::Stopped,
            });
            self.sync_needed();
            self.promote_next_batch();
            self.emit_queue_updated();
            self.rest();
        }
    }

    fn cancel_all(&mut self) {
        let had_waiting = !self.waiting_downloads.is_empty();
        let had_queued = !self.queued.is_empty();
        let had_active = !self.active.is_empty();
        for waiting in self.waiting_downloads.drain(..) {
            waiting.run.cancel.cancel();
            self.events.send(Event::Ended {
                run: waiting.run.id,
                outcome: Outcome::Stopped,
            });
            for dl_id in waiting.pending.keys() {
                self.downloads.cancel_download(*dl_id);
            }
        }
        for q in self.queued.drain(..) {
            q.run.cancel.cancel();
            self.events.send(Event::Ended {
                run: q.run.id,
                outcome: Outcome::Stopped,
            });
        }
        for run in self.active.drain(..) {
            run.cancel.cancel();
            self.events.send(Event::Ended {
                run: run.id,
                outcome: Outcome::Stopped,
            });
        }
        if had_waiting || had_queued || had_active {
            self.sync_needed();
            self.emit_queue_updated();
            self.rest();
        }
    }

    fn cancel_project(&mut self, project: ProjectId) {
        let mut changed = false;
        let mut orphan_downloads = Vec::new();
        let mut i = 0;
        while i < self.waiting_downloads.len() {
            if self.waiting_downloads[i].run.project == project {
                let waiting = self.waiting_downloads.remove(i);
                waiting.run.cancel.cancel();
                self.emit(Event::Ended {
                    run: waiting.run.id,
                    outcome: Outcome::Stopped,
                });
                orphan_downloads.extend(waiting.pending.into_keys());
                changed = true;
            } else {
                i += 1;
            }
        }
        for dl_id in orphan_downloads {
            if !self.is_download_referenced(dl_id) {
                self.downloads.cancel_download(dl_id);
            }
        }

        let mut i = 0;
        while i < self.queued.len() {
            if self.queued[i].run.project == project {
                let q = self.queued.remove(i);
                q.run.cancel.cancel();
                self.emit(Event::Ended {
                    run: q.run.id,
                    outcome: Outcome::Stopped,
                });
                changed = true;
            } else {
                i += 1;
            }
        }

        let mut i = 0;
        while i < self.active.len() {
            if self.active[i].project == project {
                let run = self.active.remove(i);
                run.cancel.cancel();
                self.emit(Event::Ended {
                    run: run.id,
                    outcome: Outcome::Stopped,
                });
                changed = true;
            } else {
                i += 1;
            }
        }

        if changed {
            self.sync_needed();
            self.promote_next_batch();
            self.emit_queue_updated();
            self.rest();
        }
    }

    fn done(
        &mut self,
        joined: Result<(task::Id, anyhow::Result<Vec<(NodeIndex, String)>>), JoinError>,
    ) {
        let (id, result) = joined.unwrap_or_else(|error| (error.id(), Err(error.into())));
        let task = self
            .running
            .remove(&id)
            .expect("every task is recorded when it starts");
        let active_before = self.active.len();
        match result {
            Ok(failures) => {
                let per_job: HashMap<NodeIndex, String> = failures.into_iter().collect();
                self.leave(&task, &per_job);
            }
            Err(error) => {
                let reason = format!("{} failed: {error:#}", task.stage);
                match error.is::<Failed>() {
                    true => {
                        let per_job: HashMap<NodeIndex, String> = task
                            .jobs
                            .iter()
                            .map(|&(_, node)| (node, reason.clone()))
                            .collect();
                        self.leave(&task, &per_job);
                    }
                    false => self.fail(&task, &reason),
                }
            }
        }
        if self.active.len() < active_before {
            self.promote_next_batch();
            self.emit_queue_updated();
            self.rest();
        }
    }

    /// `task`'s jobs leave their runs, finished or failed per job, and a run
    /// ends with its last.
    fn leave(&mut self, task: &Running, failures: &HashMap<NodeIndex, String>) {
        for mut run in std::mem::take(&mut self.active) {
            for &(id, job) in &task.jobs {
                if id == run.id
                    && let Some(Job {
                        page,
                        stage,
                        cancelled,
                        ..
                    }) = run.remove_job_preserving_chain(job)
                    && !cancelled
                {
                    run.finished_steps += 1;
                    let run = id;
                    self.emit(match failures.get(&job) {
                        None => Event::Finished { run, page, stage },
                        Some(reason) => Event::Failed {
                            run,
                            page,
                            stage,
                            reason: reason.clone(),
                        },
                    });
                }
            }
            if run.graph.node_count() == 0 {
                let outcome = if run.finished_steps > 0 {
                    Outcome::Done
                } else {
                    Outcome::Stopped
                };
                self.emit(Event::Ended {
                    run: run.id,
                    outcome,
                });
            } else {
                self.active.push(run);
            }
        }
        self.sync_needed();
    }

    /// The runs `task` worked for end, failed for `reason`.
    fn fail(&mut self, task: &Running, reason: &str) {
        for run in std::mem::take(&mut self.active) {
            if task.jobs.iter().any(|&(id, _)| id == run.id) {
                run.cancel.cancel();
                self.emit(Event::Ended {
                    run: run.id,
                    outcome: Outcome::Failed {
                        page: task.page,
                        stage: task.stage,
                        reason: reason.to_owned(),
                    },
                });
            } else {
                self.active.push(run);
            }
        }
        self.sync_needed();
    }

    fn sync_needed(&self) {
        let map: HashMap<RunId, HashSet<Key>> = self
            .active
            .iter()
            .map(|run| {
                let keys = run
                    .graph
                    .node_weights()
                    .filter_map(|job| stages::key(job.stage, &run.config))
                    .collect();
                (run.id, keys)
            })
            .collect();
        *self.needed_by_run.write() = map;
    }

    /// A run ended: the models no setting uses leave `KEEP` from now, and
    /// libtorch's cache now, if no active run is left.
    fn rest(&mut self) {
        self.idle = Some(Instant::now() + KEEP);
        if self.active.is_empty() {
            self.gpu.empty_cache();
        }
    }

    /// Unloads the models neither the settings nor any active/queued/waiting
    /// run uses, once the GPU's current call ends.
    fn settle(&self) {
        let mut wanted: HashSet<Key> = stages::chosen(&self.config.read()).collect();
        for run in self
            .active
            .iter()
            .chain(self.queued.iter().map(|q| &q.run))
            .chain(self.waiting_downloads.iter().map(|w| &w.run))
        {
            wanted.extend(stages::chosen(&run.config));
        }
        self.gpu
            .run(Turn::Upkeep, move |models| models.retain(&wanted));
    }

    /// The ready jobs not yet started. Within each run, jobs are admitted one
    /// stage at a time (stage-batched across all pages in `Detect`, `Recognize`,
    /// `Inpaint`, `Translate` order) so models load once per stage and OCR
    /// batches fill cleanly. A project's translation isn't ready while another
    /// holds its conversation.
    fn ready(&self) -> Vec<Candidate> {
        self.active
            .iter()
            .enumerate()
            .flat_map(|(at, run)| {
                let min_stage = run.graph.node_weights().map(|job| job.stage).min();
                run.graph.externals(Incoming).filter_map(move |job| {
                    let node = &run.graph[job];
                    if Some(node.stage) != min_stage {
                        return None;
                    }
                    let held =
                        node.stage == Stage::Translate && self.conversations.busy(run.project);
                    (!node.running && !node.cancelled && !held).then_some(Candidate {
                        run: at,
                        job,
                        stage: node.stage,
                        priority: Priority {
                            mode: run.mode,
                            run: run.id,
                            stage: node.stage,
                            page: node.index,
                        },
                    })
                })
            })
            .collect()
    }

    fn collect_translate_batch(&self, first: Candidate) -> Vec<Candidate> {
        let mut chosen = vec![first];
        if first.stage != Stage::Translate {
            return chosen;
        }
        let run = &self.active[first.run];
        if !run.config.translator.combine_pages
            || run.config.translator.vision
            || run.graph[first.job].page.is_none()
        {
            return chosen;
        }
        let cap = run
            .config
            .translator
            .max_pages_per_request
            .map_or(AUTO_TRANSLATE_CHUNK, |n| n as usize)
            .clamp(1, MAX_TRANSLATE_CHUNK);
        let mut cursor = first.job;
        while chosen.len() < cap {
            let Some(next) = run.graph.neighbors_directed(cursor, Outgoing).find(|&n| {
                let node = &run.graph[n];
                node.stage == Stage::Translate
                    && node.page.is_some()
                    && !node.running
                    && !node.cancelled
            }) else {
                break;
            };
            let prereqs_done = run
                .graph
                .neighbors_directed(next, Incoming)
                .all(|pred| pred == cursor);
            if !prereqs_done {
                break;
            }
            let node = &run.graph[next];
            chosen.push(Candidate {
                run: first.run,
                job: next,
                stage: Stage::Translate,
                priority: Priority {
                    mode: run.mode,
                    run: run.id,
                    stage: Stage::Translate,
                    page: node.index,
                },
            });
            cursor = next;
        }
        chosen
    }

    fn admit(&mut self) {
        loop {
            let ready = self.ready();
            // A stage with no model of Koharu's to load, a hosted
            // translation, doesn't wait for the GPU, so it isn't counted
            // among the pages in flight.
            let hosted = ready
                .iter()
                .filter(|candidate| {
                    stages::key(candidate.stage, &self.active[candidate.run].config).is_none()
                })
                .min_by_key(|candidate| candidate.priority);
            if let Some(&hosted) = hosted {
                let chosen = self.collect_translate_batch(hosted);
                self.start(chosen, None);
                continue;
            }
            let Some(&first) = ready.iter().min_by_key(|candidate| candidate.priority) else {
                return;
            };
            let Ok(permit) = Arc::clone(&self.ahead).try_acquire_owned() else {
                return;
            };
            let mut chosen = self.collect_translate_batch(first);
            if first.stage == Stage::Recognize {
                let model = stages::key(first.stage, &self.active[first.run].config);
                let mut others: Vec<Candidate> = ready
                    .iter()
                    .filter(|other| {
                        other.stage == Stage::Recognize
                            && other.priority != first.priority
                            && stages::key(other.stage, &self.active[other.run].config) == model
                    })
                    .copied()
                    .collect();
                others.sort_by_key(|other| other.priority);
                chosen.extend(others.into_iter().take(RECOGNIZE_PAGES - 1));
            }
            self.start(chosen, Some(permit));
        }
    }

    fn start(&mut self, chosen: Vec<Candidate>, permit: Option<OwnedSemaphorePermit>) {
        let members: Vec<Member> = chosen
            .iter()
            .map(|candidate| {
                let run = &mut self.active[candidate.run];
                let job = &mut run.graph[candidate.job];
                job.running = true;
                Member {
                    run: run.id,
                    job: candidate.job,
                    page: job.page,
                    index: job.index,
                    blocks: job.blocks.clone(),
                    session: Arc::clone(&run.session),
                    cancel: job.cancel.clone(),
                    started: Default::default(),
                }
            })
            .collect();
        let first = chosen[0];
        let run = &self.active[first.run];
        let model = stages::model_name(first.stage, &run.config);
        let inputs = Inputs {
            config: Arc::clone(&run.config),
            area: run.area.clone(),
            conversation: (first.stage == Stage::Translate).then(|| {
                self.conversations
                    .lock(run.project)
                    .expect("a translation is ready only while its conversation is free")
            }),
        };
        let running = Running {
            stage: first.stage,
            jobs: members
                .iter()
                .map(|member| (member.run, member.job))
                .collect(),
            page: members[0].page,
        };
        let task = Task {
            stage: first.stage,
            model,
            members,
            priority: first.priority,
            gpu: self.gpu.clone(),
            events: self.events.clone(),
        };
        let handle = self.tasks.spawn(async move {
            // The page leaves the flight when its task ends, however it ends.
            let _permit = permit;
            stages::run(task, inputs).await
        });
        self.running.insert(handle.id(), running);
    }
}

fn run_work_items(run: &Run) -> HashSet<(Option<PageId>, Stage)> {
    run.graph
        .node_weights()
        .map(|job| (job.page, job.stage))
        .collect()
}

fn stages_requiring_models(run: &Run) -> BTreeSet<Stage> {
    let session = run.session.lock();
    let project = session.project();
    let mut page_ids = HashSet::new();
    let mut has_metadata = false;
    for job in run.graph.node_weights() {
        if let Some(page) = job.page {
            page_ids.insert(page);
        } else {
            has_metadata = true;
        }
    }
    let any_scan = page_ids
        .iter()
        .any(|&id| project.page(id).ok().and_then(|p| p.background).is_some());
    let any_blocks = page_ids
        .iter()
        .any(|&id| !project.blocks(id).unwrap_or_default().is_empty());
    let has_detect = run.graph.node_weights().any(|j| j.stage == Stage::Detect);
    let mut active_stages = BTreeSet::new();
    for job in run.graph.node_weights() {
        let needed = match job.stage {
            Stage::Detect => any_scan,
            Stage::Recognize => any_scan && (any_blocks || has_detect),
            Stage::Inpaint => any_scan && (run.area.is_some() || any_blocks || has_detect),
            Stage::Translate => has_metadata || any_blocks || (any_scan && has_detect),
        };
        if needed {
            active_stages.insert(job.stage);
        }
    }
    active_stages
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use koharu_scene::{Edit, Session};
    use parking_lot::Mutex;
    use tokio::sync::oneshot;

    use super::*;
    use crate::{RunPriority, Target};

    /// Fan-out mock download manager matching `DownloadManager` deduplication
    /// by `model_id` across multiple `start` calls.
    struct SharedMockDownloads {
        next_id: AtomicU64,
        ids: parking_lot::Mutex<HashMap<String, u64>>,
        senders: parking_lot::Mutex<HashMap<u64, Vec<oneshot::Sender<anyhow::Result<()>>>>>,
        cancelled: parking_lot::Mutex<Vec<u64>>,
    }

    impl SharedMockDownloads {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                next_id: AtomicU64::new(1),
                ids: parking_lot::Mutex::new(HashMap::new()),
                senders: parking_lot::Mutex::new(HashMap::new()),
                cancelled: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn resolve(&self, id: u64, make_res: impl Fn() -> anyhow::Result<()>) {
            if let Some(list) = self.senders.lock().remove(&id) {
                for tx in list {
                    let _ = tx.send(make_res());
                }
            }
        }

        fn cancelled_ids(&self) -> Vec<u64> {
            self.cancelled.lock().clone()
        }
    }

    impl DownloadOps for SharedMockDownloads {
        fn start(
            &self,
            model_id: &str,
            _model_name: &str,
            _stage: Stage,
            _files: &[PinnedFile],
        ) -> DownloadWaiter {
            let mut ids = self.ids.lock();
            let mut senders = self.senders.lock();
            let id = if let Some(&existing) = ids.get(model_id)
                && senders.contains_key(&existing)
            {
                existing
            } else {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                ids.insert(model_id.to_owned(), id);
                id
            };
            let (tx, rx) = oneshot::channel();
            senders.entry(id).or_default().push(tx);
            DownloadWaiter {
                id: Some(id),
                wait: Box::pin(async move { rx.await.map_err(|_| anyhow::anyhow!("dropped"))? }),
            }
        }

        fn cancel_download(&self, id: u64) {
            self.cancelled.lock().push(id);
            self.resolve(id, || Err(koharu_runtime::Cancelled.into()));
        }
    }

    fn make_project(
        num_pages: usize,
    ) -> anyhow::Result<(tempfile::TempDir, Arc<Mutex<Session>>, Vec<PageId>)> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        let hash = koharu_scene::Hash::of(&png);
        let mut ops: Vec<koharu_scene::Op> = Vec::new();
        if num_pages > 0 {
            ops.push(koharu_scene::Op::AddFile { bytes: png });
        }
        for index in 0..num_pages {
            ops.push(koharu_scene::Op::AddPage {
                id: None,
                index,
                size: None,
                background: Some(hash.clone()),
            });
        }
        if !ops.is_empty() {
            session.apply(Edit {
                label: "test".to_owned(),
                ops,
            })?;
        }
        let pages = session.project().pages().map(|p| p.id).collect();
        Ok((dir, Arc::new(Mutex::new(session)), pages))
    }

    fn make_run(
        id: RunId,
        session: &Arc<Mutex<Session>>,
        target: Target,
        stages: BTreeSet<Stage>,
        config: PipelineConfig,
        priority: RunPriority,
    ) -> Run {
        let project = session.lock().project();
        let plan = Plan::new(&project, &target, &stages, priority).unwrap();
        let scope = QueueScope::from_target(&target);
        Run::new(
            id,
            Arc::clone(session),
            project.id(),
            config,
            plan,
            &stages,
            priority,
            scope,
        )
    }

    fn test_scheduler(
        downloads: Arc<dyn DownloadOps>,
        files_needed_fn: fn(&BTreeSet<Stage>, &PipelineConfig) -> anyhow::Result<Vec<NeededModel>>,
    ) -> (Scheduler, Events, GpuSnapshot) {
        let config = Arc::new(RwLock::new(PipelineConfig::default()));
        let events = Events::new();
        let needed_by_run = Arc::new(RwLock::new(HashMap::new()));
        let (gpu, snapshot) = Gpu::spawn(Handle::current(), Arc::clone(&needed_by_run));
        let (_tx, scheduler) = Scheduler::new_with(
            config,
            events.clone(),
            gpu,
            snapshot.clone(),
            needed_by_run,
            downloads,
            files_needed_fn,
        );
        (scheduler, events, snapshot)
    }

    fn all_missing(
        stages: &BTreeSet<Stage>,
        config: &PipelineConfig,
    ) -> anyhow::Result<Vec<NeededModel>> {
        stages::files_needed_with(stages, config, |_| false)
    }

    fn all_present(
        stages: &BTreeSet<Stage>,
        config: &PipelineConfig,
    ) -> anyhow::Result<Vec<NeededModel>> {
        stages::files_needed_with(stages, config, |_| true)
    }

    #[test]
    fn interactive_runs_go_first_then_stage_batched_pages_in_order() {
        let pages = Priority {
            mode: Mode::Batch,
            run: RunId(0),
            stage: Stage::Detect,
            page: 0,
        };
        let interactive = Priority {
            mode: Mode::Interactive,
            run: RunId(1),
            stage: Stage::Translate,
            page: 3,
        };
        let later_page = Priority { page: 1, ..pages };
        let next_stage = Priority {
            stage: Stage::Recognize,
            ..pages
        };
        assert!(interactive < pages && pages < later_page && later_page < next_stage);
    }

    #[tokio::test]
    async fn stage_batched_barrier_runs_all_pages_per_stage_before_advancing() -> anyhow::Result<()>
    {
        let (_dir, session, pages) = make_project(3)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, _events, _snap) = test_scheduler(dl, all_present);

        let run = make_run(
            RunId(1),
            &session,
            Target::Project,
            BTreeSet::from([
                Stage::Detect,
                Stage::Recognize,
                Stage::Inpaint,
                Stage::Translate,
            ]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(run));

        // Phase 1: only Detect jobs are ready across all 3 pages.
        let ready_stage = |sched: &Scheduler| -> Vec<(Option<PageId>, Stage)> {
            let mut list: Vec<_> = sched
                .ready()
                .into_iter()
                .map(|c| {
                    let node = &sched.active[c.run].graph[c.job];
                    (c.priority, node.page, node.stage)
                })
                .collect();
            list.sort_by_key(|(p, ..)| *p);
            list.into_iter()
                .map(|(_, page, stage)| (page, stage))
                .collect()
        };

        assert_eq!(
            ready_stage(&sched),
            vec![
                (Some(pages[0]), Stage::Detect),
                (Some(pages[1]), Stage::Detect),
                (Some(pages[2]), Stage::Detect),
            ]
        );

        // Even after Page 0 finishes Detect, Recognize/Inpaint for Page 0 MUST wait for Pages 1 & 2 Detect.
        let p0_detect = sched.active[0]
            .graph
            .node_indices()
            .find(|&n| {
                sched.active[0].graph[n].page == Some(pages[0])
                    && sched.active[0].graph[n].stage == Stage::Detect
            })
            .unwrap();
        sched.active[0].remove_job_preserving_chain(p0_detect);
        assert_eq!(
            ready_stage(&sched),
            vec![
                (Some(pages[1]), Stage::Detect),
                (Some(pages[2]), Stage::Detect),
            ]
        );

        // Once all Detect jobs complete, ALL Recognize jobs become ready together (maximizing OCR batching).
        for page in [pages[1], pages[2]] {
            let node = sched.active[0]
                .graph
                .node_indices()
                .find(|&n| {
                    sched.active[0].graph[n].page == Some(page)
                        && sched.active[0].graph[n].stage == Stage::Detect
                })
                .unwrap();
            sched.active[0].remove_job_preserving_chain(node);
        }
        assert_eq!(
            ready_stage(&sched),
            vec![
                (Some(pages[0]), Stage::Recognize),
                (Some(pages[1]), Stage::Recognize),
                (Some(pages[2]), Stage::Recognize),
            ]
        );

        // Complete all Recognize jobs -> ALL Inpaint jobs become ready (Translate still gated on Inpaint).
        for page in &pages {
            let node = sched.active[0]
                .graph
                .node_indices()
                .find(|&n| {
                    sched.active[0].graph[n].page == Some(*page)
                        && sched.active[0].graph[n].stage == Stage::Recognize
                })
                .unwrap();
            sched.active[0].remove_job_preserving_chain(node);
        }
        assert_eq!(
            ready_stage(&sched),
            vec![
                (Some(pages[0]), Stage::Inpaint),
                (Some(pages[1]), Stage::Inpaint),
                (Some(pages[2]), Stage::Inpaint),
            ]
        );

        // Complete all Inpaint jobs -> Translate starts in page order.
        for page in &pages {
            let node = sched.active[0]
                .graph
                .node_indices()
                .find(|&n| {
                    sched.active[0].graph[n].page == Some(*page)
                        && sched.active[0].graph[n].stage == Stage::Inpaint
                })
                .unwrap();
            sched.active[0].remove_job_preserving_chain(node);
        }
        assert_eq!(
            ready_stage(&sched),
            vec![(Some(pages[0]), Stage::Translate)]
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancel_job_removes_single_step_bridges_translate_chain_and_cancels_waiting_downloads()
    -> anyhow::Result<()> {
        let (_dir, session, pages) = make_project(3)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, events, _snap) = test_scheduler(dl.clone(), all_present);
        let mut rx = events.subscribe();

        // Active run with 3 pages of Translate.
        let run = make_run(
            RunId(1),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Translate]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(run));

        // Cancel middle page's Translate job (pages[1]).
        sched.handle(Command::CancelJob {
            run: RunId(1),
            page: Some(pages[1]),
            stage: Stage::Translate,
        });
        assert_eq!(
            sched.active[0].ordered_jobs(),
            vec![
                crate::QueuedStep {
                    page: Some(pages[0]),
                    stage: Stage::Translate,
                },
                crate::QueuedStep {
                    page: Some(pages[2]),
                    stage: Stage::Translate,
                },
            ]
        );
        // Page 2 MUST still wait for Page 0 because the Translate chain edge was bridged.
        let ready_pages: Vec<Option<PageId>> = sched
            .ready()
            .into_iter()
            .map(|c| sched.active[c.run].graph[c.job].page)
            .collect();
        assert_eq!(ready_pages, vec![Some(pages[0])]);

        // Finish page 0 -> page 2 becomes ready.
        let p0_node = sched.active[0]
            .graph
            .node_indices()
            .find(|&n| sched.active[0].graph[n].page == Some(pages[0]))
            .unwrap();
        sched.active[0].remove_job_preserving_chain(p0_node);
        sched.active[0].finished_steps += 1;
        let ready_pages: Vec<Option<PageId>> = sched
            .ready()
            .into_iter()
            .map(|c| sched.active[c.run].graph[c.job].page)
            .collect();
        assert_eq!(ready_pages, vec![Some(pages[2])]);

        // Cancel remaining page 2 -> run ends as Done (since page 0 already finished).
        sched.handle(Command::CancelJob {
            run: RunId(1),
            page: Some(pages[2]),
            stage: Stage::Translate,
        });
        assert!(sched.active.is_empty());

        let mut cancelled_events = Vec::new();
        let mut ended_outcome = None;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::JobCancelled { run, page, stage } => {
                    cancelled_events.push((run, page, stage));
                }
                Event::Ended {
                    run: RunId(1),
                    outcome,
                } => ended_outcome = Some(outcome),
                _ => {}
            }
        }
        assert_eq!(
            cancelled_events,
            vec![
                (RunId(1), Some(pages[1]), Stage::Translate),
                (RunId(1), Some(pages[2]), Stage::Translate),
            ]
        );
        assert_eq!(ended_outcome, Some(Outcome::Done));

        // Cancelling the only Inpaint job of a download-waiting run cancels Inpaint's download
        // and promotes the run if no other downloads remain.
        let (mut wait_sched, _w_events, _w_snap) = test_scheduler(dl.clone(), all_missing);
        let run_wait = make_run(
            RunId(2),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Detect, Stage::Inpaint]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        wait_sched.handle(Command::Run(run_wait));
        assert_eq!(wait_sched.waiting_downloads.len(), 1);
        let inpaint_dl = *wait_sched.waiting_downloads[0]
            .pending
            .iter()
            .find(|(_, st)| **st == Stage::Inpaint)
            .unwrap()
            .0;
        wait_sched.handle(Command::CancelJob {
            run: RunId(2),
            page: Some(pages[0]),
            stage: Stage::Inpaint,
        });
        assert!(dl.cancelled_ids().contains(&inpaint_dl));
        assert_eq!(wait_sched.waiting_downloads[0].pending.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn collect_translate_batch_respects_combine_pages_vision_and_cap() -> anyhow::Result<()> {
        let (_dir, session, pages) = make_project(4)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, _events, _snap) = test_scheduler(dl, all_present);

        // combine_pages = true (default), vision = false, max_pages = Some(3) -> grabs 3 consecutive pages.
        let mut cfg = PipelineConfig::default();
        cfg.translator.combine_pages = true;
        cfg.translator.vision = false;
        cfg.translator.max_pages_per_request = Some(3);
        sched.handle(Command::Run(make_run(
            RunId(1),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Translate]),
            cfg,
            RunPriority::Normal,
        )));
        let first = sched
            .ready()
            .into_iter()
            .min_by_key(|c| c.priority)
            .unwrap();
        let batch = sched.collect_translate_batch(first);
        let batch_pages: Vec<Option<PageId>> = batch
            .iter()
            .map(|c| sched.active[c.run].graph[c.job].page)
            .collect();
        assert_eq!(
            batch_pages,
            vec![Some(pages[0]), Some(pages[1]), Some(pages[2])]
        );

        // vision = true -> always 1 page per call (current page image + marked neighbor context).
        sched.handle(Command::Cancel(None));
        let mut cfg_vision = PipelineConfig::default();
        cfg_vision.translator.combine_pages = true;
        cfg_vision.translator.vision = true;
        sched.handle(Command::Run(make_run(
            RunId(2),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Translate]),
            cfg_vision,
            RunPriority::Normal,
        )));
        let first = sched
            .ready()
            .into_iter()
            .min_by_key(|c| c.priority)
            .unwrap();
        assert_eq!(sched.collect_translate_batch(first).len(), 1);

        // combine_pages = false -> always 1 page per call.
        sched.handle(Command::Cancel(None));
        let mut cfg_off = PipelineConfig::default();
        cfg_off.translator.combine_pages = false;
        sched.handle(Command::Run(make_run(
            RunId(3),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Translate]),
            cfg_off,
            RunPriority::Normal,
        )));
        let first = sched
            .ready()
            .into_iter()
            .min_by_key(|c| c.priority)
            .unwrap();
        assert_eq!(sched.collect_translate_batch(first).len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn waiting_for_download_transitions_to_active_and_completes_or_fails()
    -> anyhow::Result<()> {
        let (_dir, session, _pages) = make_project(1)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, events, _snap) = test_scheduler(dl.clone(), all_missing);
        let mut rx = events.subscribe();

        let run_ok = make_run(
            RunId(1),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(run_ok));
        assert_eq!(sched.waiting_downloads.len(), 1);
        assert!(sched.active.is_empty());
        assert_eq!(dl.cancelled_ids(), Vec::<u64>::new());

        let ev = rx.try_recv().unwrap();
        assert_eq!(
            ev,
            Event::WaitingForDownload {
                run: RunId(1),
                downloads: vec![1],
            }
        );

        dl.resolve(1, || Ok(()));
        let joined = sched.download_tasks.join_next().await.unwrap();
        sched.download_done(joined);
        assert!(sched.waiting_downloads.is_empty());
        assert_eq!(sched.active.len(), 1);

        // Failing one download of a 2-download run fails the run and cancels the other sole-owner download.
        let run_fail = make_run(
            RunId(2),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Detect, Stage::Inpaint]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(run_fail));
        assert_eq!(sched.waiting_downloads.len(), 1);
        let pending_ids: Vec<u64> = sched.waiting_downloads[0].pending.keys().copied().collect();
        assert_eq!(pending_ids.len(), 2);

        dl.resolve(pending_ids[0], || anyhow::bail!("network timeout"));
        let joined = sched.download_tasks.join_next().await.unwrap();
        sched.download_done(joined);

        assert!(sched.waiting_downloads.is_empty());
        assert_eq!(dl.cancelled_ids(), vec![pending_ids[1]]);
        let mut found_failed = false;
        while let Ok(ev) = rx.try_recv() {
            if let Event::Ended {
                run: RunId(2),
                outcome:
                    Outcome::Failed {
                        page: None,
                        stage,
                        reason,
                    },
            } = ev
            {
                assert_eq!(stage, Stage::Detect);
                assert!(reason.contains("download failed"));
                found_failed = true;
            }
        }
        assert!(found_failed);
        Ok(())
    }

    #[tokio::test]
    async fn shared_download_cancel_preserves_other_waiting_run_until_last_cancelled()
    -> anyhow::Result<()> {
        let (_dir, session, _pages) = make_project(1)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, events, _snap) = test_scheduler(dl.clone(), all_missing);
        let mut rx = events.subscribe();

        for id in [RunId(1), RunId(2)] {
            let run = make_run(
                id,
                &session,
                Target::Project,
                BTreeSet::from([Stage::Detect]),
                PipelineConfig::default(),
                RunPriority::Normal,
            );
            sched.handle(Command::Run(run));
        }
        assert_eq!(sched.waiting_downloads.len(), 2);

        // Cancel Run 1: shared download id 1 is still owned by Run 2 -> NOT cancelled.
        sched.handle(Command::Cancel(Some(RunId(1))));
        assert_eq!(sched.waiting_downloads.len(), 1);
        assert!(dl.cancelled_ids().is_empty());

        // Cancel Run 2: now id 1 has no remaining waiters -> cancelled!
        sched.handle(Command::Cancel(Some(RunId(2))));
        assert!(sched.waiting_downloads.is_empty());
        assert_eq!(dl.cancelled_ids(), vec![1]);

        let mut stopped = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let Event::Ended {
                run,
                outcome: Outcome::Stopped,
            } = ev
            {
                stopped.push(run);
            }
        }
        assert_eq!(stopped, vec![RunId(1), RunId(2)]);
        Ok(())
    }

    #[tokio::test]
    async fn single_active_batch_queues_second_batch_and_rerun_now_preempts() -> anyhow::Result<()>
    {
        let (_dir, session, _pages) = make_project(2)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, events, _snap) = test_scheduler(dl, all_present);
        let mut rx = events.subscribe();

        let b1 = make_run(
            RunId(1),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        let mut b2_cfg = PipelineConfig::default();
        b2_cfg.translator.vision = true;
        let b2 = make_run(
            RunId(2),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Inpaint]),
            b2_cfg,
            RunPriority::Normal,
        );
        let rerun = make_run(
            RunId(3),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::RerunNow,
        );

        sched.handle(Command::Run(b1));
        assert_eq!(sched.active.len(), 1);
        assert!(sched.queued.is_empty());

        sched.handle(Command::Run(b2));
        assert_eq!(sched.active.len(), 1);
        assert_eq!(sched.queued.len(), 1);
        assert_eq!(sched.queued[0].run.id, RunId(2));

        // RerunNow bypasses `queued` and enters `active` with `Mode::Interactive`.
        sched.handle(Command::Run(rerun));
        assert_eq!(sched.active.len(), 2);
        assert_eq!(sched.active[1].id, RunId(3));
        assert_eq!(sched.active[1].mode, Mode::Interactive);
        assert_eq!(sched.queued.len(), 1);

        // Ready candidates pick RerunNow before active Batch run!
        let ready = sched.ready();
        let first = ready.iter().min_by_key(|c| c.priority).unwrap();
        assert_eq!(sched.active[first.run].id, RunId(3));

        // Cancelling active b1 promotes b2 from queued.
        sched.handle(Command::Cancel(Some(RunId(1))));
        assert_eq!(
            sched.active.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![RunId(3), RunId(2)]
        );
        assert!(sched.queued.is_empty());

        let mut latest_order = None;
        while let Ok(ev) = rx.try_recv() {
            if let Event::QueueUpdated { order } = ev {
                latest_order = Some(order);
            }
        }
        assert_eq!(latest_order, Some(Vec::new()));
        Ok(())
    }

    #[tokio::test]
    async fn coalesces_overlapping_queued_runs_without_cross_producting_stages()
    -> anyhow::Result<()> {
        let (_dir, session, pages) = make_project(3)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, events, _snap) = test_scheduler(dl, all_present);
        let mut rx = events.subscribe();

        // Keep an active batch run busy on Detect with a different config so queued runs don't promote.
        let mut active_cfg = PipelineConfig::default();
        active_cfg.detector.text = Some(0.99);
        let active_batch = make_run(
            RunId(0),
            &session,
            Target::Pages(vec![pages[2]]),
            BTreeSet::from([Stage::Detect]),
            active_cfg,
            RunPriority::Normal,
        );
        sched.handle(Command::Run(active_batch));

        // Run 1: P0 {Detect, Translate}
        let r1 = make_run(
            RunId(1),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Detect, Stage::Translate]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        // Run 2: P0, P1 {Translate} -> overlaps P0 Translate, must NOT add Detect to P1!
        let r2 = make_run(
            RunId(2),
            &session,
            Target::Pages(vec![pages[0], pages[1]]),
            BTreeSet::from([Stage::Translate]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        // Run 3: P2 {Inpaint} -> disjoint from Run 1, stays separate.
        let r3 = make_run(
            RunId(3),
            &session,
            Target::Pages(vec![pages[2]]),
            BTreeSet::from([Stage::Inpaint]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );

        sched.handle(Command::Run(r1));
        sched.handle(Command::Run(r2));
        sched.handle(Command::Run(r3));

        assert_eq!(sched.queued.len(), 2);
        let merged = &sched.queued[0].run;
        assert_eq!(merged.id, RunId(1));
        assert_eq!(merged.scope, QueueScope::Pages { count: 2 });
        let items: BTreeSet<(Option<PageId>, Stage)> = merged
            .graph
            .node_weights()
            .map(|j| (j.page, j.stage))
            .collect();
        assert_eq!(
            items,
            BTreeSet::from([
                (Some(pages[0]), Stage::Detect),
                (Some(pages[0]), Stage::Translate),
                (Some(pages[1]), Stage::Translate),
            ])
        );
        assert!(!items.contains(&(Some(pages[1]), Stage::Detect)));

        let mut coalesced = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let Event::Coalesced { run, into } = ev {
                coalesced.push((run, into));
            }
        }
        assert_eq!(coalesced, vec![(RunId(2), RunId(1))]);
        Ok(())
    }

    #[tokio::test]
    async fn memory_aware_scoring_groups_resident_prefers_lower_vram_and_ages_with_fifo()
    -> anyhow::Result<()> {
        tokio::time::pause();
        let (_dir, session, pages) = make_project(3)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, _events, snap) = test_scheduler(dl, all_present);

        // Keep an active batch run busy.
        let active = make_run(
            RunId(0),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(active));

        // Configure resident model = Flux3Klein (`inpaint:flux3-klein`).
        let k_flux = stages::key(Stage::Inpaint, &{
            let mut cfg = PipelineConfig::default();
            cfg.inpainter.model = crate::Inpainter::Flux3Klein;
            cfg
        })
        .unwrap();
        let k_qwen = stages::key(Stage::Inpaint, &{
            let mut cfg = PipelineConfig::default();
            cfg.inpainter.model = crate::Inpainter::QwenImage;
            cfg
        })
        .unwrap();
        let k_lama = stages::key(Stage::Inpaint, &PipelineConfig::default()).unwrap();

        let gib = 1024 * 1024 * 1024;
        let mib = 1024 * 1024;
        let device_key = "cuda:CUDA0:Mock".to_owned();
        snap.set_device_info(Some(device_key.clone()), Some(1024 * mib));
        snap.set_resident(HashSet::from([k_flux.clone()]));
        for (k, bytes) in [(&k_flux, 6 * gib), (&k_qwen, 8 * gib), (&k_lama, 200 * mib)] {
            let files = stages::files_for_key(k)?;
            snap.vram().record_load(
                crate::vram::FootprintKey {
                    device: device_key.clone(),
                    model_key: crate::vram::model_key(k.stage, &k.id, k.vision),
                    revision: crate::vram::revision_for_files(&files),
                },
                bytes,
                false,
                None,
            );
        }

        let mut cfg_qwen = PipelineConfig::default();
        cfg_qwen.inpainter.model = crate::Inpainter::QwenImage;
        let mut cfg_lama = PipelineConfig::default();
        cfg_lama.inpainter.model = crate::Inpainter::Lama;
        let mut cfg_flux = PipelineConfig::default();
        cfg_flux.inpainter.model = crate::Inpainter::Flux3Klein;

        // Enqueue Qwen (heavy 8 GiB + 6 GiB eviction = 14 GiB), then LaMa (200 MiB + 6 GiB eviction),
        // then Flux (0 load, already resident!).
        sched.handle(Command::Run(make_run(
            RunId(1),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_qwen,
            RunPriority::Normal,
        )));
        sched.handle(Command::Run(make_run(
            RunId(2),
            &session,
            Target::Pages(vec![pages[1]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_lama,
            RunPriority::Normal,
        )));
        sched.handle(Command::Run(make_run(
            RunId(3),
            &session,
            Target::Pages(vec![pages[2]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_flux,
            RunPriority::Normal,
        )));

        let order: Vec<(RunId, QueueOrderReason)> = sched
            .score_queued()
            .into_iter()
            .map(|(i, r)| (sched.queued[i].run.id, r))
            .collect();
        assert_eq!(order[0], (RunId(3), QueueOrderReason::GroupedByLoadedModel));
        assert_eq!(order[1], (RunId(2), QueueOrderReason::LowerVramCost));
        assert_eq!(order[2], (RunId(1), QueueOrderReason::Fifo));

        // Small elapsed time (< starvation threshold) makes aging_bonus > 0, yet LowerVramCost
        // and Fifo must still be reported rather than falsely flagging AgedPriority.
        tokio::time::advance(Duration::from_millis(10)).await;
        let brief_wait_order: Vec<(RunId, QueueOrderReason)> = sched
            .score_queued()
            .into_iter()
            .map(|(i, r)| (sched.queued[i].run.id, r))
            .collect();
        assert_eq!(
            brief_wait_order,
            vec![
                (RunId(3), QueueOrderReason::GroupedByLoadedModel),
                (RunId(2), QueueOrderReason::LowerVramCost),
                (RunId(1), QueueOrderReason::Fifo),
            ]
        );

        // Advance virtual time by 60 seconds: 60 * 256 MiB = 15 GiB > 14 GiB!
        // Run 1 now ages to effective_cost = 0, tying with Run 3 and beating it by FIFO RunId!
        tokio::time::advance(Duration::from_secs(60)).await;
        let aged_order: Vec<(RunId, QueueOrderReason)> = sched
            .score_queued()
            .into_iter()
            .map(|(i, r)| (sched.queued[i].run.id, r))
            .collect();
        assert_eq!(aged_order[0], (RunId(1), QueueOrderReason::AgedPriority));
        assert_eq!(aged_order[1], (RunId(2), QueueOrderReason::AgedPriority));
        assert_eq!(
            aged_order[2],
            (RunId(3), QueueOrderReason::GroupedByLoadedModel)
        );
        Ok(())
    }

    #[tokio::test]
    async fn unpaused_scoring_prefers_lower_vram_fallback_and_keeps_fifo_without_need_for_aging()
    -> anyhow::Result<()> {
        let (_dir, session, pages) = make_project(3)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, events, _snap) = test_scheduler(dl, all_present);
        let mut rx = events.subscribe();

        let active = make_run(
            RunId(0),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(active));

        // No recorded VRAM footprints and no paused clock: fallback uses PinnedFile::bytes
        // (Flux3Klein >> LaMa) and real wall-clock nanoseconds elapse between enqueue and score.
        let mut cfg_flux = PipelineConfig::default();
        cfg_flux.inpainter.model = crate::Inpainter::Flux3Klein;
        let mut cfg_lama = PipelineConfig::default();
        cfg_lama.inpainter.model = crate::Inpainter::Lama;

        sched.handle(Command::Run(make_run(
            RunId(1),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_flux,
            RunPriority::Normal,
        )));
        tokio::time::sleep(Duration::from_millis(5)).await;
        sched.handle(Command::Run(make_run(
            RunId(2),
            &session,
            Target::Pages(vec![pages[1]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_lama,
            RunPriority::Normal,
        )));

        let mut latest_order = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let Event::QueueUpdated { order } = ev {
                latest_order = order;
            }
        }
        assert_eq!(
            latest_order
                .iter()
                .map(|entry| (entry.run, entry.reason))
                .collect::<Vec<_>>(),
            vec![
                (RunId(2), Some(QueueOrderReason::LowerVramCost)),
                (RunId(1), Some(QueueOrderReason::Fifo)),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn aged_run_already_cheapest_keeps_fifo_reason() -> anyhow::Result<()> {
        tokio::time::pause();
        let (_dir, session, pages) = make_project(2)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, _events, _snap) = test_scheduler(dl, all_present);

        let active = make_run(
            RunId(0),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        sched.handle(Command::Run(active));

        let mut cfg_lama = PipelineConfig::default();
        cfg_lama.inpainter.model = crate::Inpainter::Lama;
        let mut cfg_flux = PipelineConfig::default();
        cfg_flux.inpainter.model = crate::Inpainter::Flux3Klein;

        sched.handle(Command::Run(make_run(
            RunId(1),
            &session,
            Target::Pages(vec![pages[0]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_lama,
            RunPriority::Normal,
        )));
        sched.handle(Command::Run(make_run(
            RunId(2),
            &session,
            Target::Pages(vec![pages[1]]),
            BTreeSet::from([Stage::Inpaint]),
            cfg_flux,
            RunPriority::Normal,
        )));

        // Advance 10s so LaMa ages to effective_cost == 0 (`aged == true`), yet it already led
        // Flux3Klein by both FIFO order and raw VRAM cost.
        tokio::time::advance(Duration::from_secs(10)).await;
        let order: Vec<(RunId, QueueOrderReason)> = sched
            .score_queued()
            .into_iter()
            .map(|(i, r)| (sched.queued[i].run.id, r))
            .collect();
        assert_eq!(
            order,
            vec![
                (RunId(1), QueueOrderReason::Fifo),
                (RunId(2), QueueOrderReason::Fifo),
            ]
        );
        Ok(())
    }

    #[test]
    fn stages_requiring_models_skips_inpaint_area_without_scan() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("scanless.khr"), koharu_scene::Meta::default())?;
        session.apply(Edit {
            label: "blank page".to_owned(),
            ops: vec![koharu_scene::Op::AddPage {
                id: None,
                index: 0,
                size: Some(koharu_scene::Size {
                    width: 16,
                    height: 16,
                }),
                background: None,
            }],
        })?;
        let page = session.project().pages().next().unwrap().id;
        let session = Arc::new(Mutex::new(session));
        let run = make_run(
            RunId(1),
            &session,
            Target::Area {
                page,
                area: crate::Area {
                    rect: koharu_scene::Rect {
                        x: 0,
                        y: 0,
                        width: 4,
                        height: 4,
                    },
                    pixels: vec![255; 16],
                },
                prompt: None,
                layer: None,
            },
            BTreeSet::from([Stage::Inpaint]),
            PipelineConfig::default(),
            RunPriority::RerunNow,
        );
        assert!(stages_requiring_models(&run).is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn cancel_none_and_close_project_drain_all_queues() -> anyhow::Result<()> {
        let (_d1, s1, _) = make_project(1)?;
        let (_d2, s2, _) = make_project(1)?;
        let dl = SharedMockDownloads::new();
        let (mut sched, _events, _snap) = test_scheduler(dl.clone(), all_present);

        // Put active + queued on project 1, active/interactive on project 2, and a waiting run on project 1.
        sched.handle(Command::Run(make_run(
            RunId(1),
            &s1,
            Target::Project,
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        )));
        sched.handle(Command::Run(make_run(
            RunId(2),
            &s1,
            Target::Project,
            BTreeSet::from([Stage::Inpaint]),
            PipelineConfig::default(),
            RunPriority::Normal,
        )));
        sched.handle(Command::Run(make_run(
            RunId(3),
            &s2,
            Target::Project,
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::RerunNow,
        )));

        sched.handle(Command::Close(s1.lock().project().id()));
        assert_eq!(
            sched.active.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![RunId(3)]
        );
        assert!(sched.queued.is_empty());

        sched.handle(Command::Cancel(None));
        assert!(sched.active.is_empty());
        assert!(sched.queued.is_empty());
        assert!(sched.waiting_downloads.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn pipeline_run_reports_accelerator_lock_contention_and_unload_releases()
    -> anyhow::Result<()> {
        let (_dir, session, _pages) = make_project(1)?;
        let lock_dir = tempfile::tempdir()?;
        let external = koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())?;

        let config = Arc::new(RwLock::new(PipelineConfig::default()));
        let events = Events::new();
        let needed_by_run = Arc::new(RwLock::new(HashMap::new()));
        let (gpu, snapshot) = Gpu::spawn_with(
            Handle::current(),
            Arc::clone(&needed_by_run),
            LockMode::AcquireIn(Some(lock_dir.path().to_path_buf())),
        );
        let dl = SharedMockDownloads::new();
        let (tx, scheduler) = Scheduler::new_with(
            config.clone(),
            events.clone(),
            gpu,
            snapshot,
            needed_by_run,
            dl,
            all_present,
        );
        tokio::spawn(scheduler.run());

        let mut rx = events.subscribe();
        let run = make_run(
            RunId(1),
            &session,
            Target::Project,
            BTreeSet::from([Stage::Detect]),
            PipelineConfig::default(),
            RunPriority::Normal,
        );
        tx.send(Command::Run(run))?;

        let ended = loop {
            if let Event::Ended {
                run: RunId(1),
                outcome,
            } = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await??
            {
                break outcome;
            }
        };
        match ended {
            Outcome::Failed { stage, reason, .. } => {
                assert_eq!(stage, Stage::Detect);
                assert!(
                    reason
                        .contains("Another Koharu process is currently using the GPU accelerator"),
                    "unexpected failure reason: {reason}"
                );
            }
            other => panic!("expected Outcome::Failed, got {other:?}"),
        }

        drop(external);
        tx.send(Command::Unload)?;
        drop(tx);
        // Verify lock remains free after scheduler shutdown.
        tokio::time::sleep(Duration::from_millis(50)).await;
        koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())?;
        Ok(())
    }
}
