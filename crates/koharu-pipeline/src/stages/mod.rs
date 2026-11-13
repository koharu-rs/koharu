//! The stages, each an async function over a task's pages that reads top to
//! bottom: CPU work on blocking threads, the model on the GPU thread through
//! `Task::inference`, and each page's result written as one undo step, so the
//! GPU goes on while a page is read or written. Only each stage knows its
//! models; the scheduler sees a stage through `key`.

pub(crate) mod detect;
pub(crate) mod inpaint;
pub(crate) mod recognize;
pub(crate) mod translate;

use std::{sync::Arc, time::Instant};

use anyhow::anyhow;
use image::DynamicImage;
use koharu_scene::{Block, Edit, LayerId, Op, PageId, Project, Session};
use parking_lot::Mutex;
use petgraph::stable_graph::NodeIndex;
use serde_json::{Value, json};
use strum::VariantArray;
use tokio::sync::{OwnedMutexGuard, broadcast, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::info_span;

use crate::{
    Error, Event, PipelineConfig, RunId, Stage,
    gpu::{Gpu, Key, Models, Turn},
    scheduler::Priority,
};

/// A page of a run that a task works on.
#[derive(Clone)]
pub(crate) struct Member {
    pub(crate) run: RunId,
    pub(crate) job: NodeIndex,
    pub(crate) page: PageId,
    /// The page's place in its run, from 0, which traces name it by.
    pub(crate) index: usize,
    /// The blocks a run on blocks works on; every block of the page when
    /// `None`.
    pub(crate) blocks: Option<Arc<[LayerId]>>,
    pub(crate) session: Arc<Mutex<Session>>,
    pub(crate) cancel: CancellationToken,
}

impl Member {
    /// The page's scan, decoded, and the project it was read from; `None`
    /// when the page is gone or has no scan.
    pub(crate) fn scan(&self) -> anyhow::Result<Option<(Project, DynamicImage)>> {
        let (project, bytes) = {
            let session = self.session.lock();
            let project = session.project();
            let Some(background) = project
                .page(self.page)
                .ok()
                .and_then(|page| page.background)
            else {
                return Ok(None);
            };
            let bytes = session.file(&background.file)?;
            (project, bytes)
        };
        let _span = info_span!("decode").entered();
        Ok(Some((project, image::load_from_memory(&bytes)?)))
    }

    /// The blocks of `project`'s page the task works on, in reading order.
    pub(crate) fn blocks<'p>(&self, project: &'p Project) -> Vec<(LayerId, &'p Block)> {
        let mut blocks = project.blocks(self.page).unwrap_or_default();
        if let Some(only) = &self.blocks {
            blocks.retain(|(id, _)| only.contains(id));
        }
        blocks
    }

    /// Applies the ops `write` makes of the project as it is then, which a
    /// layer removed meanwhile isn't in, as one undo step named for `stage`;
    /// nothing once the run stopped.
    pub(crate) async fn apply(
        &self,
        stage: Stage,
        write: impl FnOnce(&Project) -> anyhow::Result<Vec<Op>> + Send + 'static,
    ) -> anyhow::Result<()> {
        let member = self.clone();
        blocking(move || {
            let _span = info_span!("write", %stage, page = member.index + 1).entered();
            let mut session = member.session.lock();
            if member.cancel.is_cancelled() {
                return Ok(());
            }
            let ops = write(&session.project())?;
            if !ops.is_empty() {
                session.apply(Edit {
                    label: stage.to_string(),
                    ops,
                })?;
            }
            Ok(())
        })
        .await
    }
}

/// A stage on one or more pages at once, recognition's on several, and what
/// it reaches: the GPU at its place in line, and the events.
pub(crate) struct Task {
    pub(crate) stage: Stage,
    pub(crate) members: Vec<Member>,
    pub(crate) priority: Priority,
    pub(crate) gpu: Gpu,
    pub(crate) events: broadcast::Sender<Event>,
}

/// A stage's end once its run stopped, which isn't a failure.
#[derive(Debug, thiserror::Error)]
#[error("the run stopped")]
pub(crate) struct Stopped;

impl Task {
    /// `work` on the GPU thread at the task's place in line, with `members`
    /// started then. A stopped run's work is dropped unrun, and a panicking
    /// model fails its run rather than the thread every model is on.
    pub(crate) async fn inference<T: Send + 'static>(
        &self,
        members: &[&Member],
        work: impl FnOnce(&mut Models) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let stage = self.stage;
        let cancels: Vec<CancellationToken> = members.iter().map(|m| m.cancel.clone()).collect();
        let started = self.started_events(members);
        let pages: Vec<usize> = members.iter().map(|member| member.index + 1).collect();
        let events = self.events.clone();
        let queued = Instant::now();
        let (sender, receiver) = oneshot::channel();
        self.gpu.run(Turn::Stage(self.priority), move |models| {
            if cancels.iter().all(CancellationToken::is_cancelled) {
                return;
            }
            let _span =
                info_span!("inference", %stage, ?pages, waited = ?queued.elapsed()).entered();
            for event in started {
                // No one listening isn't a failure.
                let _ = events.send(event);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(models)))
                .unwrap_or_else(|panic| {
                    let message = panic
                        .downcast_ref::<&str>()
                        .map(|message| (*message).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    Err(anyhow!("the model panicked: {message}"))
                });
            let _ = sender.send(result);
        });
        receiver.await.map_err(|_| Stopped)?
    }

    /// Tells the app `members` started, for a stage that doesn't wait for the
    /// GPU.
    pub(crate) fn started(&self, members: &[&Member]) {
        for event in self.started_events(members) {
            let _ = self.events.send(event);
        }
    }

    fn started_events(&self, members: &[&Member]) -> Vec<Event> {
        members
            .iter()
            .map(|member| Event::Started {
                run: member.run,
                page: member.page,
                stage: self.stage,
            })
            .collect()
    }
}

/// What a stage needs besides its pages: the run's settings, its painted
/// area, and for a translation, its project's conversation.
pub(crate) struct Inputs {
    pub(crate) config: Arc<PipelineConfig>,
    pub(crate) area: Option<Arc<inpaint::AreaWork>>,
    pub(crate) conversation: Option<OwnedMutexGuard<translate::Conversation>>,
}

/// Runs `task`'s stage: its failure, if any, as the run reports it.
pub(crate) async fn run(task: Task, inputs: Inputs) -> Result<(), String> {
    let stage = task.stage;
    let config = &inputs.config;
    let result = match stage {
        Stage::Detect => detect::detect(&task, config).await,
        Stage::Recognize => recognize::recognize(&task, config).await,
        Stage::Inpaint => inpaint::inpaint(&task, config, inputs.area).await,
        Stage::Translate => {
            let conversation = inputs
                .conversation
                .expect("a translation holds its conversation");
            translate::translate(&task, config, conversation).await
        }
    };
    match result {
        Err(error) if !error.is::<Stopped>() => {
            let error = anyhow::Error::from(Error::Stage {
                stage,
                source: error,
            });
            Err(format!("{error:#}"))
        }
        _ => Ok(()),
    }
}

/// The model of Koharu's that `config` has `stage` load; `None` for a hosted
/// translation, which loads none and doesn't wait for the GPU.
pub(crate) fn key(stage: Stage, config: &PipelineConfig) -> Option<Key> {
    let name = match stage {
        Stage::Detect => config.detector.model.to_string(),
        Stage::Recognize => config.recognizer.model.to_string(),
        Stage::Inpaint => config.inpainter.model.to_string(),
        Stage::Translate => {
            let translator = &config.translator;
            // A `vendor/model` reference: koharu-llm runs only its own
            // vendor's models in this process.
            let (vendor, _) = translator.model.split_once('/')?;
            if vendor != koharu_llm::KOHARU {
                return None;
            }
            // With its projector, it's another load.
            match translator.vision {
                true => format!("{} reading images", translator.model),
                false => translator.model.clone(),
            }
        }
    };
    Some(Key { stage, name })
}

/// Every model of Koharu's that `config` chooses, which a switch of settings
/// keeps.
pub(crate) fn chosen(config: &PipelineConfig) -> impl Iterator<Item = Key> + '_ {
    Stage::VARIANTS
        .iter()
        .filter_map(|&stage| key(stage, config))
}

/// `work` on a blocking thread, off the async ones.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(work).await?
}

/// A model's record of what it did, in C2PA's vocabulary.
pub(crate) fn action(action: &str, model: &str, parameters: Option<Value>) -> Value {
    let mut value = json!({
        "action": action,
        "softwareAgent": {"name": model, "version": env!("CARGO_PKG_VERSION")},
        "digitalSourceType": "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia",
    });
    if let Some(parameters) = parameters {
        value["parameters"] = parameters;
    }
    value
}
