//! A run: the lane, the work beside it, and the edit applier.

use std::{sync::Arc, time::Instant};

use anyhow::{Result, bail};
use koharu_scene::{Change, Edit, PageId, StyleId};
use tokio::{
    sync::{Mutex, Semaphore, mpsc},
    task::JoinSet,
};

use crate::{
    Host, Inpainting, OcrModel, Outcome as Ended, PipelineConfig, Progress, Report, Run, State,
    Step, Target,
    plan::{Job, Outcome, Place, Plan},
    residency::{IDLE, Model, Residency, TRANSLATION_SIZE},
    steps::{self, Context, LINE_HEIGHT, Prepared, Scope, TYPESETTER},
};

/// Hosted requests in flight at once, Marker's default.
const NETWORK: usize = 3;

pub struct Pipeline {
    residency: Arc<Mutex<Residency>>,
    /// Runs take turns on the one lane.
    turn: Mutex<()>,
    idle: tokio::task::JoinHandle<()>,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.idle.abort();
    }
}

enum Event {
    Computed(Job, Result<Prepared<Edit>>),
    Applied(Job, koharu_scene::Result<Change>),
}

impl Pipeline {
    /// A pipeline on `devices`. It must be made inside a multi-threaded tokio
    /// runtime, where it unloads idle models and runs blocking model calls in
    /// place.
    pub fn new(devices: &'static koharu_runtime::Devices) -> Self {
        let residency = Arc::new(Mutex::new(Residency::new(devices)));
        let idle = tokio::spawn(expire(Arc::clone(&residency)));
        Self {
            residency,
            turn: Mutex::new(()),
            idle,
        }
    }

    pub async fn run<H: Host + 'static>(&self, host: Arc<H>, run: Run) -> Result<Report> {
        let _turn = self.turn.lock().await;
        let config: PipelineConfig = koharu_config::get()?;
        let (pages, scope) = pages(&*host, &run.target)?;
        let selection = &config.translation.model;
        let hosted = if run.steps.contains(Step::Translation)
            && selection.provider != koharu_llm::Provider::Local
        {
            Some(Arc::new(
                koharu_llm::model(selection, TRANSLATION_SIZE).await?,
            ))
        } else {
            None
        };
        let dialogue = if run.steps.contains(Step::Typesetting) {
            Some(dialogue(&*host)?)
        } else {
            None
        };
        let context = Arc::new(Context {
            project: host.read(|session| session.id()),
            config,
            redo: run.redo,
            dialogue,
        });
        let mut plan = Plan::new(pages.len(), run.steps, |step| place(step, &context.config));
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let (edits, edits_rx) = mpsc::unbounded_channel();
        let applier = tokio::spawn(apply(Arc::clone(&host), edits_rx, events_tx.clone()));
        let network = Arc::new(Semaphore::new(NETWORK));
        let mut work = JoinSet::new();
        // Jobs computing or waiting for the applier.
        let mut in_flight = 0usize;
        let run_ = Runner {
            host: &*host,
            pages: &pages,
            scope: &scope,
            context: &context,
        };

        loop {
            while let Ok(event) = events.try_recv() {
                run_.settle(&mut plan, &edits, &mut in_flight, event);
            }
            if run.cancel.is_cancelled() || plan.finished() {
                break;
            }

            for (job, place) in plan.off_lane() {
                let Some(work_) = run_.start(&mut plan, job) else {
                    continue;
                };
                in_flight += 1;
                let (context, hosted, events, network, cancel) = (
                    Arc::clone(&context),
                    hosted.clone(),
                    events_tx.clone(),
                    Arc::clone(&network),
                    run.cancel.clone(),
                );
                work.spawn(async move {
                    let _permit = match place {
                        Place::Network => Some(network.acquire_owned().await),
                        Place::Cpu | Place::Lane(_) => None,
                    };
                    let edit = steps::off_lane(work_, &context, hosted.as_deref());
                    if let Some(edit) = cancel.run_until_cancelled(edit).await {
                        let _ = events.send(Event::Computed(job, edit.map(Prepared::Work)));
                    }
                });
            }

            let batch = {
                let residency = self.residency.lock().await;
                plan.next_batch(|model| residency.is_loaded(model), Model::batch)
            };
            if let Some((model, jobs)) = batch {
                let mut started = Vec::new();
                for job in jobs {
                    if let Some(work_) = run_.start(&mut plan, job) {
                        started.push((job, work_));
                    }
                }
                if started.is_empty() {
                    continue;
                }
                let (jobs, works): (Vec<Job>, Vec<_>) = started.into_iter().unzip();
                let mut residency = self.residency.lock().await;
                let results = match residency.get(&model, |model| plan.next_use(model)).await {
                    Ok(loaded) => {
                        let batch = steps::on_lane(loaded, works, &context);
                        match run.cancel.run_until_cancelled(batch).await {
                            Some(results) => results,
                            None => break,
                        }
                    }
                    Err(error) => {
                        let reason = format!("{error:#}");
                        jobs.iter()
                            .map(|_| Err(anyhow::anyhow!(reason.clone())))
                            .collect()
                    }
                };
                drop(residency);
                for (job, result) in jobs.into_iter().zip(results) {
                    in_flight += 1;
                    run_.settle(
                        &mut plan,
                        &edits,
                        &mut in_flight,
                        Event::Computed(job, result.map(Prepared::Work)),
                    );
                }
                continue;
            }

            if in_flight == 0 {
                bail!("the run has work left that nothing can start");
            }
            tokio::select! {
                Some(event) = events.recv() => run_.settle(&mut plan, &edits, &mut in_flight, event),
                () = run.cancel.cancelled() => {}
            }
        }

        // Edits already computed still apply: a finished step stays.
        work.abort_all();
        drop((edits, events_tx));
        applier.await?;
        while let Some(event) = events.recv().await {
            if let Event::Applied(..) = event {
                run_.settle(
                    &mut plan,
                    &mpsc::unbounded_channel().0,
                    &mut in_flight,
                    event,
                );
            }
        }
        let outcome = |(job, reason): (Job, &str)| Ended {
            page: pages[job.page],
            step: job.step,
            reason: reason.to_owned(),
        };
        let (done, total) = plan.progress();
        Ok(Report {
            failures: plan.failures().map(outcome).collect(),
            skips: plan.skips().map(outcome).collect(),
            done,
            total,
            cancelled: run.cancel.is_cancelled(),
        })
    }
}

/// What every job of one run shares.
struct Runner<'a, H> {
    host: &'a H,
    pages: &'a [PageId],
    scope: &'a Scope,
    context: &'a Context,
}

impl<H: Host> Runner<'_, H> {
    /// Reads `job`'s work from the project, or settles it if it has none.
    fn start(&self, plan: &mut Plan<Model>, job: Job) -> Option<steps::Work> {
        self.report(plan, job, State::Started);
        let page = self.pages[job.page];
        let read = self
            .host
            .read(|session| steps::read(session, job.step, page, self.scope, self.context));
        let outcome = match read {
            Ok(Prepared::Work(work)) => return Some(work),
            Ok(Prepared::Skip(reason)) => Outcome::Skipped(reason),
            Err(error) => Outcome::Failed(format!("{error:#}")),
        };
        self.finish(plan, job, outcome);
        None
    }

    fn settle(
        &self,
        plan: &mut Plan<Model>,
        edits: &mpsc::UnboundedSender<(Job, Edit)>,
        in_flight: &mut usize,
        event: Event,
    ) {
        let (job, outcome) = match event {
            Event::Computed(job, Ok(Prepared::Work(edit))) => {
                if edits.send((job, edit)).is_ok() {
                    return;
                }
                (
                    job,
                    Outcome::Failed("the run ended before its edit applied".to_owned()),
                )
            }
            Event::Computed(job, Ok(Prepared::Skip(reason))) => (job, Outcome::Skipped(reason)),
            Event::Computed(job, Err(error)) => (job, Outcome::Failed(format!("{error:#}"))),
            Event::Applied(job, Ok(_)) => (job, Outcome::Done),
            Event::Applied(job, Err(koharu_scene::Error::Stale { .. })) => (job, Outcome::Stale),
            Event::Applied(job, Err(error)) => (job, Outcome::Failed(error.to_string())),
        };
        *in_flight = in_flight.saturating_sub(1);
        self.finish(plan, job, outcome);
    }

    fn finish(&self, plan: &mut Plan<Model>, job: Job, outcome: Outcome) {
        let state = match &outcome {
            Outcome::Done => State::Finished,
            Outcome::Skipped(reason) => State::Skipped(reason.clone()),
            Outcome::Failed(reason) => State::Failed(reason.clone()),
            Outcome::Stale => State::Started,
        };
        if plan.finish(job, outcome) {
            self.report(plan, job, state);
        }
    }

    fn report(&self, plan: &Plan<Model>, job: Job, state: State) {
        let (done, total) = plan.progress();
        self.host.report(Progress {
            page: self.pages[job.page],
            step: job.step,
            state,
            done,
            total,
        });
    }
}

/// Applies finished edits through the host, one at a time in the order they
/// finish, while the lane runs on.
async fn apply<H: Host>(
    host: Arc<H>,
    mut edits: mpsc::UnboundedReceiver<(Job, Edit)>,
    events: mpsc::UnboundedSender<Event>,
) {
    while let Some((job, edit)) = edits.recv().await {
        let _ = events.send(Event::Applied(job, host.apply(edit)));
    }
}

/// Unloads models once they've been idle long enough.
async fn expire(residency: Arc<Mutex<Residency>>) {
    loop {
        let now = Instant::now();
        let next = residency.lock().await.expire(now).unwrap_or(now + IDLE);
        tokio::time::sleep_until(tokio::time::Instant::from_std(next)).await;
    }
}

/// The project's "Dialogue" style that typesetting made, made now if there is
/// none, so pages typeset side by side share one.
pub(crate) fn dialogue<H: Host>(host: &H) -> Result<StyleId> {
    let styles = host.read(|session| session.project().map(|project| project.styles))?;
    let made = styles.iter().find(|style| {
        style.style.name == "Dialogue"
            && style
                .origin
                .iter()
                .any(|record| record.action.software_agent.name == TYPESETTER)
    });
    if let Some(style) = made {
        return Ok(style.id);
    }
    let id = StyleId::new();
    let style = serde_json::from_value(serde_json::json!({
        "name": "Dialogue",
        "style": {"align": "center", "line_height": LINE_HEIGHT},
    }))?;
    host.apply(Edit {
        project: host.read(|session| session.id()),
        label: Step::Typesetting.to_string(),
        author: koharu_scene::Author::Model {
            action: koharu_scene::Action {
                action: "c2pa.created".to_owned(),
                software_agent: koharu_scene::SoftwareAgent {
                    name: TYPESETTER.to_owned(),
                    version: None,
                },
                digital_source_type: None,
                parameters: serde_json::Map::new(),
            },
        },
        reads: Vec::new(),
        ops: vec![koharu_scene::Op::AddTextStyle {
            id: Some(id.into()),
            style,
            ingredients: Vec::new(),
        }],
    })?;
    Ok(id)
}

/// The run's pages, in order, and the part of each it covers.
fn pages<H: Host>(host: &H, target: &Target) -> Result<(Vec<PageId>, Scope)> {
    Ok(match target {
        Target::Pages(pages) => (pages.clone(), Scope::Page),
        Target::Layers(layers) => {
            let mut pages: Vec<PageId> = Vec::new();
            for layer in layers {
                let page = host.read(|session| session.layer(*layer).map(|answer| answer.page))?;
                if !pages.contains(&page) {
                    pages.push(page);
                }
            }
            (pages, Scope::Layers(layers.clone()))
        }
        Target::Area { page, rect } => (vec![*page], Scope::Area(*rect)),
    })
}

/// Where `step` runs under `config`.
fn place(step: Step, config: &PipelineConfig) -> Place<Model> {
    match step {
        Step::Detection => Place::Lane(Model::Layout),
        Step::Ocr => Place::Lane(match config.ocr {
            OcrModel::PaddleOcrVl1_6 => Model::PaddleOcrVl,
            OcrModel::MangaOcr => Model::MangaOcr,
            OcrModel::BaberuOcr => Model::BaberuOcr,
            OcrModel::HayaiOcr => Model::HayaiOcr,
        }),
        Step::Translation => {
            let selection = &config.translation.model;
            match selection.provider {
                koharu_llm::Provider::Local => Place::Lane(Model::Translator {
                    model: selection.model.clone(),
                    quantization: selection.quantization.clone(),
                }),
                _ => Place::Network,
            }
        }
        Step::Typesetting => Place::Cpu,
        Step::Inpainting => Place::Lane(match config.inpainting {
            Inpainting::LaMa => Model::LaMa,
            Inpainting::AotInpainting => Model::Aot,
            Inpainting::Flux2Klein => Model::Flux2Klein,
            Inpainting::RoremMixed => Model::RoremMixed,
        }),
    }
}
