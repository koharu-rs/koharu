//! What a run tells the app as it goes, and what each run told so far.

use std::{collections::HashMap, sync::Arc, time::Duration};

use koharu_scene::PageId;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::Stage;

/// How many events a slow listener can fall behind by.
const EVENTS: usize = 1024;

/// A run, numbered in the order runs were asked for.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[derive(specta::Type)]
pub struct RunId(pub u64);

/// A run was queued, a stage started or finished on a page, or a run ended.
/// A stage starts when the GPU takes it, or for a hosted translation when
/// it's sent.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
// Named apart from the agent's own `Event`, which crosses to the interface
// too; JSON carries no container name.
#[serde(rename = "PipelineEvent", tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The run's first event: the page stages it does, each of which starts
    /// and finishes once unless the run ends first.
    Queued {
        run: RunId,
        steps: usize,
    },
    Started {
        run: RunId,
        page: PageId,
        stage: Stage,
    },
    Finished {
        run: RunId,
        page: PageId,
        stage: Stage,
    },
    Ended {
        run: RunId,
        outcome: Outcome,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Done,
    /// The first failure ends the run; what it finished stays.
    Failed {
        page: PageId,
        stage: Stage,
        reason: String,
    },
    Stopped,
}

/// Where events go: every run's history, then whoever listens. Recording as
/// they're sent means a history never misses one, however slow a listener.
#[derive(Clone)]
pub(crate) struct Events {
    sender: broadcast::Sender<Event>,
    runs: Arc<Mutex<HashMap<RunId, Record>>>,
}

struct Record {
    events: Vec<Event>,
    /// Whether the run ended, which a waiting history watches.
    ended: watch::Sender<bool>,
}

impl Events {
    pub(crate) fn new() -> Self {
        Self {
            sender: broadcast::channel(EVENTS).0,
            runs: Arc::default(),
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    pub(crate) fn send(&self, event: Event) {
        let (Event::Queued { run, .. }
        | Event::Started { run, .. }
        | Event::Finished { run, .. }
        | Event::Ended { run, .. }) = &event;
        {
            let mut runs = self.runs.lock();
            let record = runs.entry(*run).or_insert_with(Record::new);
            if let Event::Ended { .. } = &event {
                record.ended.send_replace(true);
            }
            record.events.push(event.clone());
        }
        // No one listening isn't a failure.
        let _ = self.sender.send(event);
    }

    /// What every run told so far, run by run.
    pub(crate) fn told(&self) -> Vec<Event> {
        let runs = self.runs.lock();
        let mut ids: Vec<RunId> = runs.keys().copied().collect();
        ids.sort();
        ids.into_iter()
            .flat_map(|run| runs[&run].events.clone())
            .collect()
    }

    /// What `run` told so far, once it ended or `wait` is over; `None` for a
    /// run never asked for.
    pub(crate) async fn history(&self, run: RunId, wait: Duration) -> Option<Vec<Event>> {
        let mut ended = self.runs.lock().get(&run)?.ended.subscribe();
        // A run still going when the wait is over is told as it is.
        let _ = tokio::time::timeout(wait, ended.wait_for(|ended| *ended)).await;
        Some(self.runs.lock().get(&run)?.events.clone())
    }
}

impl Record {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            ended: watch::channel(false).0,
        }
    }
}
