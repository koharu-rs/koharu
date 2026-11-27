//! What a run tells the app as it goes, what each run told so far, and the
//! spans each run leaves: one from its queuing to its end, made where it was
//! asked for so it continues that trace, and one for each page stage while
//! it runs.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use koharu_scene::PageId;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use tracing::{Span, info_span};

use crate::{RunPriority, Stage, Target};

/// How many events a slow listener can fall behind by.
const EVENTS: usize = 1024;

/// A run, numbered in the order runs were asked for.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
#[derive(specta::Type)]
pub struct RunId(pub u64);

/// Why a queued run sits at its current position in the queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum QueueOrderReason {
    Fifo,
    GroupedByLoadedModel,
    LowerVramCost,
    AgedPriority,
    WaitingForDownload,
}

/// Lightweight summary of a run's target scope (without pixel buffers).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueScope {
    Project,
    Pages { count: usize },
    Blocks { count: usize },
    Area,
    Metadata,
}

impl QueueScope {
    #[must_use]
    pub(crate) fn from_target(target: &Target) -> Self {
        match target {
            Target::Project => Self::Project,
            Target::Pages(pages) => Self::Pages { count: pages.len() },
            Target::Blocks(blocks) => Self::Blocks {
                count: blocks.len(),
            },
            Target::Area { .. } => Self::Area,
            Target::Metadata => Self::Metadata,
        }
    }
}

/// One page-stage (or metadata-translation) job inside a run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, specta::Type)]
pub struct QueuedStep {
    pub page: Option<PageId>,
    pub stage: Stage,
}

/// A queued or download-waiting run in the scheduler's current execution order.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct QueuedRunSummary {
    pub run: RunId,
    pub steps: usize,
    pub scope: QueueScope,
    pub stages: BTreeSet<Stage>,
    pub jobs: Vec<QueuedStep>,
    pub priority: RunPriority,
    pub waiting_downloads: Vec<u64>,
    pub reason: Option<QueueOrderReason>,
}

/// A run was queued, waiting on downloads, coalesced, a stage started,
/// finished, failed, or was cancelled on a page, or a run ended. A stage
/// starts when the GPU takes it, or for a hosted translation when it's sent.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
// Named apart from the agent's own `Event`, which crosses to the interface
// too; JSON carries no container name.
#[serde(rename = "PipelineEvent", tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The run's first event: the page stages it does, each of which starts
    /// once, and finishes or fails, unless the job or run is cancelled or
    /// coalesces first.
    Queued {
        run: RunId,
        steps: usize,
        priority: RunPriority,
        scope: QueueScope,
        stages: BTreeSet<Stage>,
        jobs: Vec<QueuedStep>,
    },
    WaitingForDownload {
        run: RunId,
        downloads: Vec<u64>,
    },
    Started {
        run: RunId,
        page: Option<PageId>,
        stage: Stage,
        model: Option<String>,
    },
    Finished {
        run: RunId,
        page: Option<PageId>,
        stage: Stage,
    },
    /// A failure that is the page's (or metadata step's) alone: the run goes on.
    Failed {
        run: RunId,
        page: Option<PageId>,
        stage: Stage,
        reason: String,
    },
    /// A single page-stage (or metadata) job inside a run was cancelled.
    JobCancelled {
        run: RunId,
        page: Option<PageId>,
        stage: Stage,
    },
    /// A queued batch run was merged into an earlier compatible queued run.
    Coalesced {
        run: RunId,
        into: RunId,
    },
    /// The scheduler's current queued and download-waiting order changed.
    QueueUpdated {
        order: Vec<QueuedRunSummary>,
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
    /// A failure that isn't one page's alone ends the run; what it finished
    /// stays.
    Failed {
        page: Option<PageId>,
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
    latest_queue: Arc<Mutex<Option<Event>>>,
}

struct Record {
    events: Vec<Event>,
    /// Whether the run ended, which a waiting history watches.
    ended: watch::Sender<bool>,
    /// The run's span until it ends.
    span: Option<Span>,
    /// Each stage's span while it runs.
    stages: BTreeMap<(Option<PageId>, Stage), Span>,
}

impl Events {
    pub(crate) fn new() -> Self {
        Self {
            sender: broadcast::channel(EVENTS).0,
            runs: Arc::default(),
            latest_queue: Arc::default(),
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    pub(crate) fn send(&self, event: Event) {
        if matches!(event, Event::QueueUpdated { .. }) {
            *self.latest_queue.lock() = Some(event.clone());
            let _ = self.sender.send(event);
            return;
        }
        let (Event::Queued { run, .. }
        | Event::WaitingForDownload { run, .. }
        | Event::Started { run, .. }
        | Event::Finished { run, .. }
        | Event::Failed { run, .. }
        | Event::JobCancelled { run, .. }
        | Event::Coalesced { run, .. }
        | Event::Ended { run, .. }) = &event
        else {
            unreachable!("QueueUpdated handled above");
        };
        {
            let mut runs = self.runs.lock();
            let record = runs.entry(*run).or_insert_with(Record::new);
            match &event {
                // Sent by `Pipeline::run`, on its caller's thread.
                Event::Queued { run, steps, .. } => {
                    record.span = Some(info_span!(
                        "pipeline run",
                        sentry.op = "pipeline.run",
                        run = run.0,
                        steps,
                        outcome = tracing::field::Empty,
                    ));
                }
                Event::WaitingForDownload { .. } | Event::QueueUpdated { .. } => {}
                Event::Started { page, stage, .. } => {
                    if let Some(span) = &record.span {
                        let stage_span = info_span!(
                            parent: span,
                            "pipeline stage",
                            sentry.op = "pipeline.stage",
                            %stage,
                            page = page.map(tracing::field::display),
                        );
                        record.stages.insert((*page, *stage), stage_span);
                    }
                }
                Event::Finished { page, stage, .. } | Event::JobCancelled { page, stage, .. } => {
                    record.stages.remove(&(*page, *stage));
                }
                Event::Failed {
                    page,
                    stage,
                    reason,
                    ..
                } => {
                    match page {
                        Some(_) => tracing::warn!("a page failed: {reason}"),
                        None => tracing::warn!("metadata failed: {reason}"),
                    }
                    record.stages.remove(&(*page, *stage));
                }
                Event::Coalesced { .. } => {
                    record.ended.send_replace(true);
                    record.stages.clear();
                    if let Some(span) = record.span.take() {
                        span.record("outcome", "coalesced");
                    }
                }
                Event::Ended { outcome, .. } => {
                    if let Outcome::Failed { reason, .. } = outcome {
                        tracing::warn!("a run failed: {reason}");
                    }
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

    /// What every run told so far, run by run, followed by the latest queue
    /// snapshot if one was emitted.
    pub(crate) fn told(&self) -> Vec<Event> {
        let mut all = {
            let runs = self.runs.lock();
            let mut ids: Vec<RunId> = runs.keys().copied().collect();
            ids.sort();
            ids.into_iter()
                .flat_map(|run| runs[&run].events.clone())
                .collect::<Vec<_>>()
        };
        if let Some(queue) = self.latest_queue.lock().clone() {
            all.push(queue);
        }
        all
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn coalesced_run_completes_history_and_queue_updated_stays_out_of_run_records() {
        let events = Events::new();
        let run = RunId(2);
        events.send(Event::Queued {
            run,
            steps: 2,
            priority: RunPriority::Normal,
            scope: QueueScope::Project,
            stages: BTreeSet::from([Stage::Detect]),
            jobs: Vec::new(),
        });
        let summary = QueuedRunSummary {
            run: RunId(1),
            steps: 3,
            scope: QueueScope::Project,
            stages: BTreeSet::from([Stage::Detect]),
            jobs: Vec::new(),
            priority: RunPriority::Normal,
            waiting_downloads: Vec::new(),
            reason: Some(QueueOrderReason::Fifo),
        };
        events.send(Event::QueueUpdated {
            order: vec![summary.clone()],
        });
        events.send(Event::Coalesced {
            run,
            into: RunId(1),
        });

        let started = std::time::Instant::now();
        let history = events
            .history(run, Duration::from_secs(5))
            .await
            .expect("coalesced run has history");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(history.len(), 2);
        assert!(matches!(
            history.last(),
            Some(Event::Coalesced {
                run: RunId(2),
                into: RunId(1)
            })
        ));
        assert!(
            !history
                .iter()
                .any(|ev| matches!(ev, Event::QueueUpdated { .. }))
        );

        let told = events.told();
        assert_eq!(
            told.last(),
            Some(&Event::QueueUpdated {
                order: vec![summary]
            })
        );
    }
}
