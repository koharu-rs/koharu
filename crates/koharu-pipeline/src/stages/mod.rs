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
use koharu_scene::{Block, Content, Edit, LayerId, Op, PageId, Project, Session};
use parking_lot::Mutex;
use petgraph::stable_graph::NodeIndex;
use serde_json::{Value, json};
use strum::VariantArray;
use tokio::sync::{OwnedMutexGuard, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::info_span;

use crate::{
    Error, Event, PipelineConfig, RunId, Stage,
    event::Events,
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

    /// The blocks of `project`'s page the task works on, in reading order,
    /// but those locked or in a locked layer, which a run leaves as they are.
    pub(crate) fn blocks<'p>(&self, project: &'p Project) -> Vec<(LayerId, &'p Block)> {
        let mut blocks = project.blocks(self.page).unwrap_or_default();
        blocks.retain(|(id, _)| project.locked_at(*id).is_none());
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
        tokio::task::spawn_blocking(move || {
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
        .await?
    }
}

/// Block `id` as `project` holds it when a stage writes, if it's still there
/// and neither it nor a layer it's in is locked: a lock can come on while the
/// stage runs, and the scene refuses a page's whole edit for one locked write.
pub(crate) fn writable(project: &Project, id: LayerId) -> Option<&Block> {
    if project.locked_at(id).is_some() {
        return None;
    }
    match &project.layer(id).ok()?.content {
        Content::Block(block) => Some(block),
        _ => None,
    }
}

/// A stage on one or more pages at once, recognition's on several, and what
/// it reaches: the GPU at its place in line, and the events.
pub(crate) struct Task {
    pub(crate) stage: Stage,
    pub(crate) members: Vec<Member>,
    pub(crate) priority: Priority,
    pub(crate) gpu: Gpu,
    pub(crate) events: Events,
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
                events.send(event);
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
            self.events.send(event);
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

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use anyhow::{Context as _, Result};
    use koharu_scene::{Created, Edit, LayerId, Session};
    use parking_lot::Mutex;
    use petgraph::stable_graph::NodeIndex;
    use serde_json::{Value, json};
    use tokio_util::sync::CancellationToken;

    use super::{Member, writable};
    use crate::RunId;

    /// A project of one 400-pixel page holding `layers`, each `(label,
    /// parent label, layer)`, and the labelled layers' ids in order.
    pub(crate) fn page(
        dir: &tempfile::TempDir,
        layers: &[(&str, Option<&str>, Value)],
    ) -> Result<(Session, Vec<LayerId>)> {
        let mut session = Session::create(dir.path().join("test.khr"))?;
        let mut ops = vec![
            json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 400}}),
        ];
        for (at, (label, parent, layer)) in layers.iter().enumerate() {
            // Last among its siblings.
            let index = layers[..at]
                .iter()
                .filter(|(_, other, _)| other == parent)
                .count();
            let mut op = json!({"op": "add_layer", "id": format!("${label}"), "page": "$p", "index": index, "layer": layer});
            if let Some(parent) = parent {
                op["parent"] = json!(format!("${parent}"));
            }
            ops.push(op);
        }
        let change = session.apply(Edit {
            label: "test".to_owned(),
            ops: ops
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()?,
        })?;
        let ids = layers
            .iter()
            .map(|(label, ..)| {
                change
                    .created
                    .iter()
                    .find_map(|created| match created {
                        Created::Layer {
                            id,
                            label: Some(found),
                        } if found == label => Some(*id),
                        _ => None,
                    })
                    .with_context(|| format!("no layer {label}"))
            })
            .collect::<Result<_>>()?;
        Ok((session, ids))
    }

    /// A panel or a balloon, by `kind`, 200 by 160 pixels at the page's
    /// corner.
    pub(crate) fn holder(kind: &str) -> Value {
        json!({"kind": kind, "outline": "M0 0 L200 0 L200 160 L0 160 Z"})
    }

    /// A block with its source's box at `x`.
    pub(crate) fn block(x: f64) -> Value {
        json!({"kind": "block", "source": {
            "content": "テスト", "language": "ja",
            "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 40.0, "height": 120.0}}},
        }})
    }

    /// Locks `layer`.
    pub(crate) fn lock(session: &mut Session, layer: LayerId) -> Result<()> {
        session.apply(Edit {
            label: "lock".to_owned(),
            ops: vec![serde_json::from_value(
                json!({"op": "update_layer", "layer": layer, "patch": {"locked": true}}),
            )?],
        })?;
        Ok(())
    }

    #[test]
    fn a_run_leaves_out_a_locked_block_and_one_in_a_locked_layer() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("balloon", None, holder("balloon")),
                ("inside", Some("balloon"), block(20.0)),
                ("locked", None, block(120.0)),
                ("free", None, block(220.0)),
            ],
        )?;
        let &[balloon, inside, locked, free] = &ids[..] else {
            anyhow::bail!("four layers");
        };
        lock(&mut session, balloon)?;
        lock(&mut session, locked)?;
        let project = session.project();
        let page = project.pages().next().context("a page")?.id;
        let session = Arc::new(Mutex::new(session));
        let worked = |blocks: Option<Arc<[LayerId]>>| -> Vec<LayerId> {
            let member = Member {
                run: RunId(1),
                job: NodeIndex::new(0),
                page,
                index: 0,
                blocks,
                session: session.clone(),
                cancel: CancellationToken::new(),
            };
            member
                .blocks(&project)
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        };
        assert_eq!(worked(None), [free]);
        assert_eq!(worked(Some(Arc::from([inside, locked, free]))), [free]);
        assert!(writable(&project, inside).is_none());
        assert!(writable(&project, locked).is_none());
        assert!(writable(&project, free).is_some());
        Ok(())
    }
}
