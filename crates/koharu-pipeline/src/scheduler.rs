//! The task that owns the runs, so nothing is locked. Each run is a graph of
//! jobs, a page's step each, with an edge for what each needs: detection
//! before the page's other steps, recognition before translation, and each
//! page's translation before the next page's. A job is ready when nothing
//! points at it, and a finished job leaves the graph. The scheduler admits
//! ready jobs while fewer than three pages are in flight, and gives the GPU
//! thread the prepared one with the highest priority whenever it's free.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{Arc, mpsc as std_mpsc},
};

use koharu_scene::{LayerId, PageId, ProjectId, Session};
use parking_lot::{Mutex, RwLock};
use petgraph::{
    Direction::Incoming,
    stable_graph::{NodeIndex, StableDiGraph},
};
use tokio::{
    runtime::Handle,
    sync::{broadcast, mpsc},
};
use tokio_util::sync::CancellationToken;

use crate::{
    Event, Outcome, PipelineConfig, RunId, Step,
    models::{self, Key, Models, Work},
    steps::{
        Member, Task,
        detect::Detect,
        inpaint::{AreaWork, Inpaint},
        perform,
        recognize::Recognize,
        translate::{Conversation, Lent, Translate},
    },
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
    /// A prepared job, waiting for the GPU.
    Gpu {
        priority: Priority,
        work: Work,
    },
    GpuDone,
    Done(Done),
    /// A project's conversation, back from a translation.
    Returned {
        project: ProjectId,
        conversation: Box<Conversation>,
    },
}

/// How a task went.
pub(crate) struct Done {
    pub(crate) step: Step,
    pub(crate) members: Vec<(RunId, NodeIndex)>,
    pub(crate) counted: bool,
    /// The page it failed on, and why.
    pub(crate) result: Result<(), (PageId, String)>,
}

/// What a task holds of the scheduler: a way to reach it that doesn't keep
/// it running, and the events.
#[derive(Clone)]
pub(crate) struct Link {
    commands: mpsc::WeakUnboundedSender<Command>,
    events: broadcast::Sender<Event>,
}

impl Link {
    pub(crate) fn send(&self, command: Command) {
        if let Some(commands) = self.commands.upgrade() {
            // The scheduler ends only with the pipeline.
            let _ = commands.send(command);
        }
    }

    pub(crate) fn events(&self) -> broadcast::Sender<Event> {
        self.events.clone()
    }

    fn emit(&self, event: Event) {
        // No one listening isn't a failure.
        let _ = self.events.send(event);
    }
}

/// Runs on blocks or an area, by hand, go ahead of runs on pages.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Queue {
    Hand,
    Pages,
}

/// A job's place in line; the smallest goes first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Priority {
    queue: Queue,
    run: RunId,
    page: usize,
    step: Step,
}

pub(crate) struct Run {
    id: RunId,
    queue: Queue,
    session: Arc<Mutex<Session>>,
    project: ProjectId,
    config: Arc<PipelineConfig>,
    area: Option<Arc<AreaWork>>,
    graph: StableDiGraph<Job, ()>,
    cancel: CancellationToken,
}

struct Job {
    page: PageId,
    /// The page's place in the run.
    index: usize,
    step: Step,
    blocks: Option<Arc<[LayerId]>>,
    running: bool,
}

/// What a run works on: its pages in order, each with the blocks it's
/// limited to.
pub(crate) struct Plan {
    pub(crate) queue: Queue,
    pub(crate) pages: Vec<(PageId, Option<Arc<[LayerId]>>)>,
    pub(crate) area: Option<Arc<AreaWork>>,
}

impl Run {
    pub(crate) fn new(
        id: RunId,
        session: Arc<Mutex<Session>>,
        project: ProjectId,
        config: PipelineConfig,
        plan: Plan,
        steps: &BTreeSet<Step>,
    ) -> Self {
        let mut graph = StableDiGraph::new();
        let mut translated = None;
        for (index, (page, blocks)) in plan.pages.into_iter().enumerate() {
            let jobs: BTreeMap<Step, NodeIndex> = steps
                .iter()
                .map(|&step| {
                    let job = Job {
                        page,
                        index,
                        step,
                        blocks: blocks.clone(),
                        running: false,
                    };
                    (step, graph.add_node(job))
                })
                .collect();
            if let Some(&detect) = jobs.get(&Step::Detect) {
                for (_, &job) in jobs.range(Step::Recognize..) {
                    graph.add_edge(detect, job, ());
                }
            }
            if let Some(&translate) = jobs.get(&Step::Translate) {
                if let Some(&recognize) = jobs.get(&Step::Recognize) {
                    graph.add_edge(recognize, translate, ());
                }
                if let Some(before) = translated.replace(translate) {
                    graph.add_edge(before, translate, ());
                }
            }
        }
        Self {
            id,
            queue: plan.queue,
            session,
            project,
            config: Arc::new(config),
            area: plan.area,
            graph,
            cancel: CancellationToken::new(),
        }
    }
}

/// A ready job: its run's place among the runs, and its node.
#[derive(Clone, Copy)]
struct Candidate {
    run: usize,
    job: NodeIndex,
    step: Step,
    priority: Priority,
}

pub(crate) struct Scheduler {
    commands: mpsc::UnboundedReceiver<Command>,
    link: Link,
    gpu: std_mpsc::Sender<Work>,
    config: Arc<RwLock<PipelineConfig>>,
    /// In the order they were asked for.
    runs: Vec<Run>,
    in_flight: usize,
    waiting: Vec<(Priority, Work)>,
    gpu_busy: bool,
    /// Each open project's translation conversation, but those lent out.
    conversations: HashMap<ProjectId, Conversation>,
    lent: HashSet<ProjectId>,
    /// Projects closed while their conversation was lent out.
    closed: HashSet<ProjectId>,
}

impl Scheduler {
    /// Starts the scheduler and the GPU thread. They end when every sender
    /// the pipeline holds drops.
    pub(crate) fn spawn(
        config: Arc<RwLock<PipelineConfig>>,
        events: broadcast::Sender<Event>,
    ) -> mpsc::UnboundedSender<Command> {
        let (sender, commands) = mpsc::unbounded_channel();
        let scheduler = Self {
            commands,
            link: Link {
                commands: sender.downgrade(),
                events,
            },
            gpu: models::spawn(Handle::current()),
            config,
            runs: Vec::new(),
            in_flight: 0,
            waiting: Vec::new(),
            gpu_busy: false,
            conversations: HashMap::new(),
            lent: HashSet::new(),
            closed: HashSet::new(),
        };
        tokio::spawn(scheduler.run());
        sender
    }

    async fn run(mut self) {
        while let Some(command) = self.commands.recv().await {
            self.handle(command);
            self.admit();
            self.dispatch();
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Run(run) if run.graph.node_count() == 0 => self.link.emit(Event::Ended {
                run: run.id,
                outcome: Outcome::Done,
            }),
            Command::Run(run) => self.runs.push(run),
            Command::Stop => {
                for run in std::mem::take(&mut self.runs) {
                    run.cancel.cancel();
                    self.link.emit(Event::Ended {
                        run: run.id,
                        outcome: Outcome::Stopped,
                    });
                }
                // Each task waiting for the GPU sees its work dropped.
                self.waiting.clear();
                self.settle();
            }
            Command::Unload => self.on_gpu(|models| models.retain(&HashSet::new())),
            Command::Settle => self.settle(),
            Command::Close(project) => {
                if self.lent.contains(&project) {
                    self.closed.insert(project);
                }
                self.conversations.remove(&project);
            }
            Command::Gpu { priority, work } => self.waiting.push((priority, work)),
            Command::GpuDone => self.gpu_busy = false,
            Command::Done(done) => self.done(done),
            Command::Returned {
                project,
                conversation,
            } => {
                self.lent.remove(&project);
                if !self.closed.remove(&project) {
                    self.conversations.insert(project, *conversation);
                }
            }
        }
    }

    fn done(&mut self, done: Done) {
        if done.counted {
            self.in_flight -= 1;
        }
        let runs = std::mem::take(&mut self.runs);
        let mut ended = false;
        match done.result {
            Ok(()) => {
                for mut run in runs {
                    for &(id, job) in &done.members {
                        if id == run.id
                            && let Some(job) = run.graph.remove_node(job)
                        {
                            self.link.emit(Event::Finished {
                                run: id,
                                page: job.page,
                                step: job.step,
                            });
                        }
                    }
                    if run.graph.node_count() == 0 {
                        ended = true;
                        self.link.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Done,
                        });
                    } else {
                        self.runs.push(run);
                    }
                }
            }
            Err((page, reason)) => {
                for run in runs {
                    if done.members.iter().any(|&(id, _)| id == run.id) {
                        ended = true;
                        run.cancel.cancel();
                        self.link.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Failed {
                                page,
                                step: done.step,
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
    /// the GPU's current work ends.
    fn settle(&mut self) {
        let mut wanted: HashSet<Key> = Key::chosen(&self.config.read()).collect();
        for run in &self.runs {
            wanted.extend(Key::chosen(&run.config));
        }
        self.on_gpu(move |models| models.retain(&wanted));
    }

    /// Queues `work` on the GPU thread behind what it's running.
    fn on_gpu(&self, work: impl FnOnce(&mut Models) + Send + 'static) {
        // The GPU thread ends only with the scheduler.
        let _ = self.gpu.send(Box::new(work));
    }

    /// The ready jobs not yet started. A project's translation isn't ready
    /// while its conversation is lent out.
    fn ready(&self) -> Vec<Candidate> {
        self.runs
            .iter()
            .enumerate()
            .flat_map(|(at, run)| {
                run.graph.externals(Incoming).filter_map(move |job| {
                    let node = &run.graph[job];
                    let lent = node.step == Step::Translate && self.lent.contains(&run.project);
                    (!node.running && !lent).then_some(Candidate {
                        run: at,
                        job,
                        step: node.step,
                        priority: Priority {
                            queue: run.queue,
                            run: run.id,
                            page: node.index,
                            step: node.step,
                        },
                    })
                })
            })
            .collect()
    }

    fn admit(&mut self) {
        loop {
            let ready = self.ready();
            // A hosted translation doesn't wait for the GPU, so it isn't
            // counted among the pages in flight.
            let hosted = ready
                .iter()
                .filter(|candidate| {
                    candidate.step == Step::Translate
                        && !models::local(&self.runs[candidate.run].config.translator.model)
                })
                .min_by_key(|candidate| candidate.priority);
            if let Some(&hosted) = hosted {
                self.start(vec![hosted], false);
                continue;
            }
            if self.in_flight >= AHEAD {
                return;
            }
            let Some(&first) = ready.iter().min_by_key(|candidate| candidate.priority) else {
                return;
            };
            let mut chosen = vec![first];
            if first.step == Step::Recognize {
                let model = self.runs[first.run].config.recognizer.model;
                let mut others: Vec<Candidate> = ready
                    .iter()
                    .filter(|other| {
                        other.step == Step::Recognize
                            && other.priority != first.priority
                            && self.runs[other.run].config.recognizer.model == model
                    })
                    .copied()
                    .collect();
                others.sort_by_key(|other| other.priority);
                chosen.extend(others.into_iter().take(RECOGNIZE_PAGES - 1));
            }
            self.start(chosen, true);
        }
    }

    fn start(&mut self, chosen: Vec<Candidate>, counted: bool) {
        let members = chosen
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
        if counted {
            self.in_flight += 1;
        }
        let first = chosen[0];
        let task = Task {
            members,
            priority: first.priority,
            counted,
        };
        let run = &self.runs[first.run];
        let config = Arc::clone(&run.config);
        let link = self.link.clone();
        match first.step {
            Step::Detect => {
                tokio::spawn(perform(Detect::new(config.detector), task, link));
            }
            Step::Recognize => {
                let stages = Recognize::new(config.recognizer.model);
                tokio::spawn(perform(stages, task, link));
            }
            Step::Inpaint => {
                let stages = Inpaint::new(config.inpainter.model, run.area.clone());
                tokio::spawn(perform(stages, task, link));
            }
            Step::Translate => {
                let project = run.project;
                let conversation = self
                    .conversations
                    .remove(&project)
                    .unwrap_or_else(Conversation::new);
                self.lent.insert(project);
                let lent = Lent::new(project, conversation, link.clone());
                let stages = Translate::new(config.translator.clone(), lent);
                tokio::spawn(perform(stages, task, link));
            }
        }
    }

    /// Hands the GPU thread, when it's free, the prepared job with the
    /// highest priority, so hand work goes ahead at the next call.
    fn dispatch(&mut self) {
        if self.gpu_busy {
            return;
        }
        let Some(at) = (0..self.waiting.len()).min_by_key(|&at| self.waiting[at].0) else {
            return;
        };
        let (_, work) = self.waiting.swap_remove(at);
        let link = self.link.clone();
        self.gpu_busy = true;
        self.on_gpu(move |models| {
            work(models);
            link.send(Command::GpuDone);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run on `pages` pages of a project in a folder removed with it.
    fn run(id: u64, queue: Queue, pages: usize, steps: &[Step]) -> (tempfile::TempDir, Run) {
        let dir = tempfile::tempdir().expect("a folder");
        let session = Session::create(dir.path().join("test.khr")).expect("a project");
        let project = session.project().id();
        let pages = (0..pages)
            .map(|_| (PageId(Default::default()), None))
            .collect();
        let plan = Plan {
            queue,
            pages,
            area: None,
        };
        let run = Run::new(
            RunId(id),
            Arc::new(Mutex::new(session)),
            project,
            PipelineConfig::default(),
            plan,
            &steps.iter().copied().collect(),
        );
        (dir, run)
    }

    /// The ready jobs' pages and steps, as the scheduler sees them.
    fn ready(runs: &[Run]) -> Vec<(u64, usize, Step)> {
        let mut ready: Vec<(u64, usize, Step)> = runs
            .iter()
            .flat_map(|run| {
                run.graph.externals(Incoming).map(|job| {
                    let job = &run.graph[job];
                    (run.id.0, job.index, job.step)
                })
            })
            .collect();
        ready.sort();
        ready
    }

    fn finish(run: &mut Run, index: usize, step: Step) {
        let job = run
            .graph
            .node_indices()
            .find(|&job| run.graph[job].index == index && run.graph[job].step == step)
            .expect("the job");
        run.graph.remove_node(job);
    }

    #[test]
    fn a_page_waits_for_detection_and_translation_for_the_page_before() {
        let steps = [Step::Detect, Step::Recognize, Step::Translate];
        let (_dir, run) = run(0, Queue::Pages, 2, &steps);
        let mut runs = vec![run];
        assert_eq!(ready(&runs), [(0, 0, Step::Detect), (0, 1, Step::Detect)]);
        finish(&mut runs[0], 1, Step::Detect);
        finish(&mut runs[0], 1, Step::Recognize);
        assert_eq!(ready(&runs), [(0, 0, Step::Detect)]);
        finish(&mut runs[0], 0, Step::Detect);
        finish(&mut runs[0], 0, Step::Recognize);
        assert_eq!(ready(&runs), [(0, 0, Step::Translate)]);
        finish(&mut runs[0], 0, Step::Translate);
        assert_eq!(ready(&runs), [(0, 1, Step::Translate)]);
    }

    #[test]
    fn hand_work_goes_first_then_the_oldest_page_in_step_order() {
        let pages = Priority {
            queue: Queue::Pages,
            run: RunId(0),
            page: 0,
            step: Step::Detect,
        };
        let hand = Priority {
            queue: Queue::Hand,
            run: RunId(1),
            page: 3,
            step: Step::Translate,
        };
        let later = Priority { page: 1, ..pages };
        let next_step = Priority {
            step: Step::Recognize,
            ..pages
        };
        assert!(hand < pages && pages < next_step && next_step < later);
    }
}
