//! The thread that owns the models and runs one call at a time, as
//! koharu-llm's engine thread owns its model, so a model is never shared and
//! needs no lock. Calls wait in a priority queue, so an interactive run goes
//! ahead at the next call.
//!
//! Each model's VRAM footprint (`load_bytes`, `peak_working_bytes`,
//! `inpaint_bytes_per_mp`) is measured on devices with reliable VRAM accounting
//! (`CUDA` / `ROCm` discrete GPUs) and persisted to `~/.koharu/vram.json`.
//! Before loading or running a model, `Models::prepare` evicts resident models
//! when needed to stay above `vram::SAFETY_MARGIN`, and reactive OOM retry
//! evicts all other models and retries once on unexpected memory pressure.

use std::{
    any::Any,
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
        mpsc,
    },
    time::Duration,
};

use bytesize::ByteSize;
use koharu_runtime::Device;
use koharu_torch::Cuda;
use parking_lot::RwLock;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result, RunId, Stage,
    scheduler::Priority,
    stages,
    vram::{self, FootprintKey, VramStore},
};

const SD_DIFFUSION_IDS: &[&str] = &["flux3-klein", "qwen-image"];
const SD_FREE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const SD_SPILL_GUARD_BYTES: u64 = 64 * 1024 * 1024;

/// Work for the GPU thread.
pub(crate) type Work = Box<dyn FnOnce(&mut Models) + Send>;

/// A call's place in line, the smallest first: model upkeep goes ahead of
/// every stage's call.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Turn {
    Upkeep,
    Stage(Priority),
}

/// Non-blocking snapshot of GPU model residency and VRAM state for the
/// scheduler's queue scoring, updated by the GPU thread.
#[derive(Clone)]
pub(crate) struct GpuSnapshot {
    inner: Arc<GpuSnapshotInner>,
}

struct GpuSnapshotInner {
    resident: RwLock<HashSet<Key>>,
    device_key: RwLock<Option<String>>,
    free_vram: RwLock<Option<u64>>,
    vram: VramStore,
}

impl GpuSnapshot {
    pub(crate) fn new(vram: VramStore) -> Self {
        Self {
            inner: Arc::new(GpuSnapshotInner {
                resident: RwLock::new(HashSet::new()),
                device_key: RwLock::new(None),
                free_vram: RwLock::new(None),
                vram,
            }),
        }
    }

    pub(crate) fn snapshot(&self) -> (HashSet<Key>, Option<String>, Option<u64>) {
        (
            self.inner.resident.read().clone(),
            self.inner.device_key.read().clone(),
            *self.inner.free_vram.read(),
        )
    }

    pub(crate) fn vram(&self) -> &VramStore {
        &self.inner.vram
    }

    #[cfg(test)]
    pub(crate) fn set_resident(&self, resident: HashSet<Key>) {
        *self.inner.resident.write() = resident;
    }

    #[cfg(test)]
    pub(crate) fn set_device_info(&self, device_key: Option<String>, free_vram: Option<u64>) {
        *self.inner.device_key.write() = device_key;
        *self.inner.free_vram.write() = free_vram;
    }
}

#[derive(Clone)]
pub(crate) enum LockMode {
    AcquireIn(Option<PathBuf>),
    Preacquired(Arc<koharu_runtime::AcceleratorLock>),
}

/// The way to the GPU thread. It ends, unloading the models, once every
/// handle drops and its queue is empty.
#[derive(Clone)]
pub(crate) struct Gpu(mpsc::Sender<(Turn, Work)>);

impl Gpu {
    #[cfg(test)]
    pub(crate) fn spawn(
        runtime: Handle,
        needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
    ) -> (Self, GpuSnapshot) {
        Self::spawn_with(runtime, needed_by_run, LockMode::AcquireIn(None))
    }

    pub(crate) fn spawn_with(
        runtime: Handle,
        needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
        lock_mode: LockMode,
    ) -> (Self, GpuSnapshot) {
        let (sender, receiver) = mpsc::channel::<(Turn, Work)>();
        // Cancelled when the runtime shuts down, which drops its tasks before
        // it stops its timers.
        let shutdown = CancellationToken::new();
        let guard = shutdown.clone().drop_guard();
        runtime.spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let vram = VramStore::open_default().unwrap_or_else(|error| {
            tracing::warn!("ignoring unreadable VRAM cache: {error:#}");
            VramStore::in_memory()
        });
        let snapshot = GpuSnapshot::new(vram.clone());
        let models_snapshot = snapshot.clone();
        std::thread::Builder::new()
            .name("koharu-pipeline-gpu".to_owned())
            .spawn(move || {
                let mut models = Models {
                    runtime,
                    persist_async: true,
                    shutdown,
                    loaded: HashMap::new(),
                    tick: 0,
                    device: None,
                    runtime_device: None,
                    vram,
                    snapshot: models_snapshot,
                    needed_by_run,
                    lock_mode,
                    owned_lock: None,
                };
                let mut queue = BinaryHeap::new();
                let mut sent = 0;
                loop {
                    // Waits only when nothing is queued.
                    let first = match queue.is_empty() {
                        true => match receiver.recv() {
                            Ok(call) => Some(call),
                            Err(_) => return,
                        },
                        false => None,
                    };
                    for (turn, work) in first.into_iter().chain(receiver.try_iter()) {
                        queue.push(Call { turn, sent, work });
                        sent += 1;
                    }
                    if let Some(call) = queue.pop() {
                        (call.work)(&mut models);
                        models.release_lock_if_unloaded();
                    }
                }
            })
            .expect("the OS starts a thread");
        (Self(sender), snapshot)
    }

    /// Queues `work` at `turn`.
    pub(crate) fn run(&self, turn: Turn, work: impl FnOnce(&mut Models) + Send + 'static) {
        // The thread ends only once every handle has dropped.
        let _ = self.0.send((turn, Box::new(work)));
    }

    /// Gives back to the device what libtorch's caching allocator keeps of
    /// the tensors calls freed, ahead of the calls queued. The cache grows
    /// page by page, as pages need different sizes, and the device counts
    /// it as used.
    pub(crate) fn empty_cache(&self) {
        self.run(Turn::Upkeep, |models| models.empty_cache());
    }
}

/// A queued call; `BinaryHeap` pops the greatest, so the order is reversed:
/// the earliest turn, then the earliest sent.
struct Call {
    turn: Turn,
    sent: u64,
    work: Work,
}

impl Ord for Call {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.turn, other.sent).cmp(&(self.turn, self.sent))
    }
}

impl PartialOrd for Call {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Call {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Call {}

/// A model as the settings name it: its stage, its display name, its canonical
/// model ID/reference, and whether vision projector weights are attached.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Key {
    pub(crate) stage: Stage,
    pub(crate) name: String,
    pub(crate) id: String,
    pub(crate) vision: bool,
}

/// Loaded model plus optional local LLM teardown handle and full-GPU indicator.
pub(crate) struct LoadOutcome<M> {
    pub(crate) model: M,
    pub(crate) teardown: Option<koharu_llm::Teardown>,
    pub(crate) full_gpu: bool,
}

struct Loaded {
    model: Box<dyn Any + Send>,
    last_used: u64,
    teardown: Option<koharu_llm::Teardown>,
}

pub(crate) trait DeviceOps: Send + Sync {
    fn has_vram_accounting(&self) -> bool;
    fn free_vram(&self) -> u64;
    fn device_key(&self) -> String;
    fn description(&self) -> &str;
    fn empty_cache(&self);
    fn set_device(&self) -> anyhow::Result<()>;
    fn reset_peak_stats(&self);
    fn cuda_stats(&self) -> Option<(u64, u64, u64)>;
}

struct RuntimeDevice(&'static Device);

impl DeviceOps for RuntimeDevice {
    fn has_vram_accounting(&self) -> bool {
        self.0.has_vram_accounting()
    }

    fn free_vram(&self) -> u64 {
        self.0.memory().free
    }

    fn device_key(&self) -> String {
        vram::device_key(self.0)
    }

    fn description(&self) -> &str {
        &self.0.description
    }

    fn empty_cache(&self) {
        if Cuda::device_count() > 0
            && let Err(error) = Cuda::empty_cache()
        {
            tracing::warn!("libtorch's cache wasn't emptied: {error}");
        }
    }

    fn set_device(&self) -> anyhow::Result<()> {
        if Cuda::device_count() > 0 {
            Cuda::set_device(self.0.index as i64)?;
        }
        Ok(())
    }

    fn reset_peak_stats(&self) {
        if Cuda::device_count() > 0 {
            let _ = Cuda::reset_peak_memory_stats(self.0.index as i64);
        }
    }

    fn cuda_stats(&self) -> Option<(u64, u64, u64)> {
        (Cuda::device_count() > 0)
            .then(|| {
                Cuda::memory(self.0.index as i64)
                    .ok()
                    .map(|mem| (mem.reserved, mem.reserved_peak, mem.ooms))
            })
            .flatten()
    }
}

/// The loaded models, which the GPU thread owns.
pub(crate) struct Models {
    runtime: Handle,
    persist_async: bool,
    shutdown: CancellationToken,
    loaded: HashMap<Key, Loaded>,
    tick: u64,
    device: Option<Arc<dyn DeviceOps>>,
    /// The runtime's device, once a model has loaded on it.
    runtime_device: Option<&'static Device>,
    vram: VramStore,
    snapshot: GpuSnapshot,
    needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
    lock_mode: LockMode,
    owned_lock: Option<koharu_runtime::AcceleratorLock>,
}

impl Drop for Models {
    fn drop(&mut self) {
        self.retain(&HashSet::new());
    }
}

impl Models {
    /// Returns the model `key` names (with local LLM teardown tracking),
    /// preparing VRAM room first and retrying once on OOM.
    pub(crate) fn get_tracked<M, F>(
        &mut self,
        key: &Key,
        runs: &[RunId],
        mut load: impl FnMut(&'static Device) -> F,
    ) -> Result<&M>
    where
        M: Send + 'static,
        F: Future<Output = anyhow::Result<LoadOutcome<M>>>,
    {
        self.ensure_accelerator_lock(key)?;
        if let Err(err) = self.ensure_device(key) {
            self.release_lock_if_unloaded();
            return Err(err);
        }
        self.prepare(key, runs, None);
        if let Err(err) =
            self.ensure_loaded_raw(key, |dev| load(dev.expect("runtime device is initialized")))
        {
            self.release_lock_if_unloaded();
            return Err(err);
        }
        if let Err(err) = self.set_current_device(key) {
            self.release_lock_if_unloaded();
            return Err(err);
        }
        Ok(self.loaded[key]
            .model
            .downcast_ref()
            .expect("a key loads one kind of model"))
    }

    /// Ensures `key` is loaded (after planned eviction), runs `infer`, measures
    /// working VRAM delta, and retries once after evicting all other models if
    /// inference fails with an OOM error.
    pub(crate) fn run<M, F, T>(
        &mut self,
        key: &Key,
        runs: &[RunId],
        crop_mp: Option<f64>,
        mut load: impl FnMut(&'static Device) -> F,
        infer: impl FnMut(&M) -> anyhow::Result<T>,
    ) -> anyhow::Result<T>
    where
        M: Send + 'static,
        F: Future<Output = anyhow::Result<LoadOutcome<M>>>,
    {
        self.ensure_accelerator_lock(key)?;
        if let Err(err) = self.ensure_device(key) {
            self.release_lock_if_unloaded();
            return Err(err.into());
        }
        self.prepare(key, runs, crop_mp);
        if let Err(err) =
            self.ensure_loaded_raw(key, |dev| load(dev.expect("runtime device is initialized")))
        {
            self.release_lock_if_unloaded();
            return Err(err.into());
        }
        self.run_loaded(key, crop_mp, infer)
    }

    fn run_loaded<M, T>(
        &mut self,
        key: &Key,
        crop_mp: Option<f64>,
        mut infer: impl FnMut(&M) -> anyhow::Result<T>,
    ) -> anyhow::Result<T>
    where
        M: Send + 'static,
    {
        self.set_current_device(key)?;
        let device = Arc::clone(self.device.as_ref().expect("device is initialized"));
        let accounting = device.has_vram_accounting();
        let mut retried = false;

        loop {
            if accounting {
                device.empty_cache();
                device.reset_peak_stats();
            }
            let free_before = accounting.then(|| device.free_vram());
            let (reserved_before, _, ooms_before) = device.cuda_stats().unwrap_or((0, 0, 0));

            let poller = (accounting
                && key.stage == Stage::Inpaint
                && SD_DIFFUSION_IDS.contains(&key.id.as_str()))
            .then(|| FreePoller::start(Arc::clone(&device), free_before.unwrap_or(u64::MAX)));

            let result = {
                let model: &M = self.loaded[key]
                    .model
                    .downcast_ref()
                    .expect("a key loads one kind of model");
                infer(model)
            };

            let min_polled = poller.map(FreePoller::stop);

            match result {
                Ok(out) => {
                    if accounting {
                        let free_after = device.free_vram();
                        let min_free = min_polled.map_or(free_after, |m| m.min(free_after));
                        let ggml_delta = free_before.unwrap_or(0).saturating_sub(min_free);
                        let reserved_peak = device.cuda_stats().map_or(0, |(_, peak, _)| peak);
                        let torch_delta = reserved_peak.saturating_sub(reserved_before);
                        let working_delta = ggml_delta.max(torch_delta);
                        if working_delta > 0
                            && let Some(fp_key) = self.footprint_key(key, &device.device_key())
                        {
                            self.vram.record_inference(
                                fp_key,
                                working_delta,
                                crop_mp,
                                self.persist_runtime(),
                            );
                        }
                    }
                    self.sync_snapshot();
                    return Ok(out);
                }
                Err(error) => {
                    if !retried
                        && self.has_other_loaded(key)
                        && is_oom(&error, device.as_ref(), ooms_before)
                    {
                        tracing::warn!(
                            model = %key.name,
                            "inference hit OOM; evicting other loaded models and retrying once"
                        );
                        self.evict_all_except(key);
                        retried = true;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    fn ensure_accelerator_lock(&mut self, key: &Key) -> Result<()> {
        if self.owned_lock.is_some() {
            return Ok(());
        }
        let acquired = match &self.lock_mode {
            LockMode::Preacquired(lock) => {
                tracing::trace!(
                    path = %lock.path().display(),
                    "using preacquired accelerator lock"
                );
                return Ok(());
            }
            LockMode::AcquireIn(None) => koharu_runtime::AcceleratorLock::acquire(),
            LockMode::AcquireIn(Some(root)) => koharu_runtime::AcceleratorLock::acquire_in(root),
        };
        match acquired {
            Ok(lock) => {
                self.owned_lock = Some(lock);
                Ok(())
            }
            Err(source) => {
                if let Some(&locked) = source.downcast_ref::<koharu_runtime::AcceleratorLocked>() {
                    Err(Error::AcceleratorLocked(locked))
                } else {
                    Err(Error::Load {
                        model: key.name.clone(),
                        device: "accelerator lock".to_owned(),
                        free: ByteSize(0),
                        reserved: ByteSize(0),
                        active: ByteSize(0),
                        source,
                    })
                }
            }
        }
    }

    fn release_lock_if_unloaded(&mut self) {
        if self.loaded.is_empty() {
            self.owned_lock = None;
        }
    }

    fn ensure_device(&mut self, key: &Key) -> Result<()> {
        if self.device.is_some() {
            return Ok(());
        }
        let shutdown = &self.shutdown;
        let device = self.runtime.block_on(async {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => Err(Error::Shutdown),
                device = koharu_runtime::device() => device.map_err(|source| Error::Load {
                    model: key.name.clone(),
                    device: "no device".to_owned(),
                    free: ByteSize(0),
                    reserved: ByteSize(0),
                    active: ByteSize(0),
                    source,
                }),
            }
        })?;
        self.runtime_device = Some(device);
        self.device = Some(Arc::new(RuntimeDevice(device)));
        self.sync_snapshot();
        Ok(())
    }

    fn set_current_device(&self, key: &Key) -> Result<()> {
        if let Some(device) = self.device.as_ref() {
            device.set_device().map_err(|source| Error::Stage {
                stage: key.stage,
                source,
            })?;
        }
        Ok(())
    }

    fn ensure_loaded_raw<M, F>(
        &mut self,
        key: &Key,
        mut load: impl FnMut(Option<&'static Device>) -> F,
    ) -> Result<()>
    where
        M: Send + 'static,
        F: Future<Output = anyhow::Result<LoadOutcome<M>>>,
    {
        if let Some(entry) = self.loaded.get_mut(key) {
            self.tick = self.tick.saturating_add(1);
            entry.last_used = self.tick;
            return Ok(());
        }

        let device = Arc::clone(self.device.as_ref().expect("device is initialized"));
        let accounting = device.has_vram_accounting();
        let mut retried = false;

        loop {
            let _span = tracing::info_span!("load", model = %key.name).entered();
            device.empty_cache();
            let ooms_before = device.cuda_stats().map_or(0, |(_, _, ooms)| ooms);
            let free_before = accounting.then(|| device.free_vram());
            let had_others = self.has_other_loaded(key);

            let shutdown = &self.shutdown;
            let runtime_device = self.runtime_device;
            let loaded = self.runtime.block_on(async {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => Err(Error::Shutdown),
                    outcome = load(runtime_device) => outcome.map_err(|source| failure(key, device.as_ref(), source)),
                }
            });

            match loaded {
                Ok(outcome) => {
                    if accounting && !outcome.full_gpu && !retried && had_others {
                        tracing::warn!(
                            model = %key.name,
                            "local LLM partially offloaded while other models are resident; evicting others and reloading once"
                        );
                        let teardown = outcome.teardown;
                        drop(outcome.model);
                        if let Some(teardown) = teardown {
                            teardown.wait();
                        }
                        self.evict_all_except(key);
                        retried = true;
                        continue;
                    }

                    if accounting && outcome.full_gpu {
                        device.empty_cache();
                        let free_after = device.free_vram();
                        let is_diffusion = key.stage == Stage::Inpaint
                            && SD_DIFFUSION_IDS.contains(&key.id.as_str());
                        let spill_suspected =
                            is_diffusion && had_others && free_after < SD_SPILL_GUARD_BYTES;
                        if !spill_suspected {
                            let delta = free_before.unwrap_or(0).saturating_sub(free_after);
                            if delta > 0
                                && let Some(fp_key) = self.footprint_key(key, &device.device_key())
                            {
                                self.vram
                                    .record_load(fp_key, delta, false, self.persist_runtime());
                            }
                        }
                    }

                    self.tick = self.tick.saturating_add(1);
                    self.loaded.insert(
                        key.clone(),
                        Loaded {
                            model: Box::new(outcome.model),
                            last_used: self.tick,
                            teardown: outcome.teardown,
                        },
                    );
                    self.sync_snapshot();
                    return Ok(());
                }
                Err(error) => {
                    if !retried
                        && had_others
                        && !matches!(error, Error::Shutdown)
                        && is_oom_pipeline(&error, device.as_ref(), ooms_before)
                    {
                        tracing::warn!(
                            model = %key.name,
                            "model load hit OOM; evicting other loaded models and retrying once"
                        );
                        self.evict_all_except(key);
                        retried = true;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    /// Planned VRAM eviction before loading or running `key`.
    /// Gated on `device.has_vram_accounting()` and existing footprint records.
    pub(crate) fn prepare(&mut self, key: &Key, runs: &[RunId], crop_mp: Option<f64>) {
        let Some(device) = self.device.as_ref().map(Arc::clone) else {
            return;
        };
        if !device.has_vram_accounting() {
            return;
        }
        let Some(fp_key) = self.footprint_key(key, &device.device_key()) else {
            return;
        };
        let Some(fp) = self.vram.get(&fp_key) else {
            return;
        };
        let needed = if self.loaded.contains_key(key) {
            fp.working_estimate(crop_mp)
        } else {
            fp.estimate(crop_mp)
        }
        .saturating_add(vram::SAFETY_MARGIN);

        let active_run_needed = self.active_run_needed(runs);
        device.empty_cache();
        let mut free = device.free_vram();
        while free < needed {
            let Some(victim) = self
                .loaded
                .iter()
                .filter(|(k, _)| *k != key)
                .min_by_key(|(k, entry)| {
                    let in_active = active_run_needed.contains(*k);
                    let stage_distance = if in_active {
                        k.stage.index().abs_diff(key.stage.index())
                    } else {
                        0
                    };
                    (
                        u8::from(in_active),
                        std::cmp::Reverse(stage_distance),
                        entry.last_used,
                    )
                })
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            tracing::info!(
                model = %key.name,
                victim = %victim.name,
                free_bytes = free,
                needed_bytes = needed,
                "evicting model to make VRAM room"
            );
            self.evict(&victim);
            free = device.free_vram();
        }
    }

    /// Unloads a specific model, dropping its handle first, waiting synchronously
    /// for any local LLM engine thread teardown, then emptying the allocator cache.
    pub(crate) fn evict(&mut self, key: &Key) -> bool {
        let Some(entry) = self.loaded.remove(key) else {
            return false;
        };
        drop(entry.model);
        if let Some(teardown) = entry.teardown {
            teardown.wait();
        }
        self.empty_cache();
        self.sync_snapshot();
        true
    }

    fn evict_all_except(&mut self, keep: &Key) {
        let victims: Vec<Key> = self.loaded.keys().filter(|k| *k != keep).cloned().collect();
        if victims.is_empty() {
            return;
        }
        let mut teardowns = Vec::new();
        for key in victims {
            if let Some(entry) = self.loaded.remove(&key) {
                drop(entry.model);
                if let Some(teardown) = entry.teardown {
                    teardowns.push(teardown);
                }
            }
        }
        for teardown in teardowns {
            teardown.wait();
        }
        self.empty_cache();
        self.sync_snapshot();
    }

    /// Unloads every model `wanted` doesn't hold, waits for any local LLM
    /// thread teardown, and gives what they held back to the device.
    pub(crate) fn retain(&mut self, wanted: &HashSet<Key>) {
        let victims: Vec<Key> = self
            .loaded
            .keys()
            .filter(|key| !wanted.contains(*key))
            .cloned()
            .collect();
        if victims.is_empty() {
            self.release_lock_if_unloaded();
            return;
        }
        let mut teardowns = Vec::new();
        for key in victims {
            if let Some(entry) = self.loaded.remove(&key) {
                drop(entry.model);
                if let Some(teardown) = entry.teardown {
                    teardowns.push(teardown);
                }
            }
        }
        for teardown in teardowns {
            teardown.wait();
        }
        self.empty_cache();
        self.sync_snapshot();
        self.release_lock_if_unloaded();
    }

    /// Empties libtorch's cache.
    fn empty_cache(&self) {
        if let Some(device) = self.device.as_ref() {
            device.empty_cache();
        }
        self.sync_snapshot();
    }

    fn sync_snapshot(&self) {
        *self.snapshot.inner.resident.write() = self.loaded.keys().cloned().collect();
        if let Some(device) = self.device.as_ref() {
            *self.snapshot.inner.device_key.write() = Some(device.device_key());
            *self.snapshot.inner.free_vram.write() =
                device.has_vram_accounting().then(|| device.free_vram());
        }
    }

    fn has_other_loaded(&self, key: &Key) -> bool {
        self.loaded.keys().any(|k| k != key)
    }

    fn active_run_needed(&self, runs: &[RunId]) -> HashSet<Key> {
        let needed = self.needed_by_run.read();
        let mut union = HashSet::new();
        for run in runs {
            if let Some(keys) = needed.get(run) {
                union.extend(keys.iter().cloned());
            }
        }
        union
    }

    fn footprint_key(&self, key: &Key, device_key: &str) -> Option<FootprintKey> {
        let files = stages::files_for_key(key).ok()?;
        Some(FootprintKey {
            device: device_key.to_owned(),
            model_key: vram::model_key(key.stage, &key.id, key.vision),
            revision: vram::revision_for_files(&files),
        })
    }

    fn persist_runtime(&self) -> Option<&Handle> {
        self.persist_async.then_some(&self.runtime)
    }
}

struct FreePoller {
    stop: Arc<AtomicBool>,
    min_free: Arc<AtomicU64>,
    handle: std::thread::JoinHandle<()>,
}

impl FreePoller {
    fn start(device: Arc<dyn DeviceOps>, initial_free: u64) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let min_free = Arc::new(AtomicU64::new(initial_free));
        let stop_flag = Arc::clone(&stop);
        let min_ref = Arc::clone(&min_free);
        let handle = std::thread::Builder::new()
            .name("koharu-vram-poll".to_owned())
            .spawn(move || {
                while !stop_flag.load(AtomicOrdering::Acquire) {
                    let free = device.free_vram();
                    min_ref.fetch_min(free, AtomicOrdering::AcqRel);
                    std::thread::park_timeout(SD_FREE_POLL_INTERVAL);
                }
            })
            .expect("the OS starts a thread");
        Self {
            stop,
            min_free,
            handle,
        }
    }

    fn stop(self) -> u64 {
        self.stop.store(true, AtomicOrdering::Release);
        self.handle.thread().unpark();
        let _ = self.handle.join();
        self.min_free.load(AtomicOrdering::Acquire)
    }
}

fn is_oom_pipeline(error: &Error, device: &dyn DeviceOps, ooms_before: u64) -> bool {
    if device
        .cuda_stats()
        .is_some_and(|(_, _, ooms)| ooms > ooms_before)
    {
        return true;
    }
    match error {
        Error::Load { source, .. } | Error::Stage { source, .. } => {
            is_oom(source, device, ooms_before)
        }
        _ => false,
    }
}

fn is_oom(error: &anyhow::Error, device: &dyn DeviceOps, ooms_before: u64) -> bool {
    if device
        .cuda_stats()
        .is_some_and(|(_, _, ooms)| ooms > ooms_before)
    {
        return true;
    }
    if koharu_ml::is_oom(error) || koharu_llm::is_oom(error) {
        return true;
    }
    error.chain().any(|cause| {
        let msg = cause.to_string().to_ascii_lowercase();
        [
            "out of memory",
            "outofmemory",
            "doesn't fit in free memory",
            "could not find allocations that fit available memory",
            "cudaerrormemoryallocation",
            "hiperroroutofmemory",
            "vk::outofdevicememoryerror",
            "failed to allocate",
        ]
        .iter()
        .any(|needle| msg.contains(needle))
    })
}

/// The load failure, with the memory free on `device` and libtorch's.
fn failure(key: &Key, device: &dyn DeviceOps, source: anyhow::Error) -> Error {
    let held = device.cuda_stats();
    Error::Load {
        model: key.name.clone(),
        device: device.description().to_owned(),
        free: ByteSize(device.free_vram()),
        reserved: ByteSize(held.map_or(0, |(reserved, _, _)| reserved)),
        active: ByteSize(0),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use tokio::runtime::Runtime;

    use super::*;
    use crate::vram::Footprint;

    struct MockDevice {
        accounting: bool,
        free: Arc<AtomicU64>,
        empty_cache_calls: AtomicUsize,
        ooms: AtomicU64,
    }

    impl MockDevice {
        fn new(accounting: bool, free: u64) -> Arc<Self> {
            Arc::new(Self {
                accounting,
                free: Arc::new(AtomicU64::new(free)),
                empty_cache_calls: AtomicUsize::new(0),
                ooms: AtomicU64::new(0),
            })
        }
    }

    impl DeviceOps for MockDevice {
        fn has_vram_accounting(&self) -> bool {
            self.accounting
        }

        fn free_vram(&self) -> u64 {
            self.free.load(AtomicOrdering::SeqCst)
        }

        fn device_key(&self) -> String {
            "cuda:CUDA0:Mock GPU".to_owned()
        }

        fn description(&self) -> &str {
            "Mock GPU"
        }

        fn empty_cache(&self) {
            self.empty_cache_calls.fetch_add(1, AtomicOrdering::SeqCst);
        }

        fn set_device(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn reset_peak_stats(&self) {}

        fn cuda_stats(&self) -> Option<(u64, u64, u64)> {
            Some((0, 0, self.ooms.load(AtomicOrdering::SeqCst)))
        }
    }

    struct MockModel {
        size: u64,
        free_ref: Arc<AtomicU64>,
    }

    impl Drop for MockModel {
        fn drop(&mut self) {
            self.free_ref.fetch_add(self.size, AtomicOrdering::SeqCst);
        }
    }

    fn key(stage: Stage, id: &str) -> Key {
        Key {
            stage,
            name: id.to_owned(),
            id: id.to_owned(),
            vision: false,
        }
    }

    fn test_models(
        rt: &Runtime,
        device: Arc<MockDevice>,
        vram: VramStore,
        needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
    ) -> Models {
        test_models_with_lock(rt, device, vram, needed_by_run, LockMode::AcquireIn(None))
    }

    fn test_models_with_lock(
        rt: &Runtime,
        device: Arc<MockDevice>,
        vram: VramStore,
        needed_by_run: Arc<RwLock<HashMap<RunId, HashSet<Key>>>>,
        lock_mode: LockMode,
    ) -> Models {
        Models {
            runtime: rt.handle().clone(),
            persist_async: false,
            shutdown: CancellationToken::new(),
            loaded: HashMap::new(),
            tick: 0,
            device: Some(device),
            runtime_device: None,
            snapshot: GpuSnapshot::new(vram.clone()),
            vram,
            needed_by_run,
            lock_mode,
            owned_lock: None,
        }
    }

    fn record_fp(vram: &VramStore, key: &Key, load_bytes: u64, working_bytes: u64) {
        let files = stages::files_for_key(key).unwrap();
        vram.record_load(
            FootprintKey {
                device: "cuda:CUDA0:Mock GPU".to_owned(),
                model_key: vram::model_key(key.stage, &key.id, key.vision),
                revision: vram::revision_for_files(&files),
            },
            load_bytes,
            false,
            None,
        );
        vram.record_inference(
            FootprintKey {
                device: "cuda:CUDA0:Mock GPU".to_owned(),
                model_key: vram::model_key(key.stage, &key.id, key.vision),
                revision: vram::revision_for_files(&files),
            },
            working_bytes,
            None,
            None,
        );
    }

    #[test]
    fn prepare_evicts_unused_lru_then_active_furthest_stage_lru() {
        let rt = Runtime::new().unwrap();
        let gib = 1024 * 1024 * 1024;
        // Total 8 GiB, currently 1 GiB free after 4 models loaded.
        let device = MockDevice::new(true, gib);
        let vram = VramStore::in_memory();

        let k_detect_active = key(Stage::Detect, "koharu-layout-highres");
        let k_rec_unused_old = key(Stage::Recognize, "manga-ocr-ar");
        let k_rec_unused_new = key(Stage::Recognize, "baberu-ocr");
        let k_inpaint_active_close = key(Stage::Inpaint, "lama");
        let k_translate_target = key(Stage::Translate, "koharu/gemma4-e2b-it");

        record_fp(&vram, &k_translate_target, 3 * gib, 0);

        let mut needed_map = HashMap::new();
        needed_map.insert(
            RunId(1),
            HashSet::from([k_detect_active.clone(), k_inpaint_active_close.clone()]),
        );
        let needed_by_run = Arc::new(RwLock::new(needed_map));

        let mut models = test_models(&rt, Arc::clone(&device), vram, needed_by_run);
        // Insert with explicit LRU ticks:
        // 1: k_rec_unused_old (unused, oldest)
        // 2: k_detect_active (active, stage 0 -> dist 3 from Translate)
        // 3: k_rec_unused_new (unused, newer)
        // 4: k_inpaint_active_close (active, stage 2 -> dist 1 from Translate)
        let insert = |models: &mut Models, k: Key, size: u64, tick: u64| {
            models.loaded.insert(
                k,
                Loaded {
                    model: Box::new(MockModel {
                        size,
                        free_ref: Arc::clone(&device.free),
                    }),
                    last_used: tick,
                    teardown: None,
                },
            );
        };
        insert(&mut models, k_rec_unused_old.clone(), gib, 1);
        insert(&mut models, k_detect_active.clone(), gib, 2);
        insert(&mut models, k_rec_unused_new.clone(), gib, 3);
        insert(&mut models, k_inpaint_active_close.clone(), gib, 4);

        // Target needs 3 GiB + 512 MiB = 3.5 GiB; currently 1 GiB free -> must evict 3 models.
        // Expected eviction order:
        // 1. k_rec_unused_old (unused, tick 1) -> free = 2 GiB
        // 2. k_rec_unused_new (unused, tick 3) -> free = 3 GiB
        // 3. k_detect_active (active, dist 3 > dist 1) -> free = 4 GiB >= 3.5 GiB -> stop!
        models.prepare(&k_translate_target, &[RunId(1)], None);

        assert!(!models.loaded.contains_key(&k_rec_unused_old));
        assert!(!models.loaded.contains_key(&k_rec_unused_new));
        assert!(!models.loaded.contains_key(&k_detect_active));
        assert!(models.loaded.contains_key(&k_inpaint_active_close));
        assert!(device.empty_cache_calls.load(AtomicOrdering::SeqCst) >= 4);
    }

    #[test]
    fn prepare_and_recording_bypass_when_vram_accounting_disabled() {
        let rt = Runtime::new().unwrap();
        let gib = 1024 * 1024 * 1024;
        let device = MockDevice::new(false, 100);
        let vram = VramStore::in_memory();
        let k_loaded = key(Stage::Detect, "koharu-layout-highres");
        let k_target = key(Stage::Inpaint, "flux3-klein");
        record_fp(&vram, &k_target, 4 * gib, gib);

        let mut models = test_models(
            &rt,
            Arc::clone(&device),
            vram.clone(),
            Arc::new(RwLock::new(HashMap::new())),
        );
        models.loaded.insert(
            k_loaded.clone(),
            Loaded {
                model: Box::new(MockModel {
                    size: gib,
                    free_ref: Arc::clone(&device.free),
                }),
                last_used: 1,
                teardown: None,
            },
        );

        models.prepare(&k_target, &[], None);
        assert!(models.loaded.contains_key(&k_loaded));

        // Running an unmeasured model without accounting must not record a footprint.
        let out = models
            .run_loaded(&k_loaded, None, |_: &MockModel| Ok(42))
            .unwrap();
        assert_eq!(out, 42);
        let fp_key = models
            .footprint_key(&k_loaded, &device.device_key())
            .unwrap();
        assert_eq!(vram.get(&fp_key), None::<Footprint>);

        // Loading a model with full_gpu = false on a non-accounting host must not evict others or retry.
        let k_llm = key(Stage::Translate, "koharu/gemma4-e2b-it");
        let load_attempts = AtomicUsize::new(0);
        models
            .ensure_loaded_raw(&k_llm, |_| {
                load_attempts.fetch_add(1, AtomicOrdering::SeqCst);
                let free = Arc::clone(&device.free);
                async move {
                    Ok(LoadOutcome {
                        model: MockModel {
                            size: gib,
                            free_ref: free,
                        },
                        teardown: None,
                        full_gpu: false,
                    })
                }
            })
            .unwrap();
        assert_eq!(load_attempts.load(AtomicOrdering::SeqCst), 1);
        assert!(models.loaded.contains_key(&k_loaded));
        assert!(models.loaded.contains_key(&k_llm));
    }

    #[test]
    fn reactive_oom_retry_evicts_other_models_on_load_and_inference() {
        let rt = Runtime::new().unwrap();
        let gib = 1024 * 1024 * 1024;
        let device = MockDevice::new(true, 4 * gib);
        let vram = VramStore::in_memory();
        let k_other = key(Stage::Detect, "koharu-layout-highres");
        let k_target = key(Stage::Inpaint, "lama");

        let mut models = test_models(
            &rt,
            Arc::clone(&device),
            vram.clone(),
            Arc::new(RwLock::new(HashMap::new())),
        );
        models.loaded.insert(
            k_other.clone(),
            Loaded {
                model: Box::new(MockModel {
                    size: 2 * gib,
                    free_ref: Arc::clone(&device.free),
                }),
                last_used: 1,
                teardown: None,
            },
        );

        let load_attempts = AtomicUsize::new(0);
        models
            .ensure_loaded_raw(&k_target, |_| {
                let n = load_attempts.fetch_add(1, AtomicOrdering::SeqCst);
                let free = Arc::clone(&device.free);
                async move {
                    if n == 0 {
                        anyhow::bail!("CUDA out of memory");
                    }
                    free.fetch_sub(gib, AtomicOrdering::SeqCst);
                    Ok(LoadOutcome {
                        model: MockModel {
                            size: gib,
                            free_ref: free,
                        },
                        teardown: None,
                        full_gpu: true,
                    })
                }
            })
            .expect("second load attempt succeeds after evicting other model");
        assert_eq!(load_attempts.load(AtomicOrdering::SeqCst), 2);
        assert!(!models.loaded.contains_key(&k_other));
        assert!(models.loaded.contains_key(&k_target));

        // Now test reactive OOM retry during inference (via LibTorch ooms counter increment).
        models.loaded.insert(
            k_other.clone(),
            Loaded {
                model: Box::new(MockModel {
                    size: gib,
                    free_ref: Arc::clone(&device.free),
                }),
                last_used: 2,
                teardown: None,
            },
        );
        let infer_attempts = AtomicUsize::new(0);
        let result = models
            .run_loaded(&k_target, Some(1.0), |_: &MockModel| {
                let n = infer_attempts.fetch_add(1, AtomicOrdering::SeqCst);
                if n == 0 {
                    device.ooms.fetch_add(1, AtomicOrdering::SeqCst);
                    anyhow::bail!("unexpected allocator failure");
                }
                Ok("ok")
            })
            .expect("second infer attempt succeeds after evicting other model");
        assert_eq!(result, "ok");
        assert_eq!(infer_attempts.load(AtomicOrdering::SeqCst), 2);
        assert!(!models.loaded.contains_key(&k_other));
    }

    #[test]
    fn accelerator_lock_acquired_on_first_load_blocks_external_holder_and_releases_on_unload_and_drop()
     {
        let rt = Runtime::new().unwrap();
        let lock_dir = tempfile::tempdir().unwrap();
        let device = MockDevice::new(false, 1024);
        let vram = VramStore::in_memory();
        let k_detect = key(Stage::Detect, "koharu-layout-highres");

        let mut models = test_models_with_lock(
            &rt,
            Arc::clone(&device),
            vram,
            Arc::new(RwLock::new(HashMap::new())),
            LockMode::AcquireIn(Some(lock_dir.path().to_path_buf())),
        );

        // Lock is not acquired before the first GPU load.
        assert!(models.owned_lock.is_none());
        let probe = koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())
            .expect("external lock succeeds before first load");
        drop(probe);

        // Contends if an external holder already owns the lock.
        let external = koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).unwrap();
        let err = models
            .ensure_accelerator_lock(&k_detect)
            .expect_err("must fail when external holder owns lock");
        assert!(matches!(err, Error::AcceleratorLocked(_)));
        assert!(!is_oom_pipeline(&err, device.as_ref(), 0));
        drop(external);

        // Acquires on load and blocks external acquisition while a model is loaded.
        models.ensure_accelerator_lock(&k_detect).unwrap();
        models
            .ensure_loaded_raw(&k_detect, |_| {
                let free = Arc::clone(&device.free);
                async move {
                    Ok(LoadOutcome {
                        model: MockModel {
                            size: 1,
                            free_ref: free,
                        },
                        teardown: None,
                        full_gpu: true,
                    })
                }
            })
            .unwrap();
        assert!(models.owned_lock.is_some());
        assert!(koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).is_err());

        // `retain(&HashSet::new())` (used by `Pipeline::unload()`) releases the lock.
        models.retain(&HashSet::new());
        assert!(models.loaded.is_empty());
        assert!(models.owned_lock.is_none());
        let probe = koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())
            .expect("external lock succeeds after unload");
        drop(probe);

        // Re-acquires after unload when a model is loaded again, even though `device` is Some.
        models.ensure_accelerator_lock(&k_detect).unwrap();
        models
            .ensure_loaded_raw(&k_detect, |_| {
                let free = Arc::clone(&device.free);
                async move {
                    Ok(LoadOutcome {
                        model: MockModel {
                            size: 1,
                            free_ref: free,
                        },
                        teardown: None,
                        full_gpu: true,
                    })
                }
            })
            .unwrap();
        assert!(koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).is_err());

        // Dropping `Models` runs `retain(&HashSet::new())` and releases the lock.
        drop(models);
        koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())
            .expect("external lock succeeds after Models drops");
    }

    #[test]
    fn accelerator_lock_released_if_initial_load_fails() {
        let rt = Runtime::new().unwrap();
        let lock_dir = tempfile::tempdir().unwrap();
        let device = MockDevice::new(false, 1024);
        let vram = VramStore::in_memory();
        let k_detect = key(Stage::Detect, "koharu-layout-highres");

        let mut models = test_models_with_lock(
            &rt,
            Arc::clone(&device),
            vram,
            Arc::new(RwLock::new(HashMap::new())),
            LockMode::AcquireIn(Some(lock_dir.path().to_path_buf())),
        );

        models.ensure_accelerator_lock(&k_detect).unwrap();
        let err = models
            .ensure_loaded_raw::<MockModel, _>(&k_detect, |_| async move {
                anyhow::bail!("simulated load failure")
            })
            .expect_err("load must fail");
        assert!(matches!(err, Error::Load { .. }));
        models.release_lock_if_unloaded();
        assert!(models.owned_lock.is_none());
        koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())
            .expect("lock released after failed initial load");
    }

    #[test]
    fn accelerator_lock_stays_held_when_prepare_evicts_sole_resident_model() {
        let rt = Runtime::new().unwrap();
        let lock_dir = tempfile::tempdir().unwrap();
        let gib = 1024 * 1024 * 1024;
        let device = MockDevice::new(true, 3 * gib);
        let vram = VramStore::in_memory();

        let k_old = key(Stage::Detect, "koharu-layout-highres");
        let k_new = key(Stage::Translate, "koharu/gemma4-e2b-it");
        record_fp(&vram, &k_new, 2 * gib, 0);

        let mut models = test_models_with_lock(
            &rt,
            Arc::clone(&device),
            vram,
            Arc::new(RwLock::new(HashMap::new())),
            LockMode::AcquireIn(Some(lock_dir.path().to_path_buf())),
        );

        let load_mock = |free_ref: Arc<AtomicU64>, size: u64| async move {
            free_ref.fetch_sub(size, AtomicOrdering::SeqCst);
            Ok(LoadOutcome {
                model: MockModel { size, free_ref },
                teardown: None,
                full_gpu: true,
            })
        };

        // Load sole resident model `k_old` (consumes 2 GiB, leaving 1 GiB free).
        models.ensure_accelerator_lock(&k_old).unwrap();
        models
            .ensure_loaded_raw(&k_old, |_| load_mock(Arc::clone(&device.free), 2 * gib))
            .unwrap();
        assert_eq!(device.free_vram(), gib);
        assert!(models.owned_lock.is_some());
        assert!(koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).is_err());

        // Swapping to `k_new` requires 2.5 GiB > 1 GiB free, so `prepare` evicts `k_old`.
        models.ensure_accelerator_lock(&k_new).unwrap();
        models.prepare(&k_new, &[], None);
        assert!(!models.loaded.contains_key(&k_old));
        assert!(models.loaded.is_empty());
        assert_eq!(device.free_vram(), 3 * gib);
        assert!(
            models.owned_lock.is_some(),
            "accelerator lock must remain held after prepare evicts sole resident model"
        );
        assert!(
            koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).is_err(),
            "external process must remain blocked between prepare eviction and ensure_loaded_raw"
        );

        // Lock remains held throughout `ensure_loaded_raw` and once `k_new` is resident.
        let lock_dir_path = lock_dir.path().to_path_buf();
        models
            .ensure_loaded_raw(&k_new, |_| {
                assert!(
                    koharu_runtime::AcceleratorLock::acquire_in(&lock_dir_path).is_err(),
                    "external process must remain blocked during replacement load"
                );
                load_mock(Arc::clone(&device.free), 2 * gib)
            })
            .unwrap();
        assert!(models.loaded.contains_key(&k_new));
        assert!(models.owned_lock.is_some());
        assert!(koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).is_err());

        // `retain(&HashSet::new())` (used by `unload()`) releases the lock once empty.
        models.retain(&HashSet::new());
        assert!(models.loaded.is_empty());
        assert!(models.owned_lock.is_none());
        let probe = koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())
            .expect("external lock succeeds after retain(&HashSet::new())");
        drop(probe);

        // Verify `Drop` also releases the lock after a full swap.
        models.ensure_accelerator_lock(&k_old).unwrap();
        models
            .ensure_loaded_raw(&k_old, |_| load_mock(Arc::clone(&device.free), 2 * gib))
            .unwrap();
        models.ensure_accelerator_lock(&k_new).unwrap();
        models.prepare(&k_new, &[], None);
        assert!(models.owned_lock.is_some());
        models
            .ensure_loaded_raw(&k_new, |_| load_mock(Arc::clone(&device.free), 2 * gib))
            .unwrap();
        assert!(koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path()).is_err());

        drop(models);
        koharu_runtime::AcceleratorLock::acquire_in(lock_dir.path())
            .expect("external lock succeeds after Models drops");
    }
}
