use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use koharu_scene::{Content, LayerId, PageId, Project, ProjectId, Session};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, mpsc};

use crate::{
    Error, Event, PipelineConfig, Request, Result, RunId, Step, Target,
    scheduler::{Command, Plan, Queue, Run, Scheduler},
    steps::inpaint::AreaWork,
};

const EVENTS: usize = 1024;

/// Runs Koharu's models for the app, its canvas, and the agent alike: a
/// cheap handle to clone. The scheduler and the thread owning the models end
/// when the last handle drops.
#[derive(Clone)]
pub struct Pipeline(Arc<Shared>);

struct Shared {
    config: Arc<RwLock<PipelineConfig>>,
    events: broadcast::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    next: AtomicU64,
}

impl Pipeline {
    /// Starts the scheduler and the thread owning the models, so it needs a
    /// Tokio runtime. Models load on first use.
    #[must_use]
    pub fn new(config: PipelineConfig) -> Self {
        let config = Arc::new(RwLock::new(config));
        let events = broadcast::channel(EVENTS).0;
        let commands = Scheduler::spawn(Arc::clone(&config), events.clone());
        Self(Arc::new(Shared {
            config,
            events,
            commands,
            next: AtomicU64::new(0),
        }))
    }

    /// Follows a changed `[pipeline]`. A model switched away from leaves
    /// memory once the work queued with it ends; runs already asked for keep
    /// the settings they started with.
    pub fn set_config(&self, config: PipelineConfig) {
        *self.0.config.write() = config;
        self.send(Command::Settle);
    }

    /// What every run tells the app from now on.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    /// Queues `request` on the project `session` holds and returns its run.
    pub fn run(&self, session: Arc<Mutex<Session>>, request: Request) -> Result<RunId> {
        let config = request
            .config
            .unwrap_or_else(|| self.0.config.read().clone());
        let project = session.lock().project();
        let plan = plan(&project, &request.target, &request.steps, &config)?;
        let id = RunId(self.0.next.fetch_add(1, Ordering::Relaxed));
        let run = Run::new(id, session, project.id(), config, plan, &request.steps);
        self.send(Command::Run(run));
        Ok(id)
    }

    /// Stops every run, running and queued. What they finished stays, and
    /// nothing more is written; a model call on the GPU finishes, but a
    /// local translation stops at its next token.
    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    /// Unloads every model once the GPU's current work ends, as ComfyUI's
    /// Unload Models does.
    pub fn unload(&self) {
        self.send(Command::Unload);
    }

    /// Forgets the translation conversation of a project that closed.
    pub fn close(&self, project: ProjectId) {
        self.send(Command::Close(project));
    }

    fn send(&self, command: Command) {
        self.0
            .commands
            .send(command)
            .unwrap_or_else(|_| panic!("the scheduler runs while the pipeline does"));
    }
}

/// What a run on `target` works on, or why it can't run.
fn plan(
    project: &Project,
    target: &Target,
    steps: &std::collections::BTreeSet<Step>,
    config: &PipelineConfig,
) -> Result<Plan> {
    if steps.is_empty() {
        return Err(Error::invalid("a run needs a step"));
    }
    let pages = |pages: Vec<PageId>| Plan {
        queue: Queue::Pages,
        pages: pages.into_iter().map(|page| (page, None)).collect(),
        area: None,
    };
    Ok(match target {
        Target::Project => pages(project.pages().map(|page| page.id).collect()),
        Target::Pages(chosen) => {
            for page in chosen {
                project.page(*page)?;
            }
            pages(
                project
                    .pages()
                    .map(|page| page.id)
                    .filter(|page| chosen.contains(page))
                    .collect(),
            )
        }
        Target::Blocks(blocks) => {
            if steps.contains(&Step::Detect) {
                return Err(Error::invalid("detection doesn't run on blocks"));
            }
            let mut held = BTreeMap::<PageId, Vec<LayerId>>::new();
            for &block in blocks {
                let page = project
                    .pages()
                    .find(|page| {
                        project.layers_of(page.id).is_ok_and(|layers| {
                            layers.iter().any(|layer| {
                                layer.id == block && matches!(layer.content, Content::Block(_))
                            })
                        })
                    })
                    .ok_or_else(|| Error::invalid(format!("layer {block} isn't a block")))?;
                held.entry(page.id).or_default().push(block);
            }
            Plan {
                queue: Queue::Hand,
                pages: project
                    .pages()
                    .filter_map(|page| Some((page.id, Some(held.remove(&page.id)?.into()))))
                    .collect(),
                area: None,
            }
        }
        Target::Area { page, area, prompt } => {
            if steps.iter().ne([Step::Inpaint].iter()) {
                return Err(Error::invalid("an area takes inpainting only"));
            }
            project.page(*page)?;
            if area.pixels.len() != area.rect.width as usize * area.rect.height as usize {
                return Err(Error::invalid(format!(
                    "the area has {} pixels for a {}×{} rectangle",
                    area.pixels.len(),
                    area.rect.width,
                    area.rect.height
                )));
            }
            let prompt = prompt
                .as_deref()
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty());
            let model = config.inpainter.model;
            if prompt.is_some() && !model.prompted() {
                return Err(Error::invalid(format!("{model} takes no prompt")));
            }
            Plan {
                queue: Queue::Hand,
                pages: vec![(*page, None)],
                area: Some(Arc::new(AreaWork {
                    area: area.clone(),
                    prompt: prompt.map(str::to_owned),
                })),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Created, Edit, Rect};
    use serde_json::json;

    use super::*;
    use crate::{Area, Inpainter};

    /// A project of two pages, the second holding one block, in a folder
    /// removed with it.
    fn project() -> anyhow::Result<(tempfile::TempDir, Project, PageId, LayerId)> {
        let dir = tempfile::tempdir()?;
        let mut session = Session::create(dir.path().join("test.khr"))?;
        let ops = [
            json!({"op": "add_page", "index": 0, "size": {"width": 100, "height": 100}}),
            json!({"op": "add_page", "id": "$p", "index": 1, "size": {"width": 100, "height": 100}}),
            json!({"op": "add_layer", "page": "$p", "index": 0, "layer": {"kind": "block", "source": {}}}),
        ];
        let change = session.apply(Edit {
            label: "test".to_owned(),
            ops: ops
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()?,
        })?;
        let Some(Created::Layer { id: block, .. }) = change.created.get(2).cloned() else {
            anyhow::bail!("no block");
        };
        let project = session.project();
        let page = project.pages().nth(1).map(|page| page.id).expect("a page");
        Ok((dir, project, page, block))
    }

    #[test]
    fn a_target_refuses_steps_that_dont_run_on_it() -> anyhow::Result<()> {
        let (_dir, project, page, block) = project()?;
        let config = PipelineConfig::default();
        let steps = |steps: &[Step]| steps.iter().copied().collect();
        let blocks = Target::Blocks(vec![block]);
        assert!(plan(&project, &blocks, &steps(&[Step::Detect]), &config).is_err());
        let found = plan(&project, &blocks, &steps(&[Step::Recognize]), &config)?;
        assert_eq!(found.pages.len(), 1);
        assert_eq!(found.pages[0].0, page);

        let area = |prompt: Option<&str>| Target::Area {
            page,
            area: Area {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                pixels: vec![255, 0],
            },
            prompt: prompt.map(str::to_owned),
        };
        let inpaint = steps(&[Step::Inpaint]);
        assert!(plan(&project, &area(None), &steps(&[Step::Translate]), &config).is_err());
        assert!(plan(&project, &area(None), &inpaint, &config).is_ok());
        // LaMa takes no prompt; FLUX.3 does.
        assert!(plan(&project, &area(Some("sky")), &inpaint, &config).is_err());
        let mut flux = config.clone();
        flux.inpainter.model = Inpainter::Flux3Klein;
        assert!(plan(&project, &area(Some("sky")), &inpaint, &flux).is_ok());
        Ok(())
    }
}
