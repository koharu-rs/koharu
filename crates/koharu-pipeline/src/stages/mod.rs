//! The stages, each an async function over a task's pages that reads top to
//! bottom: CPU work on blocking threads, the model on the GPU thread through
//! `Task::inference`, and each page's result written as one undo step, so the
//! GPU goes on while a page is read or written. Only each stage knows its
//! models; the scheduler sees a stage through `key`.

pub(crate) mod detect;
pub(crate) mod inpaint;
pub(crate) mod recognize;
pub(crate) mod translate;

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::anyhow;
use image::{DynamicImage, GrayImage, Luma};
use koharu_runtime::PinnedFile;
use koharu_scene::{Block, Content, Edit, LayerId, Op, PageId, Project, Rect, Session};
use kurbo::{BezPath, ParamCurve as _};
use parking_lot::Mutex;
use petgraph::stable_graph::NodeIndex;
use serde_json::{Value, json};
use strum::VariantArray;
use tokio::sync::{OwnedMutexGuard, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::info_span;

use self::{detect::Detector, inpaint::Inpainter, recognize::Recognizer};
use crate::{
    Event, PipelineConfig, RunId, Stage,
    event::Events,
    gpu::{Gpu, Key, Models, Turn},
    scheduler::Priority,
};

/// A page (or project metadata when `page` is `None`) of a run that a task works on.
#[derive(Clone)]
pub(crate) struct Member {
    pub(crate) run: RunId,
    pub(crate) job: NodeIndex,
    pub(crate) page: Option<PageId>,
    /// The page's place in its run, from 0, which traces name it by.
    pub(crate) index: usize,
    /// The blocks a run on blocks works on; every block of the page when
    /// `None`.
    pub(crate) blocks: Option<Arc<[LayerId]>>,
    pub(crate) session: Arc<Mutex<Session>>,
    pub(crate) cancel: CancellationToken,
    /// Whether the app was told the stage started on the page, which it's
    /// told once, however many turns the stage takes.
    pub(crate) started: Arc<AtomicBool>,
}

impl Member {
    pub(crate) fn page_id(&self) -> PageId {
        self.page.expect("a page stage has a page id")
    }

    /// The page's scan, decoded, and the project it was read from; `None`
    /// when the page is gone or has no scan.
    pub(crate) fn scan(&self) -> anyhow::Result<Option<(Project, DynamicImage)>> {
        let Some(page_id) = self.page else {
            return Ok(None);
        };
        let (project, bytes) = {
            let session = self.session.lock();
            let project = session.project();
            let Some(background) = project.page(page_id).ok().and_then(|page| page.background)
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
        let Some(page_id) = self.page else {
            return Vec::new();
        };
        let mut blocks = project.blocks(page_id).unwrap_or_default();
        blocks.retain(|(id, _)| project.locked_at(*id).is_none());
        if let Some(only) = &self.blocks {
            blocks.retain(|(id, _)| only.contains(id));
        }
        blocks
    }

    /// Applies the ops `write` makes of the session as it is then, whose
    /// project a layer removed meanwhile isn't in, as one undo step named for
    /// `stage`; nothing once the run stopped.
    pub(crate) async fn apply(
        &self,
        stage: Stage,
        write: impl FnOnce(&Session) -> anyhow::Result<Vec<Op>> + Send + 'static,
    ) -> anyhow::Result<()> {
        let member = self.clone();
        tokio::task::spawn_blocking(move || {
            let _span =
                info_span!("write", %stage, page = member.page.map(|_| member.index + 1)).entered();
            let mut session = member.session.lock();
            if member.cancel.is_cancelled() {
                return Ok(());
            }
            let ops = write(&session)?;
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
    pub(crate) model: Option<String>,
    pub(crate) members: Vec<Member>,
    pub(crate) priority: Priority,
    pub(crate) gpu: Gpu,
    pub(crate) events: Events,
}

/// A stage's failure on one page alone, which its run goes past.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct Failed(pub(crate) anyhow::Error);

/// A stage's end once its run stopped, which isn't a failure.
#[derive(Debug, thiserror::Error)]
#[error("the run stopped")]
pub(crate) struct Stopped;

impl Task {
    /// `work` on the GPU thread at the task's place in line, with `members`
    /// started then, on their first turn. A stopped run's work is dropped
    /// unrun, and a panicking model fails its run rather than the thread every
    /// model is on.
    pub(crate) async fn inference<T: Send + 'static>(
        &self,
        members: &[&Member],
        work: impl FnOnce(&mut Models) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let stage = self.stage;
        let pairs: Vec<(CancellationToken, Event)> = members
            .iter()
            .zip(self.started_events(members))
            .map(|(member, event)| (member.cancel.clone(), event))
            .collect();
        let pages: Vec<usize> = members.iter().map(|member| member.index + 1).collect();
        let events = self.events.clone();
        let queued = Instant::now();
        let (sender, receiver) = oneshot::channel();
        self.gpu.run(Turn::Stage(self.priority), move |models| {
            if pairs.iter().all(|(cancel, _)| cancel.is_cancelled()) {
                return;
            }
            let _span =
                info_span!("inference", %stage, ?pages, waited = ?queued.elapsed()).entered();
            for (cancel, event) in pairs {
                if !cancel.is_cancelled() {
                    events.send(event);
                }
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

    /// Runs `work` on the GPU thread at the task's place in line without
    /// emitting `Event::Started` (used when a batched stage loads a model once
    /// before starting individual sub-batches).
    pub(crate) async fn load_on_gpu<T: Send + 'static>(
        &self,
        members: &[&Member],
        work: impl FnOnce(&mut Models) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let cancels: Vec<CancellationToken> = members.iter().map(|m| m.cancel.clone()).collect();
        let (sender, receiver) = oneshot::channel();
        self.gpu.run(Turn::Stage(self.priority), move |models| {
            if !cancels.is_empty() && cancels.iter().all(CancellationToken::is_cancelled) {
                return;
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
        for (member, event) in members.iter().zip(self.started_events(members)) {
            if !member.cancel.is_cancelled() {
                self.events.send(event);
            }
        }
    }

    /// The Started events of `members` not yet started, which are then.
    fn started_events(&self, members: &[&Member]) -> Vec<Event> {
        members
            .iter()
            .filter(|member| !member.started.swap(true, Ordering::Relaxed))
            .map(|member| Event::Started {
                run: member.run,
                page: member.page,
                stage: self.stage,
                model: self.model.clone(),
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

/// Runs `task`'s stage; a stopped run's end isn't a failure. Returns any
/// per-member page-level failures inside a multi-page task whose other members
/// succeeded.
pub(crate) async fn run(task: Task, inputs: Inputs) -> anyhow::Result<Vec<(NodeIndex, String)>> {
    let config = &inputs.config;
    let result = match task.stage {
        Stage::Detect => detect::detect(&task, config).await.map(|()| Vec::new()),
        Stage::Recognize => recognize::recognize(&task, config)
            .await
            .map(|()| Vec::new()),
        Stage::Inpaint => inpaint::inpaint(&task, config, inputs.area)
            .await
            .map(|()| Vec::new()),
        Stage::Translate => {
            let conversation = inputs
                .conversation
                .expect("a translation holds its conversation");
            match task.members[0].page {
                Some(_) => translate::translate(&task, config, conversation).await,
                None => translate::metadata(&task, config, conversation)
                    .await
                    .map(|()| Vec::new()),
            }
        }
    };
    match result {
        Err(error) if error.is::<Stopped>() => Ok(Vec::new()),
        result => result,
    }
}

/// The model of Koharu's that `config` has `stage` load; `None` for a hosted
/// translation, which loads none and doesn't wait for the GPU.
pub(crate) fn key(stage: Stage, config: &PipelineConfig) -> Option<Key> {
    let (name, id, vision) = match stage {
        Stage::Detect => (
            config.detector.model.to_string(),
            config.detector.model.id().to_owned(),
            false,
        ),
        Stage::Recognize => (
            config.recognizer.model.to_string(),
            config.recognizer.model.id().to_owned(),
            false,
        ),
        Stage::Inpaint => (
            config.inpainter.model.to_string(),
            config.inpainter.model.id().to_owned(),
            false,
        ),
        Stage::Translate => {
            let translator = &config.translator;
            // A `vendor/model` reference: koharu-llm runs only its own
            // vendor's models in this process.
            let (vendor, _) = translator.model.split_once('/')?;
            if vendor != koharu_llm::KOHARU {
                return None;
            }
            // With its projector, it's another load.
            let name = match translator.vision {
                true => format!("{} reading images", translator.model),
                false => translator.model.clone(),
            };
            (name, translator.model.clone(), translator.vision)
        }
    };
    Some(Key {
        stage,
        name,
        id,
        vision,
    })
}

/// Resolves the pinned Hugging Face files for a local model [`Key`].
pub(crate) fn files_for_key(key: &Key) -> anyhow::Result<Vec<PinnedFile>> {
    match key.stage {
        Stage::Detect => Ok(Detector::ALL
            .iter()
            .find(|model| model.id() == key.id)
            .map_or_else(Vec::new, |model| model.files())),
        Stage::Recognize => Ok(Recognizer::ALL
            .iter()
            .find(|model| model.id() == key.id)
            .map_or_else(Vec::new, |model| model.files())),
        Stage::Inpaint => Ok(Inpainter::ALL
            .iter()
            .find(|model| model.id() == key.id)
            .map_or_else(Vec::new, |model| model.files())),
        Stage::Translate => koharu_llm::local_files(&key.id, key.vision),
    }
}

/// Display model name for `stage` under `config`, including hosted
/// translators so activity UI can show which model is active.
pub(crate) fn model_name(stage: Stage, config: &PipelineConfig) -> Option<String> {
    key(stage, config).map(|k| k.name).or_else(|| {
        (stage == Stage::Translate && !config.translator.model.trim().is_empty())
            .then(|| config.translator.model.clone())
    })
}

/// A local model whose pinned files are missing on disk and must be downloaded
/// before the scheduler admits the run to the GPU queue.
#[derive(Clone, Debug)]
pub(crate) struct NeededModel {
    pub(crate) model_id: String,
    pub(crate) model_name: String,
    pub(crate) stage: Stage,
    pub(crate) files: Vec<PinnedFile>,
}

/// Resolves the local models in `stages` whose pinned files are not yet on
/// disk, keyed by [`crate::vram::model_key`] so `vision: true` and `false`
/// never deduplicate against one another.
pub(crate) fn files_needed(
    stages: &BTreeSet<Stage>,
    config: &PipelineConfig,
) -> anyhow::Result<Vec<NeededModel>> {
    files_needed_with(stages, config, PinnedFile::exists)
}

pub(crate) fn files_needed_with(
    stages: &BTreeSet<Stage>,
    config: &PipelineConfig,
    exists: impl Fn(&PinnedFile) -> bool,
) -> anyhow::Result<Vec<NeededModel>> {
    let mut needed = Vec::new();
    for &stage in stages {
        let Some(key) = key(stage, config) else {
            continue;
        };
        let files = files_for_key(&key)?;
        if files.is_empty() || files.iter().all(&exists) {
            continue;
        }
        needed.push(NeededModel {
            model_id: crate::vram::model_key(key.stage, &key.id, key.vision),
            model_name: key.name,
            stage,
            files,
        });
    }
    Ok(needed)
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

/// The pixels of `rect` whose centres `outline` holds, as `Shape::contains`
/// tells them, a row at a time.
pub(crate) fn held(outline: &BezPath, rect: Rect) -> GrayImage {
    let mut lines = BezPath::new();
    kurbo::flatten(outline, 0.1, |element| lines.push(element));
    // Where the outline crosses each row's middle, and which way.
    let mut crossings = vec![Vec::new(); rect.height as usize];
    let row = |y: f64| (y - f64::from(rect.y) - 0.5).ceil().max(0.0) as usize;
    for line in lines.segments() {
        let (start, end) = (line.start(), line.end());
        let (top, bottom, way) = match start.y < end.y {
            true => (start, end, 1),
            false => (end, start, -1),
        };
        for at in row(top.y)..row(bottom.y).min(crossings.len()) {
            let y = f64::from(rect.y) + at as f64 + 0.5;
            let x = top.x + (bottom.x - top.x) * (y - top.y) / (bottom.y - top.y);
            crossings[at].push((x - f64::from(rect.x), way));
        }
    }
    let mut pixels = GrayImage::new(rect.width, rect.height);
    for (y, crossings) in crossings.iter_mut().enumerate() {
        crossings.sort_by(|(left, _), (right, _)| left.total_cmp(right));
        let mut winding = 0;
        for pair in crossings.windows(2) {
            winding += pair[0].1;
            if winding != 0 {
                let column = |x: f64| ((x - 0.5).ceil().max(0.0) as u32).min(rect.width);
                for x in column(pair[0].0)..column(pair[1].0) {
                    pixels.put_pixel(x, y as u32, Luma([255]));
                }
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use koharu_scene::Edit;
    use serde_json::json;

    use super::*;
    use crate::{
        gpu::{Gpu, GpuSnapshot},
        scheduler::{Mode, Priority},
        vram::VramStore,
    };

    #[test]
    fn files_needed_skips_hosted_checks_missing_and_separates_vision() {
        let stages = BTreeSet::from([
            Stage::Detect,
            Stage::Recognize,
            Stage::Inpaint,
            Stage::Translate,
        ]);

        // Hosted translator skips Translate, while local stages report missing files.
        let mut hosted = PipelineConfig::default();
        hosted.translator.model = "openai/gpt-5".to_owned();
        let needed = files_needed_with(&stages, &hosted, |_| false).unwrap();
        assert_eq!(needed.len(), 3);
        assert!(needed.iter().all(|m| m.stage != Stage::Translate));

        // When all files exist, nothing is needed.
        let none = files_needed_with(&stages, &PipelineConfig::default(), |_| true).unwrap();
        assert!(none.is_empty());

        // Vision true/false produce distinct `model_id`s and file lists.
        let mut no_vision = PipelineConfig::default();
        no_vision.translator.vision = false;
        let mut with_vision = PipelineConfig::default();
        with_vision.translator.vision = true;
        let m_no_vis =
            files_needed_with(&BTreeSet::from([Stage::Translate]), &no_vision, |_| false)
                .unwrap()
                .pop()
                .unwrap();
        let m_vis = files_needed_with(&BTreeSet::from([Stage::Translate]), &with_vision, |_| false)
            .unwrap()
            .pop()
            .unwrap();
        assert_ne!(m_no_vis.model_id, m_vis.model_id);
        assert!(m_vis.model_id.ends_with(":vision"));
        assert!(m_vis.files.len() > m_no_vis.files.len());

        // Invalid local koharu/* model fails closed.
        let mut bad = PipelineConfig::default();
        bad.translator.model = "koharu/does-not-exist".to_owned();
        assert!(files_needed_with(&BTreeSet::from([Stage::Translate]), &bad, |_| false).is_err());
    }

    #[tokio::test]
    async fn inference_skips_started_for_cancelled_member_in_shared_batch() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        session.apply(Edit {
            label: "test".to_owned(),
            ops: vec![serde_json::from_value(json!({
                "op": "add_page", "index": 0, "size": {"width": 100, "height": 100},
            }))?],
        })?;
        let page = session.project().pages().next().unwrap().id;
        let session = Arc::new(Mutex::new(session));

        let events = Events::new();
        let mut rx = events.subscribe();
        let (gpu, _snapshot) = Gpu::spawn(tokio::runtime::Handle::current(), Arc::default());
        let cancel_a = CancellationToken::new();
        cancel_a.cancel();
        let cancel_b = CancellationToken::new();

        let member_a = Member {
            run: RunId(1),
            job: NodeIndex::new(0),
            page: Some(page),
            index: 0,
            blocks: None,
            session: Arc::clone(&session),
            cancel: cancel_a,
            started: Default::default(),
        };
        let member_b = Member {
            run: RunId(2),
            job: NodeIndex::new(0),
            page: Some(page),
            index: 0,
            blocks: None,
            session,
            cancel: cancel_b,
            started: Default::default(),
        };
        let task = Task {
            stage: Stage::Recognize,
            model: Some("hayai-ocr".to_owned()),
            members: vec![member_a.clone(), member_b.clone()],
            priority: Priority::test_new(Mode::Batch, RunId(1), 0, Stage::Recognize),
            gpu,
            events: events.clone(),
        };

        let _ = VramStore::in_memory();
        let _ = std::mem::size_of::<GpuSnapshot>();
        let out = task.inference(&[&member_a, &member_b], |_| Ok(42)).await?;
        assert_eq!(out, 42);

        let mut started_runs = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let Event::Started { run, model, .. } = ev {
                assert_eq!(model.as_deref(), Some("hayai-ocr"));
                started_runs.push(run);
            }
        }
        assert_eq!(started_runs, vec![RunId(2)]);
        Ok(())
    }
}
