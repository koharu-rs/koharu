//! The app's state: the projects folder, the open project, the renderer and
//! the pipeline, and the window. The scene has no change notification, so
//! every writer of the open project, a command, the canvas, the pipeline, or
//! the agent, ends in `changed`, which shows and tells what changed.

use std::{
    collections::BTreeSet,
    sync::{Arc, Once},
};

use anyhow::{Context as _, Result};
use koharu_canvas::{Canvas, Effect};
use koharu_pipeline::{Area, Pipeline, Stage, Target};
use koharu_renderer::{Gpu, Renderer};
use koharu_scene::{Edit, History, PageId, PageSummary, Revision, Session};
use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;

use crate::{
    events::{CanvasEvent, Channels},
    library::Library,
    runs::Runs,
    screen::Screen,
};

pub struct App {
    pub library: Library,
    pub renderer: Renderer,
    pub pipeline: Pipeline,
    pub project: Mutex<Option<Project>>,
    pub channels: Channels,
    pub screen: Arc<Screen>,
    pub runs: Runs,
}

/// The open project.
pub struct Project {
    pub name: String,
    pub session: Arc<Mutex<Session>>,
    /// The page the editor shows; `None` when the project has none.
    pub page: Option<PageId>,
}

/// The open project as the interface lists it.
#[derive(Clone, Debug, Serialize, Type)]
pub struct ProjectInfo {
    pub name: String,
    pub revision: Revision,
    pub page: Option<PageId>,
    pub pages: Vec<PageSummary>,
    pub history: History,
}

impl App {
    pub fn new(library: Library, renderer: Renderer, pipeline: Pipeline) -> Self {
        let screen = Arc::new(Screen::new(renderer.clone()));
        Self {
            library,
            renderer,
            pipeline,
            project: Mutex::new(None),
            channels: Channels::default(),
            screen,
            runs: Runs::default(),
        }
    }

    /// Opens `session` in place of the open project, at its first page.
    pub fn open(self: &Arc<Self>, name: String, session: Session) -> Result<()> {
        self.close();
        let page = session.project().pages().next().map(|page| page.id);
        let session = Arc::new(Mutex::new(session));
        let gpu = self.renderer.gpu()?;
        name_gpu(&gpu);
        let canvas = Canvas::new(gpu, Arc::clone(&session));
        self.screen.show(canvas);
        *self.project.lock() = Some(Project {
            name,
            session,
            page,
        });
        self.serve();
        self.changed()
    }

    /// Closes the open project, if one is, and stops what runs on it, so
    /// nothing writes to a closed session. Runs stop after the project is
    /// taken, so a conversation an agent run made meanwhile goes with them.
    pub fn close(&self) {
        let project = self.project.lock().take();
        self.stop_runs();
        self.screen.hide();
        if let Some(project) = project {
            self.pipeline.close(project.session.lock().id());
        }
        self.channels.project(None);
        self.screen.request_redraw();
    }

    /// The open project's session.
    pub fn session(&self) -> Result<Arc<Mutex<Session>>> {
        self.project
            .lock()
            .as_ref()
            .map(|project| Arc::clone(&project.session))
            .context("no project is open")
    }

    pub fn info(&self) -> Option<ProjectInfo> {
        let project = self.project.lock();
        let project = project.as_ref()?;
        let session = project.session.lock();
        Some(ProjectInfo {
            name: project.name.clone(),
            revision: session.revision(),
            page: project.page,
            pages: session
                .project()
                .pages()
                .map(|page| PageSummary {
                    id: page.id,
                    size: page.size,
                    background: page.background,
                })
                .collect(),
            history: session.history(),
        })
    }

    /// Shows `page` on the canvas.
    pub fn select_page(self: &Arc<Self>, page: PageId) -> Result<()> {
        {
            let mut project = self.project.lock();
            let project = project.as_mut().context("no project is open")?;
            project.session.lock().project().page(page)?;
            project.page = Some(page);
        }
        self.changed()
    }

    /// Runs `edit` on the open project as one undo step.
    pub fn edit(self: &Arc<Self>, edit: Edit) -> Result<()> {
        tracing::info!(label = %edit.label, ops = edit.ops.len(), "edit");
        self.session()?.lock().apply(edit)?;
        self.changed()
    }

    /// Shows the open project as it is now: the current page's frame on the
    /// canvas, and the project to the interface. A page that went away
    /// gives its place to the first.
    pub fn changed(self: &Arc<Self>) -> Result<()> {
        let (session, page) = {
            let mut project = self.project.lock();
            let Some(project) = project.as_mut() else {
                return Ok(());
            };
            let pages = project.session.lock().project();
            if project.page.is_none_or(|page| pages.page(page).is_err()) {
                project.page = pages.pages().next().map(|page| page.id);
            }
            (Arc::clone(&project.session), project.page)
        };
        match page {
            Some(page) => {
                let frame = self.renderer.render(&session.lock(), page)?;
                if frame.needs_fonts() {
                    // Bundled fonts download on first use; the page shows
                    // with fallbacks until they're in.
                    let app = Arc::clone(self);
                    let fonts = frame.clone();
                    tauri::async_runtime::spawn(async move {
                        match app.renderer.load_fonts(&fonts).await {
                            Ok(()) => {
                                if let Err(error) = app.changed() {
                                    tracing::error!("failed to show the page again: {error:#}");
                                }
                            }
                            Err(error) => tracing::warn!("failed to load fonts: {error:#}"),
                        }
                    });
                }
                self.screen.set_frame(frame);
            }
            None => self
                .screen
                .show(Canvas::new(self.renderer.gpu()?, Arc::clone(&session))),
        }
        self.camera();
        self.channels.project(self.info());
        self.screen.request_redraw();
        Ok(())
    }

    /// Tells the interface where the page shows, if that moved.
    pub fn camera(&self) {
        if let Some((zoom, x, y)) = self.screen.camera() {
            self.channels.canvas(CanvasEvent::Camera { zoom, x, y });
        }
    }

    /// Carries out what a canvas gesture did.
    pub fn handle(self: &Arc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            let result = match effect {
                Effect::Edit(edit) => self.edit(edit),
                Effect::Selection(layers) => {
                    tracing::info!(layers = layers.len(), "selection");
                    self.channels.canvas(CanvasEvent::Selection { layers });
                    Ok(())
                }
                Effect::Camera(_) => {
                    self.camera();
                    Ok(())
                }
                Effect::Color(color) => {
                    tracing::info!(?color, "color picked");
                    self.channels.canvas(CanvasEvent::Color { color });
                    Ok(())
                }
                Effect::BrushDiameter(diameter) => {
                    self.channels
                        .canvas(CanvasEvent::BrushDiameter { diameter });
                    Ok(())
                }
                Effect::Notice(notice) => {
                    tracing::info!(?notice, "canvas notice");
                    self.channels.canvas(CanvasEvent::Notice { notice });
                    Ok(())
                }
                Effect::Inpaint { page, rect, mask } => self
                    .process(
                        Target::Area {
                            page,
                            area: Area { rect, pixels: mask },
                            prompt: None,
                        },
                        BTreeSet::from([Stage::Inpaint]),
                        None,
                    )
                    .map(drop),
            };
            if let Err(error) = result {
                tracing::error!("failed to carry out a canvas gesture: {error:#}");
            }
        }
    }
}

/// Names the renderer's GPU on every event, from any thread, once it has
/// one.
fn name_gpu(gpu: &Gpu) {
    static NAMED: Once = Once::new();
    NAMED.call_once(|| {
        let info = gpu.adapter.get_info();
        let driver = if info.driver_info.is_empty() {
            info.driver
        } else {
            info.driver_info
        };
        let context = sentry::protocol::GpuContext {
            name: info.name,
            id: Some(format!("{:#06x}", info.device)),
            vendor_id: Some(format!("{:#06x}", info.vendor)),
            api_type: Some(info.backend.to_string()),
            driver_version: (!driver.is_empty()).then_some(driver),
            ..Default::default()
        };
        sentry::Hub::main().with_integration(|sentry: &koharu_sentry::Sentry| {
            sentry.set_context("gpu", sentry::protocol::Context::Gpu(Box::new(context)));
        });
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use koharu_agent::AgentConfig;
    use koharu_pipeline::PipelineConfig;
    use koharu_renderer::RendererConfig;
    use tokio::sync::{mpsc, oneshot};

    use super::*;
    use crate::runs::AgentEvent;

    /// Sends when the run holding it is dropped.
    struct Dropped(Option<oneshot::Sender<()>>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_project_stops_its_runs() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let app = Arc::new(App::new(
            Library::new(dir.path().to_owned())?,
            Renderer::new(RendererConfig::default()),
            Pipeline::new(PipelineConfig::default()),
        ));
        let session = app.library.create("closing")?;
        app.open("closing".to_owned(), session)?;

        // An agent run holding the project, as the agent's tools do.
        let held = app.session()?;
        let (sender, dropped) = oneshot::channel();
        let guard = Dropped(Some(sender));
        app.start_agent(async move {
            let _held = (held, guard);
            std::future::pending::<()>().await;
        });
        assert!(app.agent_run().is_some());

        app.close();
        assert!(app.agent_run().is_none());
        assert!(app.session().is_err());
        tokio::time::timeout(Duration::from_secs(5), dropped).await??;
        // Nothing holds the file any more: it opens again.
        drop(app.library.open("closing")?);
        Ok(())
    }

    /// A run connects to its model inside the run, which closing the project
    /// aborts; connected first, a conversation made during a close would keep
    /// the closed project. A model that can't connect fails the run, not its
    /// start.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_run_connects_to_its_model_inside_the_run() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let app = Arc::new(App::new(
            Library::new(dir.path().to_owned())?,
            Renderer::new(RendererConfig::default()),
            Pipeline::new(PipelineConfig::default()),
        ));
        let session = app.library.create("connecting")?;
        app.open("connecting".to_owned(), session)?;

        let (sender, mut events) = mpsc::unbounded_channel();
        app.run_agent(
            "no model",
            AgentConfig::default(),
            "hi".to_owned(),
            move |event| {
                let _ = sender.send(event);
            },
            None,
        );
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await?
            .context("the run ended without telling")?;
        assert!(matches!(event, AgentEvent::Failed { .. }), "{event:?}");
        Ok(())
    }
}
