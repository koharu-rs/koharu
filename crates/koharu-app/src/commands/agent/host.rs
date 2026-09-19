use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};

use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use koharu_agent::{Control, Host, Invocation, Tool, ToolCall};
use koharu_desktop::{Desktop, Frame};
use koharu_pipeline::{Committer, Operation, Progress, RunStatus, Scope, Stage, StageOutput};
use koharu_scene::{Commit, EntityId, Snapshot};
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::commands::{
    ChannelExt as _,
    canvas::{CanvasChannel, Point},
    editing::{GeometryUpdate, TypographyUpdate},
    output,
    preferences::Preferences,
    processing::{JobChannel, JobKind, JobPhase, JobState, Processing, terminology_snapshot},
    project::{CurrentProject, Project, Typography},
};
use crate::host::Pipeline;

#[derive(Clone)]
pub(super) struct KoharuHost {
    project: CurrentProject,
    desktop: Desktop,
    canvas: CanvasChannel,
    processing: Processing,
    jobs: JobChannel,
    pipeline: Pipeline,
}

impl KoharuHost {
    pub(super) fn new(
        project: CurrentProject,
        desktop: Desktop,
        canvas: CanvasChannel,
        processing: Processing,
        jobs: JobChannel,
        pipeline: Pipeline,
    ) -> Self {
        Self {
            project,
            desktop,
            canvas,
            processing,
            jobs,
            pipeline,
        }
    }

    async fn project_context(&self) -> Result<Value> {
        let (project, pages) = {
            let current = self.project.project.lock().await;
            let project = current.as_ref().context("no project is open")?;
            let snapshot = project.snapshot();
            let pages = Project::pages(&snapshot)?
                .into_iter()
                .map(|page| Project::page(&snapshot, page.id))
                .collect::<Result<Vec<_>>>()?;
            (project.info(), pages)
        };
        let preferences = Preferences::load()?;
        let fonts = self.desktop.renderer().available_fonts().await?;
        let providers = preferences
            .providers
            .entries
            .iter()
            .map(|provider| {
                json!({
                    "name": provider.name,
                    "config": provider.config,
                    "credential_configured": provider
                        .credential
                        .as_ref()
                        .is_some_and(|credential| credential.configured),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project,
            "pages": pages,
            "configuration": {
                "pipeline": preferences.pipeline,
                "providers": providers,
                "typesetting": preferences.typesetting,
                "available_fonts": fonts.into_iter().map(|font| font.name).collect::<Vec<_>>(),
            },
        }))
    }

    async fn mutate<T>(
        &self,
        mutation: impl for<'project> FnOnce(
            &'project mut Project,
        ) -> Pin<
            Box<dyn Future<Output = Result<(Commit, T)>> + Send + 'project>,
        >,
    ) -> Result<Invocation>
    where
        T: serde::Serialize + Send,
    {
        let (commit, value, page, project) = {
            let mut current = self.project.project.lock().await;
            let project = current.as_mut().context("no project is open")?;
            let (commit, value) = mutation(project).await?;
            project.record_commit(&commit);
            project.reconcile_page();
            (commit, value, project.active_page(), project.info())
        };
        let revision = commit.revision;
        self.synchronize(commit, page).await?;
        Invocation::changed(json!({
            "revision": revision,
            "project": project,
            "result": value,
        }))
    }

    async fn synchronize(&self, commit: Commit, page: Option<EntityId>) -> Result<()> {
        self.desktop
            .synchronize(&commit.snapshot, page, &commit)
            .await?;
        let canvas = self.desktop.canvas_state();
        self.canvas.channel.publish(canvas);
        Ok(())
    }

    async fn run_pipeline(&self, arguments: RunPipeline, control: &Control) -> Result<Invocation> {
        let scope = arguments.scope()?;
        let operation = arguments.operation.into();
        let snapshot = self
            .project
            .project
            .lock()
            .await
            .as_ref()
            .context("no project is open")?
            .snapshot();
        let terminology = terminology_snapshot(&snapshot)?;
        let job = self.start_pipeline_job()?;

        let watcher = tauri::async_runtime::spawn({
            let control = control.clone();
            let stop = job.stop.clone();
            async move {
                control.cancelled().await;
                stop.stop();
            }
        });
        let mut committer = AgentCommitter { host: self.clone() };
        let progress = Arc::new(parking_lot::Mutex::new((0_usize, 0_usize)));
        let progress_job = job.clone();
        let request =
            koharu_pipeline::Request::new(operation, scope, job.stop.clone(), terminology)
                .with_progress(Arc::new(move |event| {
                    let update = match event {
                        Progress::Started { pages, stages } => {
                            let mut progress = progress.lock();
                            *progress = (0, pages.len().saturating_mul(stages.len()));
                            Some((0, progress.1, None, None))
                        }
                        Progress::Loading { page, stage, .. } => {
                            let progress = progress.lock();
                            Some((progress.0, progress.1, Some(page), Some(stage)))
                        }
                        Progress::Finished { page, stage, .. }
                        | Progress::Skipped { page, stage } => {
                            let mut progress = progress.lock();
                            progress.0 = progress.0.saturating_add(1).min(progress.1);
                            Some((progress.0, progress.1, Some(page), Some(stage)))
                        }
                        Progress::Running { .. } => None,
                    };
                    if let Some((completed, total, page, stage)) = update {
                        progress_job.update(completed, total, page, stage);
                    }
                }));
        let result = self
            .pipeline
            .execute(snapshot, request, &mut committer)
            .await;
        watcher.abort();
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                job.finish(JobState::Failed, Some(format!("{error:#}")));
                return Err(anyhow!(error));
            }
        };
        if report.status == RunStatus::Stopped {
            job.finish(JobState::Stopped, None);
            bail!("pipeline processing was cancelled");
        }
        job.finish(JobState::Finished, None);
        Invocation::changed(json!({
            "base_revision": report.base,
            "final_revision": report.final_revision,
            "completed": report.completed,
            "total": report.total,
            "elapsed_ms": report.elapsed.as_millis(),
        }))
    }

    fn start_pipeline_job(&self) -> Result<AgentPipelineJob> {
        AgentPipelineJob::start(&self.processing, &self.jobs)
    }
}

#[derive(Clone)]
struct AgentPipelineJob {
    id: crate::commands::processing::JobId,
    stop: koharu_pipeline::StopToken,
    processing: Processing,
    jobs: JobChannel,
}

impl AgentPipelineJob {
    fn start(processing: &Processing, jobs: &JobChannel) -> Result<Self> {
        let (id, stop) =
            processing.start_job(JobKind::Pipeline, JobPhase::Pipeline { stage: None }, jobs)?;
        Ok(Self {
            id,
            stop,
            processing: processing.clone(),
            jobs: jobs.clone(),
        })
    }

    fn update(&self, completed: usize, total: usize, page: Option<EntityId>, stage: Option<Stage>) {
        self.processing.update_job(self.id, &self.jobs, |job| {
            job.completed = completed;
            job.total = total;
            job.page = page;
            job.phase = JobPhase::Pipeline { stage };
        });
    }

    fn finish(self, state: JobState, error: Option<String>) {
        self.processing
            .finish_job(self.id, state, error, &self.jobs);
    }
}

#[async_trait]
impl Host for KoharuHost {
    async fn context(&self) -> Result<Value> {
        self.project_context().await
    }

    fn tools(&self) -> Vec<Tool> {
        static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
        TOOLS
            .get_or_init(|| {
                vec![
                    definition::<InspectProject>(
                        "inspect_project",
                        "Read the latest complete semantic project state after edits. This does not include page images.",
                    ),
                    definition::<ViewPage>(
                        "view_page",
                        "Render and inspect one page image. Call this only for pages whose visual appearance matters to the request.",
                    ),
                    definition::<RenamePage>("rename_page", "Rename a project page."),
                    definition::<MovePage>("move_page", "Move a page to a zero-based project index."),
                    definition::<DeletePages>("delete_pages", "Delete pages and their contents."),
                    definition::<AddTextBox>("add_text_box", "Add a paragraph text box to a page."),
                    definition::<SetText>("set_source_text", "Replace an element's source text."),
                    definition::<SetTranslation>(
                        "set_translation",
                        "Set an element's translation, or pass null to remove it.",
                    ),
                    definition::<SetTypography>(
                        "set_typography",
                        "Replace an element's typography settings. Font is a family name from available_fonts.",
                    ),
                    definition::<SetGeometry>(
                        "set_geometry",
                        "Replace an element's page-space polygon geometry.",
                    ),
                    definition::<SetVisibility>(
                        "set_visibility",
                        "Change visibility and/or opacity for elements.",
                    ),
                    definition::<DeleteElements>("delete_elements", "Delete elements and descendants."),
                    definition::<MoveElement>(
                        "move_element",
                        "Move an element under a page or element at a zero-based index.",
                    ),
                    definition::<RunPipeline>(
                        "run_pipeline",
                        "Run Koharu's configured processing pipeline for the whole project, selected pages, or selected text elements.",
                    ),
                ]
            })
            .clone()
    }

    #[tracing::instrument(
        target = "koharu_metrics",
        name = "agent_tool",
        skip_all,
        fields(tool = call.name.as_str()),
    )]
    async fn invoke(&self, call: ToolCall, control: &Control) -> Result<Invocation> {
        match call.name.as_str() {
            "inspect_project" => {
                let _: InspectProject = arguments(&call)?;
                Invocation::read(self.project_context().await?)
            }
            "view_page" => {
                let arguments: ViewPage = arguments(&call)?;
                let page = entity(&arguments.page)?;
                let (label, snapshot) = {
                    let current = self.project.project.lock().await;
                    let project = current.as_ref().context("no project is open")?;
                    let snapshot = project.snapshot();
                    let label = snapshot.page(page)?.page()?.label;
                    (label, snapshot)
                };
                let renderer = self.desktop.renderer();
                let rasterizer = self.desktop.rasterizer().await?;
                let bytes =
                    output::rendered_preview(&renderer, rasterizer, &snapshot, page).await?;
                Ok(
                    Invocation::read(json!({ "page": page, "label": label }))?.with_image(
                        format!("Rendered page {label} ({page})"),
                        format!("data:image/webp;base64,{}", STANDARD.encode(bytes)),
                    ),
                )
            }
            "rename_page" => {
                let arguments: RenamePage = arguments(&call)?;
                let page = entity(&arguments.page)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.rename_page(page, arguments.label).await?,
                            json!({ "page": page }),
                        ))
                    })
                })
                .await
            }
            "move_page" => {
                let arguments: MovePage = arguments(&call)?;
                let page = entity(&arguments.page)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.move_page(page, arguments.index).await?,
                            json!({ "page": page }),
                        ))
                    })
                })
                .await
            }
            "delete_pages" => {
                let arguments: DeletePages = arguments(&call)?;
                let pages = entities(&arguments.pages)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.delete_pages(pages.clone()).await?,
                            json!({ "pages": pages }),
                        ))
                    })
                })
                .await
            }
            "add_text_box" => {
                let arguments: AddTextBox = arguments(&call)?;
                let page = entity(&arguments.page)?;
                let frame = Frame {
                    x: arguments.x,
                    y: arguments.y,
                    width: arguments.width,
                    height: arguments.height,
                    angle_degrees: arguments.angle_degrees,
                };
                self.mutate(|project| {
                    Box::pin(async move {
                        let (commit, element) = project.add_text_box(page, frame).await?;
                        Ok((commit, json!({ "page": page, "element": element })))
                    })
                })
                .await
            }
            "set_source_text" => {
                let arguments: SetText = arguments(&call)?;
                let element = entity(&arguments.element)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.set_source_text(element, arguments.text).await?,
                            json!({ "element": element }),
                        ))
                    })
                })
                .await
            }
            "set_translation" => {
                let arguments: SetTranslation = arguments(&call)?;
                let element = entity(&arguments.element)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.set_translation(element, arguments.text).await?,
                            json!({ "element": element }),
                        ))
                    })
                })
                .await
            }
            "set_typography" => {
                let arguments: SetTypography = arguments(&call)?;
                let element = entity(&arguments.element)?;
                let typography = Typography {
                    preferred_font: arguments.preferred_font,
                    font_weight: arguments.font_weight,
                    font_style: arguments.font_style.map(Into::into),
                    size: arguments.size,
                    auto_fit: arguments.auto_fit,
                    color: arguments.color,
                    stroke_color: arguments.stroke_color,
                    stroke_width: arguments.stroke_width,
                    alignment: arguments.alignment.map(Into::into),
                    writing_mode: arguments.writing_mode.map(Into::into),
                };
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project
                                .set_typography(vec![TypographyUpdate {
                                    layer: element,
                                    typography,
                                }])
                                .await?,
                            json!({ "element": element }),
                        ))
                    })
                })
                .await
            }
            "set_geometry" => {
                let arguments: SetGeometry = arguments(&call)?;
                let element = entity(&arguments.element)?;
                let points = arguments
                    .points
                    .into_iter()
                    .map(|point| Point {
                        x: point.x,
                        y: point.y,
                    })
                    .collect();
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project
                                .set_geometry(vec![GeometryUpdate {
                                    layer: element,
                                    points: Some(points),
                                }])
                                .await?,
                            json!({ "element": element }),
                        ))
                    })
                })
                .await
            }
            "set_visibility" => {
                let arguments: SetVisibility = arguments(&call)?;
                let elements = entities(&arguments.elements)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project
                                .set_visibility(
                                    elements.clone(),
                                    arguments.visible,
                                    arguments.opacity,
                                )
                                .await?,
                            json!({ "elements": elements }),
                        ))
                    })
                })
                .await
            }
            "delete_elements" => {
                let arguments: DeleteElements = arguments(&call)?;
                let elements = entities(&arguments.elements)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.delete_layers(elements.clone()).await?,
                            json!({ "elements": elements }),
                        ))
                    })
                })
                .await
            }
            "move_element" => {
                let arguments: MoveElement = arguments(&call)?;
                let element = entity(&arguments.element)?;
                let parent = entity(&arguments.parent)?;
                self.mutate(|project| {
                    Box::pin(async move {
                        Ok((
                            project.move_layer(element, parent, arguments.index).await?,
                            json!({ "element": element, "parent": parent }),
                        ))
                    })
                })
                .await
            }
            "run_pipeline" => self.run_pipeline(arguments(&call)?, control).await,
            name => bail!("unknown Koharu tool {name}"),
        }
    }
}

struct AgentCommitter {
    host: KoharuHost,
}

#[async_trait]
impl Committer for AgentCommitter {
    async fn commit(&mut self, output: StageOutput) -> Result<Snapshot> {
        let (commit, page) = {
            let mut current = self.host.project.project.lock().await;
            let project = current.as_mut().context("no project is open")?;
            let Some(commit) = project.commit_rebased(output.patch).await? else {
                return Ok(project.snapshot());
            };
            project.record_commit(&commit);
            (commit, project.active_page())
        };
        let snapshot = commit.snapshot.clone();
        self.host.synchronize(commit, page).await?;
        Ok(snapshot)
    }
}

fn definition<T: JsonSchema>(name: &str, description: &str) -> Tool {
    Tool::new(
        name,
        description,
        serde_json::to_value(schema_for!(T)).expect("tool argument schema must serialize"),
    )
}

fn arguments<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> Result<T> {
    serde_json::from_str(&call.arguments)
        .with_context(|| format!("invalid arguments for {}", call.name))
}

fn entity(value: &str) -> Result<EntityId> {
    serde_json::from_value(Value::String(value.to_owned()))
        .with_context(|| format!("invalid entity ID {value}"))
}

fn entities(values: &[String]) -> Result<Vec<EntityId>> {
    values.iter().map(|value| entity(value)).collect()
}

#[derive(Deserialize, JsonSchema)]
struct InspectProject {}

#[derive(Deserialize, JsonSchema)]
struct ViewPage {
    page: String,
}

#[derive(Deserialize, JsonSchema)]
struct RenamePage {
    page: String,
    label: String,
}

#[derive(Deserialize, JsonSchema)]
struct MovePage {
    page: String,
    index: usize,
}

#[derive(Deserialize, JsonSchema)]
struct DeletePages {
    pages: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AddTextBox {
    page: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    angle_degrees: f32,
}

#[derive(Deserialize, JsonSchema)]
struct SetText {
    element: String,
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct SetTranslation {
    element: String,
    text: Option<String>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AgentTextAlignment {
    Start,
    Center,
    End,
    Justify,
}

impl From<AgentTextAlignment> for koharu_scene::TextAlignment {
    fn from(value: AgentTextAlignment) -> Self {
        match value {
            AgentTextAlignment::Start => Self::Start,
            AgentTextAlignment::Center => Self::Center,
            AgentTextAlignment::End => Self::End,
            AgentTextAlignment::Justify => Self::Justify,
        }
    }
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AgentWritingMode {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AgentFontStyle {
    Normal,
    Italic,
    Oblique,
}

impl From<AgentFontStyle> for koharu_scene::FontStyle {
    fn from(value: AgentFontStyle) -> Self {
        match value {
            AgentFontStyle::Normal => Self::Normal,
            AgentFontStyle::Italic => Self::Italic,
            AgentFontStyle::Oblique => Self::Oblique,
        }
    }
}

impl From<AgentWritingMode> for koharu_scene::WritingMode {
    fn from(value: AgentWritingMode) -> Self {
        match value {
            AgentWritingMode::Horizontal => Self::Horizontal,
            AgentWritingMode::Vertical => Self::Vertical,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct SetTypography {
    element: String,
    preferred_font: Option<String>,
    font_weight: Option<u16>,
    font_style: Option<AgentFontStyle>,
    size: Option<f32>,
    auto_fit: bool,
    color: Option<[u8; 4]>,
    stroke_color: Option<[u8; 4]>,
    stroke_width: Option<f32>,
    alignment: Option<AgentTextAlignment>,
    writing_mode: Option<AgentWritingMode>,
}

#[derive(Deserialize, JsonSchema)]
struct SetGeometry {
    element: String,
    points: Vec<AgentPoint>,
}

#[derive(Deserialize, JsonSchema)]
struct AgentPoint {
    x: f64,
    y: f64,
}

#[derive(Deserialize, JsonSchema)]
struct SetVisibility {
    elements: Vec<String>,
    visible: Option<bool>,
    opacity: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteElements {
    elements: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct MoveElement {
    element: String,
    parent: String,
    index: usize,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AgentPipelineOperation {
    Full,
    Detection,
    Ocr,
    Translation,
    Inpainting,
}

impl From<AgentPipelineOperation> for Operation {
    fn from(value: AgentPipelineOperation) -> Self {
        match value {
            AgentPipelineOperation::Full => Self::Full,
            AgentPipelineOperation::Detection => Self::Only {
                stage: Stage::Detection,
            },
            AgentPipelineOperation::Ocr => Self::Only { stage: Stage::Ocr },
            AgentPipelineOperation::Translation => Self::Only {
                stage: Stage::Translation,
            },
            AgentPipelineOperation::Inpainting => Self::Only {
                stage: Stage::Inpainting,
            },
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct RunPipeline {
    operation: AgentPipelineOperation,
    #[serde(default)]
    pages: Vec<String>,
    #[serde(default)]
    elements: Vec<String>,
}

impl RunPipeline {
    fn scope(&self) -> Result<Scope> {
        match (self.pages.is_empty(), self.elements.is_empty()) {
            (true, true) => Ok(Scope::Project),
            (false, true) => Ok(Scope::Pages(entities(&self.pages)?)),
            (true, false) => Ok(Scope::Entities(entities(&self.elements)?)),
            (false, false) => bail!("pipeline scope cannot contain both pages and elements"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    use super::*;
    use crate::commands::{Channel, processing::Job};

    #[tokio::test]
    async fn agent_pipeline_job_uses_shared_channel_across_reconnect() {
        let processing = Processing::default();
        let jobs = JobChannel::default();
        let (first_tx, first_rx) = mpsc::channel();
        *jobs.channel.lock() = Some(Channel::<Job>::from_sink(move |job| {
            first_tx.send(job).is_ok()
        }));
        let host = KoharuHost::new(
            CurrentProject {
                project: Arc::new(tokio::sync::Mutex::new(None)),
            },
            Desktop::new().unwrap(),
            CanvasChannel::default(),
            processing.clone(),
            jobs.clone(),
            Pipeline::empty(),
        );

        let job = host.start_pipeline_job().unwrap();
        let running = first_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(running["kind"], "pipeline");
        assert_eq!(running["state"], "running");
        let snapshot = processing.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(running["id"], serde_json::to_value(snapshot[0].id).unwrap());
        assert!(
            processing
                .start_job(JobKind::GlossaryScan, JobPhase::PreparingOcr, &jobs)
                .is_err()
        );

        let (reconnected_tx, reconnected_rx) = mpsc::channel();
        *jobs.channel.lock() = Some(Channel::<Job>::from_sink(move |job| {
            reconnected_tx.send(job).is_ok()
        }));
        let snapshot = processing.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].state, JobState::Running);

        job.update(1, 2, None, Some(Stage::Ocr));
        let progress = reconnected_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(progress["id"], running["id"]);
        assert_eq!(progress["completed"], 1);
        assert_eq!(progress["total"], 2);
        assert_eq!(progress["phase"]["kind"], "pipeline");
        assert_eq!(progress["phase"]["stage"], "ocr");

        job.finish(JobState::Finished, None);
        let finished = reconnected_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(finished["id"], running["id"]);
        assert_eq!(finished["state"], "finished");
        assert!(processing.snapshot().is_empty());
    }
}
