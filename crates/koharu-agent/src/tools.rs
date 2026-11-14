//! Koharu's tools on the open project, as rig's own tools: the scene's
//! queries, an edit of the scene's ops, the pipeline's run, stop, and
//! history, and a look at a page. The in-app agent registers them with rig,
//! and the MCP server serves the same set to outside agents. A call finds
//! the open project in the tool context it runs with.

use std::{collections::BTreeSet, io::Cursor, sync::Arc, time::Duration};

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage, imageops};
use koharu_pipeline::{Event, Pipeline, Request, RunId, Stage, Target};
use koharu_renderer::{Frame, Renderer};
use koharu_scene::{Answer, Change, LayerId, Op, PageId, Query, Session};
use parking_lot::Mutex;
use rig_agent::tool::ToolSet;
use rig_core::{
    completion::{
        ToolAnnotations,
        message::{ImageMediaType, ToolResultContent},
    },
    tool::{Tool, ToolContext, ToolExecutionError, validate::validate},
};
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::Deserialize;
use serde_json::Value;

/// How long `history` waits for a run to end, so an agent following a run
/// doesn't spend a turn on every look, as Codex waits on a long command for
/// up to 30 seconds before it answers.
const WAIT: Duration = Duration::from_secs(30);

/// The longest side of a page `view_page` shows, as translation's sight is.
const SIGHT: u32 = 1280;

/// What the tools work on: one open project, with the pipeline that runs its
/// stages and the renderer that draws its pages.
#[derive(Clone)]
pub struct Tools {
    session: Arc<Mutex<Session>>,
    pipeline: Pipeline,
    renderer: Renderer,
}

impl Tools {
    /// The tools on `session`, running stages on `pipeline` and drawing pages
    /// with `renderer`.
    pub fn new(session: Arc<Mutex<Session>>, pipeline: Pipeline, renderer: Renderer) -> Self {
        Self {
            session,
            pipeline,
            renderer,
        }
    }

    /// Every tool, the same whichever project is open.
    pub fn toolset() -> ToolSet {
        let mut tools = ToolSet::default();
        tools.add_tool(GetProject);
        tools.add_tool(GetPage);
        tools.add_tool(GetLayer);
        tools.add_tool(GetHistory);
        tools.add_tool(Edit);
        tools.add_tool(Run);
        tools.add_tool(Stop);
        tools.add_tool(History);
        tools.add_tool(ViewPage);
        tools
    }

    /// The context a call on this project runs with.
    pub fn tool_context(&self) -> ToolContext {
        ToolContext::default().with_scope(Arc::new(self.clone()))
    }

    /// The project's outline, which goes with each of the agent's requests.
    pub fn context(&self) -> anyhow::Result<Value> {
        let answer = self.session.lock().query(Query::GetProject)?;
        Ok(serde_json::to_value(answer)?)
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageArgs {
    /// The page.
    page: PageId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayerArgs {
    /// The layer to read.
    layer: LayerId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditArgs {
    /// What the edit does, for the undo menu.
    label: String,
    /// The ops, applied in order. A later op can refer to a page or layer an
    /// earlier one made by the `$label` it gave as `id`.
    ops: Vec<Op>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    /// What the run works on.
    target: Target,
    /// The stages to run; each page's run in the order detect, recognize,
    /// inpaint, translate.
    stages: BTreeSet<Stage>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HistoryArgs {
    /// The run, as `run` answered it.
    run: RunId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

struct GetProject;

impl Tool for GetProject {
    const NAME: &'static str = "get_project";
    type Args = NoArgs;
    type Output = Answer;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "The project's pages in order, each with its size and scan.".to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<NoArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, context: &mut ToolContext, _: NoArgs) -> Result<Answer, Self::Error> {
        query(context, Query::GetProject).await
    }
}

struct GetPage;

impl Tool for GetPage {
    const NAME: &'static str = "get_page";
    type Args = PageArgs;
    type Output = Answer;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "A page with its layers nested in reading order: panels, balloons, and blocks, each \
         block with its source and translation."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<PageArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, context: &mut ToolContext, args: PageArgs) -> Result<Answer, Self::Error> {
        query(context, Query::GetPage { page: args.page }).await
    }
}

struct GetLayer;

impl Tool for GetLayer {
    const NAME: &'static str = "get_layer";
    type Args = LayerArgs;
    type Output = Answer;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "One layer with everything inside it.".to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<LayerArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: LayerArgs,
    ) -> Result<Answer, Self::Error> {
        query(context, Query::GetLayer { layer: args.layer }).await
    }
}

struct GetHistory;

impl Tool for GetHistory {
    const NAME: &'static str = "get_history";
    type Args = NoArgs;
    type Output = Answer;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "The project's undo steps, oldest first, and how many of them are done.".to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<NoArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, context: &mut ToolContext, _: NoArgs) -> Result<Answer, Self::Error> {
        query(context, Query::GetHistory).await
    }
}

struct Edit;

impl Tool for Edit {
    const NAME: &'static str = "edit";
    type Args = EditArgs;
    type Output = Change;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Change the project with the scene's ops, all of them as one undo step, or none if one \
         fails. Answers what changed."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<EditArgs>()
    }

    async fn call(&self, context: &mut ToolContext, args: EditArgs) -> Result<Change, Self::Error> {
        let tools = project(context)?;
        let edit = koharu_scene::Edit {
            label: args.label,
            ops: args.ops,
        };
        // On a blocking thread, an edit already applying finishes even when
        // the call is dropped, and it applies whole or not at all.
        tokio::task::spawn_blocking(move || tools.session.lock().apply(edit).map_err(failed))
            .await
            .map_err(failed)?
    }
}

struct Run;

impl Tool for Run {
    const NAME: &'static str = "run";
    type Args = RunArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Run the pipeline's stages on the project, some pages, some blocks, or a painted area, \
         replacing what they wrote before. Answers the run's id at once; the run goes on in the \
         background, and history follows it."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<RunArgs>()
    }

    async fn call(&self, context: &mut ToolContext, args: RunArgs) -> Result<String, Self::Error> {
        let tools = project(context)?;
        let request = Request {
            target: args.target,
            stages: args.stages,
            config: None,
        };
        let run = tools
            .pipeline
            .run(Arc::clone(&tools.session), request)
            .map_err(failed)?;
        Ok(run.0.to_string())
    }
}

struct Stop;

impl Tool for Stop {
    const NAME: &'static str = "stop";
    type Args = NoArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Stop every run, running and queued. What they finished stays.".to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<NoArgs>()
    }

    async fn call(&self, context: &mut ToolContext, _: NoArgs) -> Result<String, Self::Error> {
        project(context)?.pipeline.stop();
        Ok("Every run is stopped.".to_owned())
    }
}

struct History;

impl Tool for History {
    const NAME: &'static str = "history";
    type Args = HistoryArgs;
    type Output = Vec<Event>;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "What a run told so far, in order: how many page stages it does, each page's stages as \
         they started and finished, and last, once it ended, how: done, failed with its reason, \
         or stopped. Waits up to 30 seconds for the run to end first."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<HistoryArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: HistoryArgs,
    ) -> Result<Vec<Event>, Self::Error> {
        let tools = project(context)?;
        tools.pipeline.history(args.run, WAIT).await.map_err(failed)
    }
}

struct ViewPage;

impl Tool for ViewPage {
    const NAME: &'static str = "view_page";
    type Args = PageArgs;
    type Output = ToolResultContent;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Look at a page as the editor draws it, its translations and clean-ups included, at most \
         1,280 pixels on its longer side."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<PageArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: PageArgs,
    ) -> Result<ToolResultContent, Self::Error> {
        let tools = project(context)?;
        let frame = tools
            .renderer
            .render_loaded(Arc::clone(&tools.session), args.page)
            .await
            .map_err(failed)?;
        let renderer = tools.renderer.clone();
        let jpeg = tokio::task::spawn_blocking(move || seen(&renderer, &frame))
            .await
            .map_err(failed)??;
        Ok(ToolResultContent::image_base64(
            jpeg,
            Some(ImageMediaType::JPEG),
            None,
        ))
    }
}

/// The open project the call runs on.
fn project(context: &ToolContext) -> Result<Arc<Tools>, ToolExecutionError> {
    context
        .scope::<Tools>()
        .ok_or_else(|| ToolExecutionError::other("no project is open"))
}

/// `query`'s answer, the project's own values.
async fn query(context: &ToolContext, query: Query) -> Result<Answer, ToolExecutionError> {
    let tools = project(context)?;
    tokio::task::spawn_blocking(move || tools.session.lock().query(query).map_err(failed))
        .await
        .map_err(failed)?
}

/// `error` as the model sees it, word for word: rig shows a model only the
/// kind of an error it wasn't handed as a tool error.
fn failed(error: impl ToString) -> ToolExecutionError {
    ToolExecutionError::other(error.to_string())
}

/// What the model is told when `arguments` don't fit `schema`.
pub(crate) fn refusal(schema: &Value, arguments: &Value) -> Option<String> {
    let problems = validate(schema, arguments).err()?;
    let problems = problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "the arguments don't fit the tool's schema: {problems}"
    ))
}

/// `T`'s schema with every subschema inlined: some providers refuse a tool
/// schema that refers to definitions.
fn schema<T: JsonSchema>() -> Value {
    let schema = SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(schema).unwrap_or_default()
}

/// `frame` as the model sees it: over white, fit to `SIGHT`, as a JPEG in
/// base 64.
fn seen(renderer: &Renderer, frame: &Frame) -> Result<String, ToolExecutionError> {
    let raster = renderer.rasterize(frame).map_err(failed)?;
    let (width, height) = raster.image.dimensions();
    let mut page = RgbaImage::from_pixel(width, height, Rgba([255; 4]));
    imageops::overlay(&mut page, &raster.image, 0, 0);
    let mut page = DynamicImage::ImageRgba8(page);
    if width.max(height) > SIGHT {
        page = page.thumbnail(SIGHT, SIGHT);
    }
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(page.to_rgb8())
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Jpeg)
        .map_err(failed)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use koharu_pipeline::PipelineConfig;
    use koharu_renderer::RendererConfig;
    use koharu_scene::Created;
    use rig_core::completion::message::DocumentSourceKind;
    use serde_json::json;

    use super::*;

    /// Tools on an empty project, with the pipeline and renderer at their
    /// defaults.
    fn open() -> (tempfile::TempDir, Tools) {
        let dir = tempfile::tempdir().expect("a folder");
        let session = Session::create(dir.path().join("test.khr")).expect("a project");
        let tools = Tools::new(
            Arc::new(Mutex::new(session)),
            Pipeline::new(PipelineConfig::default()),
            Renderer::new(RendererConfig::default()),
        );
        (dir, tools)
    }

    fn add_page(tools: &Tools, width: u32, height: u32) -> PageId {
        let op = json!({"op": "add_page", "index": 0, "size": {"width": width, "height": height}});
        let change = tools
            .session
            .lock()
            .apply(koharu_scene::Edit {
                label: "test".to_owned(),
                ops: vec![serde_json::from_value(op).expect("an op")],
            })
            .expect("a page");
        match change.created.first() {
            Some(Created::Page { id, .. }) => *id,
            _ => panic!("{change:?}"),
        }
    }

    /// Some providers refuse a tool schema that refers to definitions.
    #[test]
    fn every_tool_schema_is_inlined() {
        for definition in Tools::toolset().tool_definitions() {
            let schema = definition.parameters.to_string();
            assert!(!schema.contains("$ref"), "{}", definition.name.as_str());
        }
    }

    /// What agents never send is skipped by schemars attributes on `Op`.
    #[test]
    fn edit_offers_the_ops_agents_send() {
        let schema = Edit.parameters().to_string();
        for op in ["update_layer", "set_text", "set_attribute", "translation"] {
            assert!(schema.contains(op), "{op}");
        }
        for op in ["add_file", "write_pixels", "set_rich_text", "\"add_page\""] {
            assert!(!schema.contains(op), "{op}");
        }
    }

    #[tokio::test]
    async fn an_edit_is_one_undo_step_under_its_label() {
        let (_dir, tools) = open();
        let page = add_page(&tools, 100, 100);
        let before = tools.session.lock().history().steps.len();
        let layer =
            json!({"op": "add_layer", "page": page, "index": 0, "layer": {"kind": "raster"}});
        let args = json!({"label": "Add rasters", "ops": [layer, layer]});
        Edit.call(
            &mut tools.tool_context(),
            serde_json::from_value(args).expect("an edit"),
        )
        .await
        .expect("the layers are added");
        let history = tools.session.lock().history();
        assert_eq!(history.steps.len(), before + 1);
        assert_eq!(
            history.steps.last().map(String::as_str),
            Some("Add rasters")
        );
    }

    #[tokio::test]
    async fn a_run_is_followed_through_its_history() {
        // The project has no pages, so detection loads no model.
        let (_dir, tools) = open();
        let args = json!({"target": "project", "stages": ["detect"]});
        let started = Run
            .call(
                &mut tools.tool_context(),
                serde_json::from_value(args).expect("a run"),
            )
            .await
            .expect("the run starts");
        let run = RunId(started.parse().expect("the run's id"));
        let events = History
            .call(&mut tools.tool_context(), HistoryArgs { run })
            .await
            .expect("a history");
        assert_eq!(
            events.last().map(|event| json!(event)),
            Some(json!({"kind": "ended", "run": run, "outcome": {"kind": "done"}}))
        );
    }

    #[tokio::test]
    async fn a_page_view_fits_in_1280_pixels() {
        let (_dir, tools) = open();
        let page = add_page(&tools, 2000, 3000);
        let view = ViewPage
            .call(&mut tools.tool_context(), PageArgs { page })
            .await
            .expect("a view");
        let ToolResultContent::Image(image) = view else {
            panic!("{view:?}");
        };
        assert_eq!(image.media_type, Some(ImageMediaType::JPEG));
        let DocumentSourceKind::Base64(data) = &image.data else {
            panic!("{:?}", image.data);
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("base 64");
        let decoded = image::load_from_memory(&bytes).expect("a JPEG");
        assert_eq!(decoded.width().max(decoded.height()), 1280);
    }
}
