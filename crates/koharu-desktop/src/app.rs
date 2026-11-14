//! The app's state: the projects folder, the open project and what runs on
//! it, the renderer and the pipeline, and the window. The open project's
//! session tells every change, whichever writer made it, and the app shows
//! and tells each.

use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use anyhow::{Context as _, Result};
use koharu_agent::{Agent, McpServer, Tools};
use koharu_canvas::{Canvas, Effect};
use koharu_pipeline::{Area, Pipeline, Stage, Target};
use koharu_renderer::{Gpu, Renderer};
use koharu_scene::{Change, Edit, PageId, Session};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter as _};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio_util::task::AbortOnDropHandle;

use crate::{
    events::{self, Event},
    library::Library,
    runs::Telemetry,
    screen::Screen,
};

/// The port the MCP server listens on, the `serve` example's, so a client's
/// configuration stays valid from one launch to the next.
const MCP_PORT: u16 = 7878;

pub struct App {
    pub library: Library,
    pub renderer: Renderer,
    pub gpu: Arc<Gpu>,
    pub pipeline: Pipeline,
    pub screen: Arc<Screen>,
    pub project: Mutex<Option<Project>>,
    pub telemetry: Telemetry,
    /// Set once the window's app starts, before anything is told.
    handle: OnceLock<AppHandle<CefRuntime>>,
    /// Set once it listens; serves the open project.
    mcp: OnceLock<McpServer>,
}

/// The open project, and what runs on it, which go when it closes.
pub struct Project {
    pub name: String,
    pub session: Arc<Mutex<Session>>,
    /// The page the canvas shows; `None` when the project has none.
    pub page: Option<PageId>,
    /// The conversation with the agent, on the model it was made with.
    pub conversation: Option<(String, Arc<tokio::sync::Mutex<Agent>>)>,
    /// The agent's run going on, numbered, which closing aborts.
    pub run: Option<(u64, AbortOnDropHandle<()>)>,
    /// Shows and tells the session's changes.
    _follow: AbortOnDropHandle<()>,
}

impl App {
    pub fn new(library: Library, renderer: Renderer, gpu: Arc<Gpu>, pipeline: Pipeline) -> Self {
        Self {
            library,
            renderer,
            screen: Arc::new(Screen::new(Arc::clone(&gpu))),
            gpu,
            pipeline,
            project: Mutex::new(None),
            telemetry: Telemetry::default(),
            handle: OnceLock::new(),
            mcp: OnceLock::new(),
        }
    }

    /// Starts telling the interface, following the pipeline, the runtime's
    /// downloads, and the renderer's fonts, and serving MCP.
    pub fn start(self: &Arc<Self>, handle: AppHandle<CefRuntime>) {
        let _ = self.handle.set(handle);
        self.follow_pipeline();
        let app = Arc::clone(self);
        let mut renderer = self.renderer.subscribe();
        tauri::async_runtime::spawn(async move {
            while renderer.changed().await.is_ok() {
                app.show_later();
            }
        });
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            match McpServer::start(MCP_PORT).await {
                Ok(server) => {
                    let server = app.mcp.get_or_init(|| server);
                    let project = app.project.lock();
                    server.open(project.as_ref().map(|project| app.tools(project)));
                }
                Err(error) => {
                    tracing::warn!("couldn't serve MCP on port {MCP_PORT}: {error:#}");
                }
            }
        });
    }

    /// Tells the interface `payload`, once it can be told.
    pub fn emit<T: Serialize + Clone>(&self, event: Event<T>, payload: T) {
        if let Some(handle) = self.handle.get()
            && let Err(error) = handle.emit(event.name, payload)
        {
            tracing::warn!("failed to tell the interface {}: {error}", event.name);
        }
    }

    /// Opens `session` in place of the open project, at its first page.
    pub fn open(self: &Arc<Self>, name: String, session: Session) -> Result<()> {
        self.close();
        let page = session.project().pages().next().map(|page| page.id);
        let follow = self.follow(session.subscribe());
        let session = Arc::new(Mutex::new(session));
        self.screen
            .show(Canvas::new(Arc::clone(&self.gpu), Arc::clone(&session)));
        {
            let mut project = self.project.lock();
            let opened = project.insert(Project {
                name: name.clone(),
                session,
                page,
                conversation: None,
                run: None,
                _follow: follow,
            });
            if let Some(server) = self.mcp.get() {
                server.open(Some(self.tools(opened)));
            }
        }
        self.emit(events::PROJECT, Some(name));
        self.emit(events::PAGE, page);
        self.show()
    }

    /// Closes the open project, if one is, and stops what runs on it, so
    /// nothing writes to a closed session.
    pub fn close(&self) {
        let project = {
            let mut project = self.project.lock();
            if let Some(server) = self.mcp.get() {
                server.open(None);
            }
            project.take()
        };
        self.pipeline.stop();
        self.screen.hide();
        if let Some(project) = project {
            self.pipeline.close(project.session.lock().id());
            self.emit(events::PROJECT, None);
            self.emit(events::PAGE, None);
        }
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

    /// The agent's tools on `project`.
    pub fn tools(&self, project: &Project) -> Tools {
        Tools::new(
            Arc::clone(&project.session),
            self.pipeline.clone(),
            self.renderer.clone(),
        )
    }

    /// Shows `page` on the canvas.
    pub fn show_page(self: &Arc<Self>, page: PageId) -> Result<()> {
        {
            let mut project = self.project.lock();
            let project = project.as_mut().context("no project is open")?;
            project.session.lock().project().page(page)?;
            project.page = Some(page);
        }
        self.emit(events::PAGE, Some(page));
        self.show()
    }

    /// Runs `edit` on the open project as one undo step.
    pub fn edit(&self, edit: Edit) -> Result<Change> {
        tracing::info!(label = %edit.label, ops = edit.ops.len(), "edit");
        Ok(self.session()?.lock().apply(edit)?)
    }

    /// Tells the interface `session`'s changes, and shows each on the
    /// canvas. A burst shows once.
    fn follow(self: &Arc<Self>, mut changes: Receiver<Change>) -> AbortOnDropHandle<()> {
        let app = Arc::clone(self);
        AbortOnDropHandle::new(tokio::spawn(async move {
            loop {
                match changes.recv().await {
                    Ok(change) => app.emit(events::CHANGE, change),
                    // The interface missed changes: it reads the project again.
                    Err(RecvError::Lagged(_)) => {
                        let name = app.project.lock().as_ref().map(|open| open.name.clone());
                        app.emit(events::PROJECT, name);
                    }
                    Err(RecvError::Closed) => return,
                }
                while let Ok(change) = changes.try_recv() {
                    app.emit(events::CHANGE, change);
                }
                app.show_later();
            }
        }))
    }

    /// Shows the open project off the async runtime's workers.
    fn show_later(self: &Arc<Self>) {
        let app = Arc::clone(self);
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = app.show() {
                tracing::error!("failed to show the project: {error:#}");
            }
        });
    }

    /// Shows the open project's page as it is now. A page that went away
    /// gives its place to the first.
    pub fn show(self: &Arc<Self>) -> Result<()> {
        let (session, page, moved) = {
            let mut project = self.project.lock();
            let Some(project) = project.as_mut() else {
                return Ok(());
            };
            let pages = project.session.lock().project();
            let moved = project.page.is_none_or(|page| pages.page(page).is_err());
            if moved {
                project.page = pages.pages().next().map(|page| page.id);
            }
            (Arc::clone(&project.session), project.page, moved)
        };
        if moved {
            self.emit(events::PAGE, page);
        }
        match page {
            Some(page) => {
                let frame = self.renderer.render(&session.lock(), page)?;
                self.screen.set_frame(frame);
            }
            None => self
                .screen
                .show(Canvas::new(Arc::clone(&self.gpu), session)),
        }
        self.camera();
        self.screen.request_redraw();
        Ok(())
    }

    /// Tells the interface where the page shows, if that moved.
    pub fn camera(&self) {
        if let Some(camera) = self.screen.camera() {
            self.emit(events::CAMERA, camera);
        }
    }

    /// Carries out what a canvas gesture did.
    pub fn handle(self: &Arc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            let result = match effect {
                Effect::Edit(edit) => self.edit(edit).map(drop),
                Effect::Selection(layers) => {
                    tracing::info!(layers = layers.len(), "selection");
                    self.emit(events::SELECTION, layers);
                    Ok(())
                }
                Effect::Camera(_) => {
                    self.camera();
                    Ok(())
                }
                Effect::Color(color) => {
                    tracing::info!(?color, "color picked");
                    self.emit(events::COLOR, color);
                    Ok(())
                }
                Effect::BrushDiameter(diameter) => {
                    self.emit(events::BRUSH_DIAMETER, diameter);
                    Ok(())
                }
                Effect::Notice(notice) => {
                    tracing::info!(?notice, "canvas notice");
                    self.emit(events::NOTICE, notice);
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

    /// The agent's run going on.
    fn agent_run(app: &App) -> Option<u64> {
        app.project
            .lock()
            .as_ref()
            .and_then(|project| project.run.as_ref().map(|(run, _)| *run))
    }

    /// An app on a projects folder in `dir`, or `None` on a machine without a
    /// GPU.
    fn app(dir: &std::path::Path) -> anyhow::Result<Option<Arc<App>>> {
        let renderer = Renderer::new(RendererConfig::default());
        let Ok(gpu) = renderer.gpu() else {
            return Ok(None);
        };
        Ok(Some(Arc::new(App::new(
            Library::new(dir.to_owned())?,
            renderer,
            gpu,
            Pipeline::new(PipelineConfig::default()),
        ))))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closing_a_project_stops_its_runs() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let Some(app) = app(dir.path())? else {
            return Ok(());
        };
        let session = app.library.create("closing")?;
        app.open("closing".to_owned(), session)?;

        // An agent run holding the project, as the agent's tools do.
        let held = app.session()?;
        let (sender, dropped) = oneshot::channel();
        let guard = Dropped(Some(sender));
        app.start_agent(async move {
            let _held = (held, guard);
            std::future::pending::<()>().await;
        })?;
        assert!(agent_run(&app).is_some());

        app.close();
        assert!(agent_run(&app).is_none());
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
        let Some(app) = app(dir.path())? else {
            return Ok(());
        };
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
        )?;
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await?
            .context("the run ended without telling")?;
        assert!(matches!(event, AgentEvent::Failed { .. }), "{event:?}");
        Ok(())
    }
}
