//! The thread that owns the models and runs one call at a time, as
//! koharu-llm's engine thread owns its model, so a model is never shared and
//! needs no lock. Calls wait in a priority queue, so an interactive run goes
//! ahead at the next call. A model loads the first time a stage asks for it and stays
//! until the settings switch away from it, `unload` is called, or the
//! pipeline ends. Nothing is unloaded to make room: a model that doesn't fit
//! fails, naming the memory free when it began to load. The stages know their
//! models; this knows only their keys.

use std::{
    any::Any,
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
    sync::mpsc,
};

use bytesize::ByteSize;
use koharu_runtime::Device;
use koharu_torch::Cuda;
use tokio::runtime::Handle;

use crate::{Error, Result, Stage, scheduler::Priority};

/// Work for the GPU thread.
pub(crate) type Work = Box<dyn FnOnce(&mut Models) + Send>;

/// A call's place in line, the smallest first: model upkeep goes ahead of
/// every stage's call.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Turn {
    Upkeep,
    Stage(Priority),
}

/// The way to the GPU thread. It ends, unloading the models, once every
/// handle drops and its queue is empty.
#[derive(Clone)]
pub(crate) struct Gpu(mpsc::Sender<(Turn, Work)>);

impl Gpu {
    pub(crate) fn spawn(runtime: Handle) -> Self {
        let (sender, receiver) = mpsc::channel::<(Turn, Work)>();
        std::thread::Builder::new()
            .name("koharu-pipeline-gpu".to_owned())
            .spawn(move || {
                let mut models = Models {
                    runtime,
                    loaded: HashMap::new(),
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
                    }
                }
            })
            .expect("the OS starts a thread");
        Self(sender)
    }

    /// Queues `work` at `turn`.
    pub(crate) fn run(&self, turn: Turn, work: impl FnOnce(&mut Models) + Send + 'static) {
        // The thread ends only once every handle has dropped.
        let _ = self.0.send((turn, Box::new(work)));
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

/// A model as the settings name it: its stage, and its name there.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Key {
    pub(crate) stage: Stage,
    pub(crate) name: String,
}

/// The loaded models, which the GPU thread owns.
pub(crate) struct Models {
    runtime: Handle,
    loaded: HashMap<Key, Box<dyn Any + Send>>,
}

impl Models {
    /// The model `key` names, which `load` gives the first time, on the
    /// runtime's device.
    pub(crate) fn get<M, F>(
        &mut self,
        key: &Key,
        load: impl FnOnce(&'static Device) -> F,
    ) -> Result<&M>
    where
        M: Send + 'static,
        F: Future<Output = anyhow::Result<M>>,
    {
        if !self.loaded.contains_key(key) {
            let _span = tracing::info_span!("load", model = %key.name).entered();
            let model = self.runtime.block_on(loaded(key, load))?;
            self.loaded.insert(key.clone(), Box::new(model));
        }
        Ok(self.loaded[key]
            .downcast_ref()
            .expect("a key loads one kind of model"))
    }

    /// Unloads every model `wanted` doesn't hold.
    pub(crate) fn retain(&mut self, wanted: &HashSet<Key>) {
        self.loaded.retain(|key, _| wanted.contains(key));
    }
}

/// The model `load` gives on the runtime's device, libtorch's cache emptied
/// first: its caching allocator keeps what its tensors freed, and the device
/// counts that as used, so llama.cpp and stable-diffusion.cpp, which fit
/// themselves to free memory, wouldn't see it as free.
async fn loaded<M, F>(key: &Key, load: impl FnOnce(&'static Device) -> F) -> Result<M>
where
    F: Future<Output = anyhow::Result<M>>,
{
    let device = koharu_runtime::device()
        .await
        .map_err(|source| Error::Load {
            model: key.name.clone(),
            device: "no device".to_owned(),
            free: ByteSize(0),
            reserved: ByteSize(0),
            active: ByteSize(0),
            source,
        })?;
    let cuda = Cuda::device_count() > 0;
    if cuda {
        Cuda::empty_cache().map_err(|source| failure(key, device, cuda, source.into()))?;
    }
    load(device)
        .await
        .map_err(|source| failure(key, device, cuda, source))
}

/// The load failure, with the memory free on `device` and libtorch's.
fn failure(key: &Key, device: &Device, cuda: bool, source: anyhow::Error) -> Error {
    let held = cuda
        .then(|| Cuda::memory(device.index as i64).ok())
        .flatten();
    Error::Load {
        model: key.name.clone(),
        device: device.description.clone(),
        free: ByteSize(device.memory().free),
        reserved: ByteSize(held.map_or(0, |held| held.reserved)),
        active: ByteSize(held.map_or(0, |held| held.active)),
        source,
    }
}
