//! The app's state: the projects folder, the open project and what runs on
//! it, the renderer and the pipeline, and the window. The open project's
//! session tells every change, whichever writer made it, and the app shows
//! and tells each.

use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use koharu_agent::{Agent, McpServer, Tools};
use koharu_canvas::{Canvas, Effect, Facing, Side, Snapshot};
use koharu_pipeline::{Area, Pipeline, PipelineConfig, Request, RunId, Stage, Target};
use koharu_renderer::Renderer;
use koharu_scene::{Change, Edit, PageId, Session};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager as _};
use tauri_runtime_cef::CefRuntime;
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio_util::task::AbortOnDropHandle;

use crate::{
    events::{self, Event},
    library::Library,
    screen::{self, Screen},
};

pub struct App {
    pub library: Library,
    pub renderer: Renderer,
    pub pipeline: Pipeline,
    pub screen: Screen,
    pub project: Mutex<Option<Project>>,
    /// Set once the window's app starts, before anything is told.
    handle: OnceLock<AppHandle<CefRuntime>>,
    /// Set once it listens; serves the open project.
    mcp: OnceLock<McpServer>,
}

/// The page facing the shown one in a two-page spread, and its side.
#[derive(Clone, Copy, Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FacingPage {
    pub page: PageId,
    pub side: Side,
}

/// What the canvas shows of the open project: its session, its page, and
/// the page facing that one in a spread.
type Shown = (Arc<Mutex<Session>>, Option<PageId>, Option<FacingPage>);

/// The open project, and what runs on it, which go when it closes.
pub struct Project {
    pub name: String,
    pub session: Arc<Mutex<Session>>,
    /// The page the canvas shows; `None` when the project has none.
    pub page: Option<PageId>,
    /// The page facing it, while the reader shows a two-page spread.
    pub facing: Option<FacingPage>,
    /// The conversation with the agent, on the model it was made with.
    pub conversation: Option<(String, Arc<tokio::sync::Mutex<Agent>>)>,
    /// The agent's run going on, numbered, which closing aborts.
    pub run: Option<(u64, AbortOnDropHandle<()>)>,
    /// An extension's chapter loading into it, which closing stops.
    pub loading: Option<AbortOnDropHandle<()>>,
    /// Shows and tells the session's changes.
    _relay: AbortOnDropHandle<()>,
}

impl App {
    pub fn new(library: Library, renderer: Renderer, pipeline: Pipeline) -> Self {
        Self {
            library,
            renderer,
            screen: Screen::default(),
            pipeline,
            project: Mutex::new(None),
            handle: OnceLock::new(),
            mcp: OnceLock::new(),
        }
    }

    /// Starts telling the interface every run's events and every download,
    /// and serving MCP.
    pub fn start(self: &Arc<Self>, handle: AppHandle<CefRuntime>) {
        let _ = self.handle.set(handle);
        let app = Arc::clone(self);
        let mut runs = self.pipeline.events();
        tauri::async_runtime::spawn(async move {
            loop {
                match runs.recv().await {
                    Ok(event) => app.emit(events::PIPELINE, event),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
            }
        });
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let mut transfers = koharu_runtime::transfers();
            while let Some(transfer) = transfers.next().await {
                app.emit(events::DOWNLOAD, transfer);
            }
        });
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            // The `serve` example's port, fixed so a client's configuration
            // stays valid from one launch to the next.
            match McpServer::start(7878).await {
                Ok(server) => {
                    let server = app.mcp.get_or_init(|| server);
                    let project = app.project.lock();
                    server.open(project.as_ref().map(|project| app.tools(project)));
                }
                Err(error) => {
                    tracing::warn!("couldn't serve MCP on port 7878: {error:#}");
                }
            }
        });
    }

    /// Tells the interface `payload`, once it can be told.
    pub fn emit<T: Serialize + Clone>(&self, event: Event<T>, payload: T) {
        if let Some(handle) = self.handle.get() {
            event.tell(handle, payload);
        }
    }

    /// Brings the window forward, for Koharu launched again while it runs.
    pub fn bring_forward(&self) {
        if let Some(window) = self
            .handle
            .get()
            .and_then(|handle| handle.get_window(screen::MAIN))
        {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }

    /// Opens `session` in place of the open project, at its first page.
    pub fn open(self: &Arc<Self>, name: String, session: Session) -> Result<()> {
        self.close();
        let page = session.project().pages().next().map(|page| page.id);
        let relay = self.relay(session.subscribe());
        let session = Arc::new(Mutex::new(session));
        self.screen.set_canvas(Some(Canvas::new(
            Arc::clone(&session),
            self.renderer.clone(),
        )));
        {
            let mut project = self.project.lock();
            let opened = project.insert(Project {
                name: name.clone(),
                session,
                page,
                facing: None,
                conversation: None,
                run: None,
                loading: None,
                _relay: relay,
            });
            if let Some(server) = self.mcp.get() {
                server.open(Some(self.tools(opened)));
            }
        }
        self.emit(events::PROJECT, Some(name));
        self.emit(events::PAGE, page);
        self.refresh()
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
        self.screen.set_canvas(None);
        if let Some(project) = project {
            self.pipeline.close(project.session.lock().id());
            self.emit(events::PROJECT, None);
            self.emit(events::PAGE, None);
        }
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

    /// Runs `stages` on `target` in the open project, with `config` in the
    /// pipeline's settings' place when given. The pipeline traces the run
    /// under whatever asked for it.
    pub fn process(
        &self,
        target: Target,
        stages: BTreeSet<Stage>,
        config: Option<PipelineConfig>,
    ) -> Result<RunId> {
        Ok(self.pipeline.run(
            self.session()?,
            Request {
                target,
                stages,
                config,
            },
        )?)
    }

    /// Runs `edit` on the open project as one undo step.
    pub fn edit(&self, edit: Edit) -> Result<Change> {
        tracing::info!(label = %edit.label, ops = edit.ops.len(), "edit");
        Ok(self.session()?.lock().apply(edit)?)
    }

    /// Tells the interface `session`'s changes, and shows them on the canvas
    /// one redraw at a time: a burst, and what comes during a redraw, shows
    /// once. The shown page settles before the interface hears of a change,
    /// as it reads the shown page again for one: a page the change removed
    /// would otherwise be the one it asks for.
    fn relay(self: &Arc<Self>, mut changes: Receiver<Change>) -> AbortOnDropHandle<()> {
        let app = Arc::clone(self);
        AbortOnDropHandle::new(tokio::spawn(async move {
            loop {
                let mut told = Vec::new();
                let lagged = match changes.recv().await {
                    Ok(change) => {
                        told.push(change);
                        false
                    }
                    Err(RecvError::Lagged(_)) => true,
                    Err(RecvError::Closed) => return,
                };
                while let Ok(change) = changes.try_recv() {
                    told.push(change);
                }
                let showing = Arc::clone(&app);
                let shown = tokio::task::spawn_blocking(move || {
                    let shown = showing.settle();
                    // The interface missed changes: it reads the project again.
                    if lagged {
                        let name = showing
                            .project
                            .lock()
                            .as_ref()
                            .map(|open| open.name.clone());
                        showing.emit(events::PROJECT, name);
                    }
                    for change in told {
                        showing.emit(events::CHANGE, change);
                    }
                    shown.map_or(Ok(()), |shown| showing.show(shown))
                })
                .await;
                if let Err(error) = shown.map_err(anyhow::Error::from).and_then(|result| result) {
                    tracing::error!("failed to show the project: {error:#}");
                }
            }
        }))
    }

    /// Shows the open project's page as it is now. A page that went away
    /// gives its place to the first.
    pub fn refresh(self: &Arc<Self>) -> Result<()> {
        self.settle().map_or(Ok(()), |shown| self.show(shown))
    }

    /// The open project's session and the page it shows, which moves to the
    /// first when it went away, telling the interface so; a facing page
    /// that went away leaves the page on its own.
    fn settle(&self) -> Option<Shown> {
        let (session, page, facing, moved) = {
            let mut project = self.project.lock();
            let project = project.as_mut()?;
            let pages = project.session.lock().project();
            let moved = project.page.is_none_or(|page| pages.page(page).is_err());
            if moved {
                project.page = pages.pages().next().map(|page| page.id);
            }
            if moved
                || project
                    .facing
                    .is_some_and(|facing| pages.page(facing.page).is_err())
            {
                project.facing = None;
            }
            (
                Arc::clone(&project.session),
                project.page,
                project.facing,
                moved,
            )
        };
        if moved {
            self.emit(events::PAGE, page);
        }
        Some((session, page, facing))
    }

    /// Shows `page` of `session` on the canvas, with the page facing it, or
    /// an empty canvas when the project has no page.
    fn show(&self, (session, page, facing): Shown) -> Result<()> {
        match page {
            Some(page) => {
                let facing = facing
                    .map(|facing| {
                        anyhow::Ok(Facing {
                            snapshot: self.render(&session, facing.page)?,
                            side: facing.side,
                        })
                    })
                    .transpose()?;
                self.screen.show(self.render(&session, page)?, facing);
            }
            None => self
                .screen
                .set_canvas(Some(Canvas::new(session, self.renderer.clone()))),
        }
        Ok(())
    }

    /// `page` as `session` holds it now. The snapshot reads the project the
    /// frame drew, under the same lock, so the canvas never takes it.
    fn render(&self, session: &Mutex<Session>, page: PageId) -> Result<Snapshot> {
        let session = session.lock();
        let frame = self.renderer.render(&session, page)?;
        Snapshot::new(frame, &session).context("the page is gone")
    }

    /// Carries out what a canvas gesture did, and gives back the page as its
    /// edits left it, so the redraw that ends the gesture shows them: the
    /// relay renders a change only later, on another thread, and the canvas
    /// would meanwhile draw the page from before the edit.
    pub fn handle(self: &Arc<Self>, effects: Vec<Effect>) -> Option<Snapshot> {
        let mut edited = false;
        for effect in effects {
            let result = match effect {
                Effect::Edit(edit) => self.edit(edit).map(|_| edited = true),
                Effect::Selection(layers) => {
                    tracing::info!(layers = layers.len(), "selection");
                    self.emit(events::SELECTION, layers);
                    Ok(())
                }
                // The screen tells the camera itself.
                Effect::Camera(_) => Ok(()),
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
                Effect::Copy(text) => arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.set_text(text))
                    .context("the clipboard took no text"),
                // Windows' clipboard ends lines with CR LF; text breaks
                // lines with LF.
                Effect::Paste => arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.get_text())
                    .map(|text| self.screen.insert(text.replace("\r\n", "\n")))
                    .context("the clipboard has no text"),
                Effect::Inpaint {
                    page,
                    layer,
                    rect,
                    mask,
                } => self
                    .process(
                        Target::Area {
                            page,
                            area: Area { rect, pixels: mask },
                            prompt: None,
                            layer: Some(layer),
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
        if !edited {
            return None;
        }
        let (session, page) = {
            let project = self.project.lock();
            let project = project.as_ref()?;
            (Arc::clone(&project.session), project.page?)
        };
        self.render(&session, page)
            .inspect_err(|error| tracing::error!("failed to show a canvas edit: {error:#}"))
            .ok()
    }
}
