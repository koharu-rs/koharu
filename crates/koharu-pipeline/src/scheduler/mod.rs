//! The task that owns the runs, so nothing is locked. It admits ready jobs
//! while a permit for a page in flight is free, each a Tokio task in one
//! `JoinSet`, and hears how each went from it, a panic included. Calls wait
//! for the GPU thread in its own line, by priority. The stages are known here
//! only by `Stage` and by the model `stages::key` names.

mod run;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use koharu_scene::{PageId, ProjectId};
use parking_lot::RwLock;
use petgraph::{Direction::Incoming, stable_graph::NodeIndex};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::{self, JoinError, JoinSet},
};

pub(crate) use self::run::{Plan, Run};
use crate::{
    Event, Outcome, PipelineConfig, RunId, Stage,
    event::Events,
    gpu::{Gpu, Key, Turn},
    stages::{self, Inputs, Member, Task, translate::Conversations},
};

/// Pages admitted and not yet written: one reading its input, one on the
/// GPU, one writing its result, so the CPU works beside the GPU while few
/// pages are held.
const AHEAD: usize = 3;
/// The most pages one recognition call reads.
const RECOGNIZE_PAGES: usize = 8;

pub(crate) enum Command {
    Run(Run),
    Stop,
    Unload,
    /// The settings changed: unload what nothing uses now.
    Settle,
    Close(ProjectId),
}

/// How a run is asked for, as BigQuery runs a job interactive or batch: an
/// interactive run, on blocks or an area, goes ahead of a batch run on pages.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Mode {
    Interactive,
    Batch,
}

/// A job's place in line; the smallest goes first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Priority {
    mode: Mode,
    run: RunId,
    page: usize,
    stage: Stage,
}

/// A ready job: its run's place among the runs, and its node.
#[derive(Clone, Copy)]
struct Candidate {
    run: usize,
    job: NodeIndex,
    stage: Stage,
    priority: Priority,
}

/// A running task's stage, its jobs, and the page it fails on.
struct Running {
    stage: Stage,
    jobs: Vec<(RunId, NodeIndex)>,
    page: PageId,
}

pub(crate) struct Scheduler {
    commands: mpsc::UnboundedReceiver<Command>,
    events: Events,
    gpu: Gpu,
    config: Arc<RwLock<PipelineConfig>>,
    /// In the order they were asked for.
    runs: Vec<Run>,
    /// A permit for each page in flight, which its task holds.
    ahead: Arc<Semaphore>,
    tasks: JoinSet<Result<(), String>>,
    running: HashMap<task::Id, Running>,
    conversations: Conversations,
}

impl Scheduler {
    /// Starts the scheduler and the GPU thread. They end when every sender
    /// the pipeline holds drops.
    pub(crate) fn spawn(
        config: Arc<RwLock<PipelineConfig>>,
        events: Events,
    ) -> mpsc::UnboundedSender<Command> {
        let (sender, commands) = mpsc::unbounded_channel();
        let scheduler = Self {
            commands,
            events,
            gpu: Gpu::spawn(Handle::current()),
            config,
            runs: Vec::new(),
            ahead: Arc::new(Semaphore::new(AHEAD)),
            tasks: JoinSet::new(),
            running: HashMap::new(),
            conversations: Conversations::default(),
        };
        tokio::spawn(scheduler.run());
        sender
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => self.handle(command),
                    None => break,
                },
                Some(joined) = self.tasks.join_next_with_id() => self.done(joined),
            }
            self.admit();
        }
        // The pipeline is gone: what's running stops, and nothing is written.
        for run in &self.runs {
            run.cancel.cancel();
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
            Command::Run(run) => self.runs.push(run),
            Command::Stop => {
                // Each task's call waiting for the GPU is dropped unrun.
                for run in std::mem::take(&mut self.runs) {
                    run.cancel.cancel();
                    self.emit(Event::Ended {
                        run: run.id,
                        outcome: Outcome::Stopped,
                    });
                }
                self.settle();
            }
            Command::Unload => self
                .gpu
                .run(Turn::Upkeep, |models| models.retain(&HashSet::new())),
            Command::Settle => self.settle(),
            Command::Close(project) => self.conversations.close(project),
        }
    }

    fn done(&mut self, joined: Result<(task::Id, Result<(), String>), JoinError>) {
        let id = match &joined {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let Running { stage, jobs, page } = self
            .running
            .remove(&id)
            .expect("every task is recorded when it starts");
        let result = match joined {
            Ok((_, result)) => result,
            Err(error) => Err(format!("{stage} failed: {error}")),
        };
        let runs = std::mem::take(&mut self.runs);
        let mut ended = false;
        match result {
            Ok(()) => {
                for mut run in runs {
                    for &(id, job) in &jobs {
                        if id == run.id
                            && let Some(job) = run.graph.remove_node(job)
                        {
                            self.emit(Event::Finished {
                                run: id,
                                page: job.page,
                                stage: job.stage,
                            });
                        }
                    }
                    if run.graph.node_count() == 0 {
                        ended = true;
                        self.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Done,
                        });
                    } else {
                        self.runs.push(run);
                    }
                }
            }
            Err(reason) => {
                for run in runs {
                    if jobs.iter().any(|&(id, _)| id == run.id) {
                        ended = true;
                        run.cancel.cancel();
                        self.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Failed {
                                page,
                                stage,
                                reason: reason.clone(),
                            },
                        });
                    } else {
                        self.runs.push(run);
                    }
                }
            }
        }
        if ended {
            self.settle();
        }
    }

    /// Unloads the models neither the settings nor a queued run use, once
    /// the GPU's current call ends.
    fn settle(&self) {
        let mut wanted: HashSet<Key> = stages::chosen(&self.config.read()).collect();
        for run in &self.runs {
            wanted.extend(stages::chosen(&run.config));
        }
        self.gpu
            .run(Turn::Upkeep, move |models| models.retain(&wanted));
    }

    /// The ready jobs not yet started. A project's translation isn't ready
    /// while another holds its conversation.
    fn ready(&self) -> Vec<Candidate> {
        self.runs
            .iter()
            .enumerate()
            .flat_map(|(at, run)| {
                run.graph.externals(Incoming).filter_map(move |job| {
                    let node = &run.graph[job];
                    let held =
                        node.stage == Stage::Translate && self.conversations.busy(run.project);
                    (!node.running && !held).then_some(Candidate {
                        run: at,
                        job,
                        stage: node.stage,
                        priority: Priority {
                            mode: run.mode,
                            run: run.id,
                            page: node.index,
                            stage: node.stage,
                        },
                    })
                })
            })
            .collect()
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
                    stages::key(candidate.stage, &self.runs[candidate.run].config).is_none()
                })
                .min_by_key(|candidate| candidate.priority);
            if let Some(&hosted) = hosted {
                self.start(vec![hosted], None);
                continue;
            }
            let Some(&first) = ready.iter().min_by_key(|candidate| candidate.priority) else {
                return;
            };
            let Ok(permit) = Arc::clone(&self.ahead).try_acquire_owned() else {
                return;
            };
            let mut chosen = vec![first];
            if first.stage == Stage::Recognize {
                let model = stages::key(first.stage, &self.runs[first.run].config);
                let mut others: Vec<Candidate> = ready
                    .iter()
                    .filter(|other| {
                        other.stage == Stage::Recognize
                            && other.priority != first.priority
                            && stages::key(other.stage, &self.runs[other.run].config) == model
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
                let run = &mut self.runs[candidate.run];
                let job = &mut run.graph[candidate.job];
                job.running = true;
                Member {
                    run: run.id,
                    job: candidate.job,
                    page: job.page,
                    index: job.index,
                    blocks: job.blocks.clone(),
                    session: Arc::clone(&run.session),
                    cancel: run.cancel.clone(),
                }
            })
            .collect();
        let first = chosen[0];
        let run = &self.runs[first.run];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_runs_go_first_then_the_oldest_page_in_stage_order() {
        let pages = Priority {
            mode: Mode::Batch,
            run: RunId(0),
            page: 0,
            stage: Stage::Detect,
        };
        let interactive = Priority {
            mode: Mode::Interactive,
            run: RunId(1),
            page: 3,
            stage: Stage::Translate,
        };
        let later = Priority { page: 1, ..pages };
        let next_stage = Priority {
            stage: Stage::Recognize,
            ..pages
        };
        assert!(interactive < pages && pages < next_stage && next_stage < later);
    }
}
