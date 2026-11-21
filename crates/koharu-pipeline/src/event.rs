//! What a run tells the app as it goes, what each run told so far, and the
//! spans each run leaves: one from its queuing to its end, made where it was
//! asked for so it continues that trace, and one for each page stage while
//! it runs.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

use koharu_scene::PageId;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use tracing::{Span, info_span};

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

/// Where events go: every run's history and spans, then whoever listens.
/// Recording as they're sent means a history never misses one, however
/// slow a listener.
#[derive(Clone)]
pub(crate) struct Events {
    sender: broadcast::Sender<Event>,
    runs: Arc<Mutex<HashMap<RunId, Record>>>,
}

struct Record {
    events: Vec<Event>,
    /// Whether the run ended, which a waiting history watches.
    ended: watch::Sender<bool>,
    /// The run's span until it ends.
    span: Option<Span>,
    /// Each page stage's span while it runs.
    stages: BTreeMap<(PageId, Stage), Span>,
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
            match &event {
                // Sent by `Pipeline::run`, on its caller's thread.
                Event::Queued { run, steps } => {
                    record.span = Some(info_span!(
                        "pipeline run",
                        sentry.op = "pipeline.run",
                        run = run.0,
                        steps,
                        outcome = tracing::field::Empty,
                    ));
                }
                Event::Started { page, stage, .. } => {
                    if let Some(span) = &record.span {
                        let stage_span = info_span!(
                            parent: span,
                            "pipeline stage",
                            sentry.op = "pipeline.stage",
                            %stage,
                            %page,
                        );
                        record.stages.insert((*page, *stage), stage_span);
                    }
                }
                Event::Finished { page, stage, .. } => {
                    record.stages.remove(&(*page, *stage));
                }
                Event::Ended { outcome, .. } => {
                    record.ended.send_replace(true);
                    // A failed or stopped run leaves stages it never finished.
                    record.stages.clear();
                    if let Some(span) = record.span.take() {
                        span.record(
                            "outcome",
                            match outcome {
                                Outcome::Done => "done",
                                Outcome::Failed { .. } => "failed",
                                Outcome::Stopped => "stopped",
                            },
                        );
                    }
                }
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
            span: None,
            stages: BTreeMap::new(),
        }
    }
}
