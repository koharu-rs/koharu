//! Koharu's tools on the open project, as rig's own tools: the scene's
//! queries but its undo history, which an agent can't act on, an edit of the
//! scene's ops, the pipeline's run, wait, and stop, and a look at a page. The in-app agent registers them with rig,
//! and the MCP server serves the same set to outside agents. A call finds
//! the open project in the tool context it runs with.

use std::{collections::BTreeSet, io::Cursor, sync::Arc, time::Duration};

use base64::Engine as _;
use image::{
    DynamicImage, ImageFormat, Rgba, RgbaImage,
    imageops::{self, FilterType},
};
use koharu_pipeline::{Event, Pipeline, Request, RunId, RunPriority, Stage, Target};
use koharu_renderer::Renderer;
use koharu_scene::{Bounds, Change, LayerId, Op, PageId, Query, Session, Transform};
use kurbo::{BezPath, Shape as _};
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
use serde_json::{Value, json};

/// How long `wait` waits for a run to end, so an agent following a run
/// doesn't spend a turn on every look, as Codex waits on a long command for
/// up to 30 seconds before it answers.
const WAIT: Duration = Duration::from_secs(30);

/// The longer side of what `view_page` shows, as translation's sight is.
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
        tools.add_tool(Edit);
        tools.add_tool(Run);
        tools.add_tool(Wait);
        tools.add_tool(Stop);
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
        let mut project = serde_json::to_value(answer)?;
        outline(&mut project);
        Ok(project)
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
    /// Queue priority: `normal` queues behind active batch work, while
    /// `rerun_now` jumps ahead at the next page/stage boundary.
    #[serde(default)]
    priority: RunPriority,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    /// The run, as `run` answered it.
    run: RunId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StopArgs {
    /// The run to stop, as `run` answered it; leave out to stop every run.
    #[serde(default)]
    run: Option<RunId>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewArgs {
    /// The page.
    page: PageId,
    /// Look at the page's scan alone, as it was imported, without its layers:
    /// what a block's source says, whether or not it's been cleaned up.
    #[serde(default)]
    original: bool,
    /// The part of the page to look at, in page pixels, such as a block's
    /// source bounds with some room around them; the whole page when left
    /// out.
    region: Option<Bounds>,
}

// schemars leaves `properties` out of a struct without fields, and LM Studio
// refuses a request when one of its tools has none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("properties" = {}))]
struct NoArgs {}

struct GetProject;

impl Tool for GetProject {
    const NAME: &'static str = "get_project";
    type Args = NoArgs;
    type Output = Value;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "The project's pages in order, each with how many blocks it has and how many are translated."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<NoArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, context: &mut ToolContext, _: NoArgs) -> Result<Value, Self::Error> {
        query(context, Query::GetProject, outline).await
    }
}

struct GetPage;

impl Tool for GetPage {
    const NAME: &'static str = "get_page";
    type Args = PageArgs;
    type Output = Value;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "A page with its size, its scan, and its layers nested in reading order: panels, \
         balloons, and blocks, each block with its source and translation. A path is shown by \
         its bounds; get_layer has it, with a layer's attributes, its mask, and its clean-up."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<PageArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, context: &mut ToolContext, args: PageArgs) -> Result<Value, Self::Error> {
        let page = Query::GetPage { page: args.page };
        query(context, page, |page| sparse(&mut page["layers"])).await
    }
}

struct GetLayer;

impl Tool for GetLayer {
    const NAME: &'static str = "get_layer";
    type Args = LayerArgs;
    type Output = Value;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "One layer with all its values, and what's inside it as get_page shows it.".to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<LayerArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, context: &mut ToolContext, args: LayerArgs) -> Result<Value, Self::Error> {
        let layer = Query::GetLayer { layer: args.layer };
        query(context, layer, |layer| sparse(&mut layer["children"])).await
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
        "Run the pipeline's stages on the project, some pages, some blocks, a painted area, or \
         project metadata, replacing what they wrote before. Optional `priority` (`normal` or \
         `rerun_now`) controls queue placement. Answers the run's id at once; the run goes on \
         in the background, and wait follows it."
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
            priority: args.priority,
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
    type Args = StopArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Stop one run by its id, or every run (running, queued, and waiting) when left out. What \
         they finished stays."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<StopArgs>()
    }

    async fn call(&self, context: &mut ToolContext, args: StopArgs) -> Result<String, Self::Error> {
        project(context)?.pipeline.cancel(args.run);
        Ok(match args.run {
            Some(run) => format!("Run {} is stopped.", run.0),
            None => "Every run is stopped.".to_owned(),
        })
    }
}

struct Wait;

impl Tool for Wait {
    const NAME: &'static str = "wait";
    type Args = WaitArgs;
    type Output = Vec<Event>;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Wait up to 30 seconds for a run to end, then answer what it told so far, in order: how \
         many steps it does, each page or metadata stage as it started and finished, or failed on \
         that step alone, and last, once it ended, how: done, failed with its reason, or stopped."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<WaitArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: WaitArgs,
    ) -> Result<Vec<Event>, Self::Error> {
        let tools = project(context)?;
        tools.pipeline.history(args.run, WAIT).await.map_err(failed)
    }
}

struct ViewPage;

impl Tool for ViewPage {
    const NAME: &'static str = "view_page";
    type Args = ViewArgs;
    type Output = ToolResultContent;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Look at a page as the editor draws it, its translations and clean-ups included, or at its \
         scan alone, 1,280 pixels on its longer side. A region fills those, so look at one to read \
         small text."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        schema::<ViewArgs>()
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: ViewArgs,
    ) -> Result<ToolResultContent, Self::Error> {
        let tools = project(context)?;
        let jpeg = tokio::task::spawn_blocking(move || {
            let mut frame = tools
                .renderer
                .render(&tools.session.lock(), args.page)
                .map_err(failed)?;
            let mut area = frame.area();
            if let Some(region) = args.region {
                area = area.intersect(region.into());
                if area.is_zero_area() {
                    return Err(failed("the region is outside the page"));
                }
            }
            if args.original {
                frame = frame
                    .background()
                    .ok_or_else(|| failed("the page has no scan"))?
                    .cropped();
            }
            seen(&frame.rasterize_rect(area.expand()).map_err(failed)?)
        })
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

/// `query`'s answer, the project's own values, cut by `cut` to what a model
/// reads them for.
async fn query(
    context: &ToolContext,
    query: Query,
    cut: fn(&mut Value),
) -> Result<Value, ToolExecutionError> {
    let tools = project(context)?;
    tokio::task::spawn_blocking(move || {
        let answer = tools.session.lock().query(query).map_err(failed)?;
        let mut answer = serde_json::to_value(answer).map_err(failed)?;
        cut(&mut answer);
        Ok(answer)
    })
    .await
    .map_err(failed)?
}

/// Takes each page's size and scan out of a project, which the page's own
/// answer has: a scan's hash is half of what a page takes in every request.
fn outline(project: &mut Value) {
    for page in project["pages"].as_array_mut().into_iter().flatten() {
        if let Value::Object(page) = page {
            page.retain(|key, _| !matches!(key.as_str(), "size" | "background"));
        }
    }
}

/// Cuts layers to what each is, where it is, and what it says, as Figma's
/// `get_metadata` answers a layer tree: whole, one page's can take more than a
/// small model's context.
fn sparse(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for key in ["outline", "path"] {
                let path = fields.get(key).and_then(Value::as_str);
                if let Some(Ok(path)) = path.map(BezPath::from_svg) {
                    let bounds = path.bounding_box().expand();
                    let bounds = json!({
                        "x": bounds.x0, "y": bounds.y0,
                        "width": bounds.width(), "height": bounds.height(),
                    });
                    fields.remove(key);
                    fields.insert("bounds".to_owned(), bounds);
                }
            }
            // A span's attributes are its marks, so only a layer's go.
            let layer = fields.contains_key("kind");
            fields.retain(|key, value| match key.as_str() {
                "attributes" | "mask" | "cleanup" if layer => false,
                "transform" => *value != json!(Transform::default()),
                _ => !value.is_null(),
            });
            fields.values_mut().for_each(sparse);
        }
        Value::Array(values) => values.iter_mut().for_each(sparse),
        _ => {}
    }
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

/// `drawn` as the model sees it: over white, scaled to `SIGHT`, as a JPEG in
/// base 64.
fn seen(drawn: &RgbaImage) -> Result<String, ToolExecutionError> {
    let (width, height) = drawn.dimensions();
    let mut view = RgbaImage::from_pixel(width, height, Rgba([255; 4]));
    imageops::overlay(&mut view, drawn, 0, 0);
    let view = DynamicImage::ImageRgba8(view);
    let view = if width.max(height) > SIGHT {
        view.thumbnail(SIGHT, SIGHT)
    } else {
        view.resize(SIGHT, SIGHT, FilterType::CatmullRom)
    };
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(view.to_rgb8())
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Jpeg)
        .map_err(failed)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use image::Rgb;
    use koharu_pipeline::PipelineConfig;
    use koharu_scene::{Background, Created, Hash, Ref};
    use rig_core::completion::message::DocumentSourceKind;
    use serde_json::json;

    use super::*;

    /// Tools on an empty project, with the pipeline and renderer at their
    /// defaults.
    fn open() -> (tempfile::TempDir, Tools) {
        let dir = tempfile::tempdir().expect("a folder");
        let session = Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())
            .expect("a project");
        let tools = Tools::new(
            Arc::new(Mutex::new(session)),
            Pipeline::new(PipelineConfig::default()),
            Renderer::new(),
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
            assert!(
                !schema.contains(r#""$ref":"#),
                "{}",
                definition.name.as_str()
            );
        }
    }

    /// They go with every request, and with a layer's types inlined they took
    /// more than a local model's whole context.
    #[test]
    fn the_tools_leave_a_small_context_room() {
        let definitions = Tools::toolset().tool_definitions();
        let taken: usize = definitions
            .iter()
            .map(|tool| tool.description.len() + tool.parameters.to_string().len())
            .sum();
        assert!(taken < 32_000, "{taken} characters");
    }

    /// What agents never send is skipped by schemars attributes on `Op`.
    #[test]
    fn edit_offers_the_ops_agents_send() {
        let schema = Edit.parameters().to_string();
        for op in [
            "update_layer",
            "set_rich_text",
            "set_attribute",
            "translation",
        ] {
            assert!(schema.contains(op), "{op}");
        }
        for op in ["add_file", "write_pixels", "\"add_page\""] {
            assert!(!schema.contains(op), "{op}");
        }
    }

    #[tokio::test]
    async fn a_page_is_read_without_the_paths_its_layers_are_read_with() {
        let (_dir, tools) = open();
        let page = add_page(&tools, 100, 100);
        let block = json!({"kind": "block", "source": {
            "content": [{"insert": "a", "attributes": {"stroke_width": 8.0}}],
        }});
        let ops = json!([
            {"op": "add_layer", "id": "$balloon", "page": page, "index": 0,
             "layer": {"kind": "balloon", "outline": "M10 20 H50 V40 H10 Z"}},
            {"op": "add_layer", "page": page, "parent": "$balloon", "index": 0, "layer": block},
        ]);
        let change = tools
            .session
            .lock()
            .apply(koharu_scene::Edit {
                label: "test".to_owned(),
                ops: serde_json::from_value(ops).expect("ops"),
            })
            .expect("a balloon and its block");
        let Some(Created::Layer { id: layer, .. }) = change.created.first() else {
            panic!("{change:?}");
        };

        let mut context = tools.tool_context();
        let read = GetPage
            .call(&mut context, PageArgs { page })
            .await
            .expect("the page");
        let balloon = &read["layers"][0];
        let bounds = json!({"x": 10.0, "y": 20.0, "width": 40.0, "height": 20.0});
        assert_eq!(balloon["bounds"], bounds);
        assert!(balloon.get("outline").is_none(), "{balloon}");
        let inside = &balloon["children"][0];
        assert!(inside.get("attributes").is_none(), "{inside}");
        // A span's marks share their name with a layer's attributes.
        let marks = &inside["source"]["content"][0]["attributes"];
        assert_eq!(*marks, json!({"stroke_width": 8.0}));

        let whole = GetLayer
            .call(&mut context, LayerArgs { layer: *layer })
            .await
            .expect("the balloon");
        assert_eq!(whole["outline"], "M10 20 H50 V40 H10 Z");
        assert_eq!(whole["children"][0], *inside);
    }

    #[tokio::test]
    async fn rich_text_is_written_as_the_schema_offers_it_and_read_back() {
        let (_dir, tools) = open();
        let page = add_page(&tools, 100, 100);
        let spans = json!([
            {"insert": "The "},
            {"insert": "Holy Sword", "ruby": "Excalibur", "attributes": {"weight": 700}},
            {"insert": " rises"},
        ]);
        let args = json!({"label": "Letter", "ops": [
            {"op": "add_layer", "id": "$block", "page": page, "index": 0,
             "layer": {"kind": "block", "source": {}}},
            {"op": "set_rich_text", "layer": "$block", "spans": spans},
        ]});
        assert_eq!(refusal(&Edit.parameters(), &args), None);
        let mut context = tools.tool_context();
        Edit.call(&mut context, serde_json::from_value(args).expect("an edit"))
            .await
            .expect("the block is lettered");
        let read = GetPage
            .call(&mut context, PageArgs { page })
            .await
            .expect("the page");
        let content = &read["layers"][0]["translation"]["content"];
        assert_eq!(*content, spans, "{read}");
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
    async fn a_run_is_waited_for_to_its_end() {
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
        let events = Wait
            .call(&mut tools.tool_context(), WaitArgs { run })
            .await
            .expect("the run's events");
        assert_eq!(
            events.last().map(|event| json!(event)),
            Some(json!({"kind": "ended", "run": run, "outcome": {"kind": "done"}}))
        );
    }

    async fn view(tools: &Tools, args: Value) -> Result<DynamicImage, ToolExecutionError> {
        let args = serde_json::from_value(args).expect("a view's arguments");
        let view = ViewPage.call(&mut tools.tool_context(), args).await?;
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
        Ok(image::load_from_memory(&bytes).expect("a JPEG"))
    }

    #[tokio::test]
    async fn a_page_view_fits_in_1280_pixels() {
        let (_dir, tools) = open();
        let page = add_page(&tools, 2000, 3000);
        let view = view(&tools, json!({"page": page})).await.expect("a view");
        assert_eq!(view.width().max(view.height()), 1280);
    }

    /// A block's source is read off the scan, whatever was drawn over it
    /// since.
    #[tokio::test]
    async fn a_view_shows_a_region_of_the_scan_under_the_layers() {
        // Under a red layer, a scan whose left half is green and right blue.
        let (_dir, tools) = open();
        let page = add_page(&tools, 200, 100);
        let region = json!({"x": 100, "y": 20, "width": 60, "height": 120});
        let original = json!({"page": page, "original": true, "region": region});
        let error = view(&tools, original.clone()).await.expect_err("no scan");
        assert_eq!(error.message(), "the page has no scan");
        let scan = image::RgbImage::from_fn(200, 100, |x, _| match x < 100 {
            true => Rgb([0, 255, 0]),
            false => Rgb([0, 0, 255]),
        });
        let mut bytes = Vec::new();
        scan.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .expect("a PNG");
        let background = Background {
            file: Hash::of(&bytes),
            visible: true,
        };
        let layer = json!({"op": "add_layer", "page": page, "index": 0, "layer": {
            "kind": "shape", "path": "M0 0 H200 V100 H0 Z", "fill": [1.0, 0.0, 0.0, 1.0],
        }});
        let ops = vec![
            Op::AddFile { bytes },
            Op::SetBackground {
                page: Ref::from(page),
                background: Some(background),
            },
            serde_json::from_value(layer).expect("an op"),
        ];
        tools
            .session
            .lock()
            .apply(koharu_scene::Edit {
                label: "test".to_owned(),
                ops,
            })
            .expect("a scan under a layer");
        // The middle's strongest channel, as a JPEG keeps it.
        let strongest = |view: &DynamicImage| {
            let pixel = view
                .to_rgb8()
                .get_pixel(view.width() / 2, view.height() / 2)
                .0;
            (0..3).max_by_key(|&channel| pixel[channel])
        };
        let scan = view(&tools, original).await.expect("a view");
        // 60 by the 80 of its 120 the page has.
        assert_eq!((scan.width(), scan.height()), (960, 1280));
        assert_eq!(strongest(&scan), Some(2));
        let drawn = view(&tools, json!({"page": page, "region": region}))
            .await
            .expect("a view");
        assert_eq!(strongest(&drawn), Some(0));
        let outside = json!({"x": 200.5, "y": 0, "width": 10, "height": 10});
        let error = view(&tools, json!({"page": page, "region": outside}))
            .await
            .expect_err("outside");
        assert_eq!(error.message(), "the region is outside the page");
    }

    #[tokio::test]
    async fn run_accepts_priority_and_stop_cancels_single_or_all_runs() {
        let (_dir, tools) = open();
        assert_eq!(
            refusal(
                &Run.parameters(),
                &json!({"target": "project", "stages": ["detect"], "priority": "rerun_now"})
            ),
            None
        );
        assert_eq!(refusal(&Stop.parameters(), &json!({})), None);
        assert_eq!(refusal(&Stop.parameters(), &json!({"run": 1})), None);

        let started = Run
            .call(
                &mut tools.tool_context(),
                serde_json::from_value(
                    json!({"target": "project", "stages": ["detect"], "priority": "rerun_now"}),
                )
                .expect("valid run args"),
            )
            .await
            .expect("run starts");
        let run = RunId(started.parse().expect("numeric run id"));

        let single = Stop
            .call(&mut tools.tool_context(), StopArgs { run: Some(run) })
            .await
            .expect("single stop");
        assert_eq!(single, format!("Run {} is stopped.", run.0));

        let events = Wait
            .call(&mut tools.tool_context(), WaitArgs { run })
            .await
            .expect("wait completes");
        assert!(matches!(
            events.last(),
            Some(Event::Ended {
                outcome: koharu_pipeline::Outcome::Done | koharu_pipeline::Outcome::Stopped,
                ..
            })
        ));

        let all = Stop
            .call(&mut tools.tool_context(), StopArgs { run: None })
            .await
            .expect("stop all");
        assert_eq!(all, "Every run is stopped.");
    }
}
