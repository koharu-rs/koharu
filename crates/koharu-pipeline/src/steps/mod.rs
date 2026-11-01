//! The five steps: what each reads from the project, the model work it does
//! on the lane or beside it, and the edit it writes.

mod detection;
mod inpainting;
mod ocr;
mod translation;
mod typesetting;

use anyhow::{Result, bail};
use image::GrayImage;
use imageproc::{
    contours::{BorderType, find_contours_with_threshold},
    geometry::{approximate_polygon_dp, arc_length, contour_area},
};
use koharu_scene::{
    Action, Author, Edit, File, Frame, Layer, LayerId, LayerView, Op, PageId, PageView, PointerBuf,
    ProjectId, Read, Rect, Relationship, Session, SoftwareAgent,
};
use kurbo::{Affine, BezPath, Point, Shape};
use serde_json::{Map, Value};
use tokio::task::block_in_place;

pub use detection::KoharuLayoutRFDetrSeg2XLConfig;
pub use inpainting::{Flux2KleinConfig, RoremMixedConfig};
pub(crate) use typesetting::{LINE_HEIGHT, TYPESETTER};

use crate::{PipelineConfig, Step, residency::Loaded};

/// What part of a page a run covers.
#[derive(Clone, Debug)]
pub(crate) enum Scope {
    Page,
    /// Only these text blocks.
    Layers(Vec<LayerId>),
    /// Only inside this rectangle.
    Area(Rect),
}

/// What every step of a run reads its settings from.
pub(crate) struct Context {
    pub(crate) project: ProjectId,
    pub(crate) config: PipelineConfig,
    pub(crate) redo: bool,
    /// The project's "Dialogue" style, which every translation names; set
    /// when the run typesets.
    pub(crate) dialogue: Option<koharu_scene::StyleId>,
}

/// A page's step as read from the project: the work to do, or why there is
/// none.
pub(crate) enum Prepared<W> {
    Skip(String),
    Work(W),
}

impl<W> Prepared<W> {
    fn map<U>(self, work: impl FnOnce(W) -> U) -> Prepared<U> {
        match self {
            Self::Skip(reason) => Prepared::Skip(reason),
            Self::Work(input) => Prepared::Work(work(input)),
        }
    }
}

pub(crate) enum Work {
    Detection(detection::Work),
    Ocr(ocr::Work),
    Translation(translation::Work),
    Typesetting(typesetting::Work),
    Inpainting(inpainting::Work),
}

/// Reads what `step` needs on `page`, under the host's lock.
pub(crate) fn read(
    session: &Session,
    step: Step,
    page: PageId,
    scope: &Scope,
    context: &Context,
) -> Result<Prepared<Work>> {
    let view = session.page(page)?;
    Ok(match step {
        Step::Detection => detection::read(session, &view, scope, context)?.map(Work::Detection),
        Step::Ocr => ocr::read(session, &view, scope, context)?.map(Work::Ocr),
        Step::Translation => {
            translation::read(session, &view, scope, context)?.map(Work::Translation)
        }
        Step::Typesetting => {
            typesetting::read(session, &view, scope, context)?.map(Work::Typesetting)
        }
        Step::Inpainting => inpainting::read(session, &view, scope, context)?.map(Work::Inpainting),
    })
}

/// Runs one lane batch on `model`, every work of one step: one edit per work,
/// in order. libtorch and stable-diffusion.cpp block, so they run in place on
/// a worker of the multi-threaded runtime.
pub(crate) async fn on_lane(
    model: &Loaded,
    works: Vec<Work>,
    context: &Context,
) -> Vec<Result<Edit>> {
    macro_rules! take {
        ($variant:ident) => {
            works
                .into_iter()
                .map(|work| match work {
                    Work::$variant(work) => work,
                    _ => unreachable!("a batch holds one step's work"),
                })
                .collect()
        };
    }
    match model {
        Loaded::Layout(model) => {
            block_in_place(|| detection::run(model, take!(Detection), context))
        }
        Loaded::MangaOcr(_)
        | Loaded::BaberuOcr(_)
        | Loaded::HayaiOcr(_)
        | Loaded::PaddleOcrVl(_) => ocr::run(model, take!(Ocr), context).await,
        Loaded::Translator(model) => translation::run(model, take!(Translation), context).await,
        Loaded::LaMa(_) | Loaded::Aot(_) | Loaded::Flux2Klein(_) | Loaded::RoremMixed(_) => {
            block_in_place(|| inpainting::run(model, take!(Inpainting), context))
        }
    }
}

/// Runs a step off the lane: typesetting on the CPU, a hosted translation on
/// the network.
pub(crate) async fn off_lane(
    work: Work,
    context: &Context,
    hosted: Option<&koharu_llm::Model>,
) -> Result<Edit> {
    match work {
        Work::Typesetting(work) => {
            let (project, redo) = (context.project, context.redo);
            let (sender, receiver) = tokio::sync::oneshot::channel();
            rayon::spawn(move || drop(sender.send(typesetting::run(project, redo, work))));
            receiver.await?
        }
        Work::Translation(work) => {
            let Some(model) = hosted else {
                bail!("a hosted translation has no model");
            };
            translation::run(model, vec![work], context)
                .await
                .pop()
                .expect("one work gives one edit")
        }
        Work::Detection(_) | Work::Ocr(_) | Work::Inpainting(_) => {
            bail!("only typesetting and hosted translation run off the lane")
        }
    }
}

/// One step's edit on one page, as `action`.
fn edit(project: ProjectId, step: Step, action: Action, reads: Vec<Read>, ops: Vec<Op>) -> Edit {
    Edit {
        project,
        label: step.to_string(),
        author: Author::Model { action },
        reads,
        ops,
    }
}

/// A C2PA action by `agent`.
fn action(action: &str, agent: &str, parameters: Map<String, Value>) -> Action {
    Action {
        action: action.to_owned(),
        software_agent: SoftwareAgent {
            name: agent.to_owned(),
            version: None,
        },
        digital_source_type: None,
        parameters,
    }
}

/// Every layer on a page, parents first.
fn walk(layers: &[LayerView]) -> Vec<&LayerView> {
    layers
        .iter()
        .flat_map(|view| {
            std::iter::once(view)
                .chain(walk(&view.children))
                .chain(walk(&view.masks))
        })
        .collect()
}

/// Whether `agent` created the layer: only creating one writes all of it.
fn made_by(view: &LayerView, agent: &str) -> bool {
    records(view, agent).any(|record| record.writes.iter().any(|path| path.is_root()))
}

/// Whether something `agent` wrote on the layer is out of date.
fn out_of_date(view: &LayerView, agent: &str) -> bool {
    records(view, agent).any(|record| record.writes.iter().any(|path| view.stale.contains(path)))
}

fn records<'a>(
    view: &'a LayerView,
    agent: &'a str,
) -> impl Iterator<Item = &'a koharu_scene::ActionRecord> {
    view.origin
        .iter()
        .filter(move |record| record.action.software_agent.name == agent)
}

/// A read of all of a layer as the page showed it, so an edit that replaces
/// the layer goes stale if anything changed it since.
fn whole(view: &LayerView) -> Read {
    Read {
        layer: view.id,
        path: PointerBuf::new(),
        hash: view.hash.clone(),
        relationship: Relationship::InputTo,
    }
}

/// The page's scan: its first top-level image.
fn scan(view: &PageView) -> Option<usize> {
    view.layers
        .iter()
        .position(|layer| matches!(layer.layer, Layer::Image(_)))
}

/// The scan's file, with a read of its `/file` as what a result derives
/// from.
fn scan_file(session: &Session, scan: LayerId) -> Result<(File, Read)> {
    let (file, read) = session.read(scan, koharu_scene::Pointer::from_static("/file"))?;
    let file = session.file(&serde_json::from_value(file)?)?;
    Ok((file, read.parent_of()))
}

/// The action a translation's text names.
const TRANSLATED: &str = "c2pa.translated";

/// The page's text blocks: the texts detection made, in reading order.
fn blocks<'a>(layers: &[&'a LayerView]) -> Vec<&'a LayerView> {
    layers
        .iter()
        .copied()
        .filter(|layer| matches!(layer.layer, Layer::Text(_)) && made_by(layer, detection::MODEL))
        .collect()
}

/// The page's balloons: detection's shapes, as paths.
fn balloons<'a>(layers: &[&'a LayerView]) -> Vec<(&'a LayerView, BezPath)> {
    layers
        .iter()
        .filter(|layer| made_by(layer, detection::MODEL))
        .filter_map(|layer| match &layer.layer {
            Layer::Shape(shape) => Some((*layer, BezPath::from_svg(shape.path.as_str()).ok()?)),
            _ => None,
        })
        .collect()
}

/// The page's translations, each with the blocks it covers.
fn translations<'a>(layers: &[&'a LayerView]) -> Vec<(&'a LayerView, Vec<LayerId>)> {
    layers
        .iter()
        .filter_map(|layer| {
            let Layer::Text(text) = &layer.layer else {
                return None;
            };
            layer
                .origin
                .iter()
                .any(|record| record.action.action == TRANSLATED)
                .then_some(())?;
            let covers = text
                .attributes
                .get("koharu")
                .and_then(|koharu| koharu.get("covers"))
                .and_then(|covers| serde_json::from_value(covers.clone()).ok())
                .unwrap_or_default();
            Some((*layer, covers))
        })
        .collect()
}

/// The balloon a text block sits in: the smallest holding its box's centre.
fn balloon_of(block: &LayerView, balloons: &[(&LayerView, BezPath)]) -> Option<usize> {
    let Layer::Text(text) = &block.layer else {
        return None;
    };
    let Frame::Box { bounds, .. } = text.frame else {
        return None;
    };
    let centre = Affine::from(text.transform)
        * Point::new(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
    balloons
        .iter()
        .enumerate()
        .filter(|(_, (_, path))| path.contains(centre))
        .min_by(|(_, (_, left)), (_, (_, right))| left.area().abs().total_cmp(&right.area().abs()))
        .map(|(index, _)| index)
}

/// A mask's outline as SVG path data in page pixels: its largest outer
/// contour, simplified. `(x, y)` is the mask's corner on the page.
fn outline(mask: &GrayImage, x: u32, y: u32) -> Option<String> {
    let mut padded = GrayImage::new(mask.width() + 2, mask.height() + 2);
    image::imageops::replace(&mut padded, mask, 1, 1);
    let contours = find_contours_with_threshold::<i32>(&padded, 0);
    let contour = contours
        .iter()
        .filter(|contour| contour.border_type == BorderType::Outer)
        .max_by(|left, right| contour_area(&left.points).total_cmp(&contour_area(&right.points)))?;
    if contour.points.len() < 3 {
        return None;
    }
    let epsilon = (arc_length(&contour.points, true) * 0.001).max(f64::EPSILON);
    let points = approximate_polygon_dp(&contour.points, epsilon, true);
    if points.len() < 3 {
        return None;
    }
    let mut path = String::new();
    for (index, point) in points.iter().enumerate() {
        let command = if index == 0 { 'M' } else { 'L' };
        let (px, py) = (
            i64::from(point.x - 1) + i64::from(x),
            i64::from(point.y - 1) + i64::from(y),
        );
        path.push_str(&format!("{command}{px} {py} "));
    }
    path.push('Z');
    Some(path)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use image::{DynamicImage, Rgb, RgbImage};
    use koharu_ml::koharu_layout_highres::{Detection, Detections, Mask};
    use koharu_scene::{Change, Hash, Ref, Size};
    use serde_json::json;

    use super::*;
    use crate::{Host, Progress};

    struct Local(Mutex<Session>);

    impl Host for Local {
        fn read<T>(&self, read: impl FnOnce(&Session) -> T) -> T {
            read(&self.0.lock().unwrap())
        }

        fn apply(&self, edit: Edit) -> koharu_scene::Result<Change> {
            self.0.lock().unwrap().apply(edit)
        }

        fn report(&self, _: Progress) {}
    }

    fn prepared<W>(prepared: Prepared<W>) -> W {
        match prepared {
            Prepared::Work(work) => work,
            Prepared::Skip(reason) => panic!("skipped: {reason}"),
        }
    }

    fn detected(
        label: &str,
        [left, top, right, bottom]: [u32; 4],
        inside: impl Fn(u32, u32) -> bool,
    ) -> Detection {
        let (width, height) = (right - left, bottom - top);
        let pixels: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| u8::from(inside(left + x, top + y)) * u8::MAX)
            .collect();
        Detection {
            label_id: 0,
            label: label.to_owned(),
            score: 0.9,
            bbox: [left, top, right, bottom].map(|value| value as f32),
            area: pixels.iter().filter(|pixel| **pixel != 0).count() as u32,
            mask: Mask {
                x: left,
                y: top,
                width,
                height,
                pixels,
            },
        }
    }

    fn translation(texts: &[(usize, usize, &str)]) -> translation::Answer {
        translation::Answer {
            texts: texts
                .iter()
                .map(|&(first, last, text)| translation::Text {
                    first,
                    last,
                    text: text.to_owned(),
                })
                .collect(),
        }
    }

    // Every step's edit, built from made-up model outputs, is one the scene
    // takes, and a rerun writes into what the step made before.
    #[test]
    fn a_page_goes_through_every_step() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let host = Local(Mutex::new(Session::create(dir.path().join("page.koharu"))?));
        // A balloon, an ellipse, with two lines of ink in it.
        let balloon = |x: u32, y: u32| {
            let (dx, dy) = (f64::from(x) - 128.0, f64::from(y) - 128.0);
            (dx / 100.0).powi(2) + (dy / 60.0).powi(2) <= 1.0
        };
        let ink = |x: u32, y: u32| {
            (100..156).contains(&x)
                && ((90..110).contains(&y) || (146..166).contains(&y))
                && !x.is_multiple_of(4)
        };
        let scan = RgbImage::from_fn(256, 256, |x, y| match (ink(x, y), balloon(x, y)) {
            (true, _) => Rgb([0; 3]),
            (false, true) => Rgb([255; 3]),
            (false, false) => Rgb([200; 3]),
        });
        let mut png = Vec::new();
        DynamicImage::ImageRgb8(scan.clone())
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        let project = host.read(|session| session.id());
        let label = |name: &str| Ref::try_from(format!("${name}")).unwrap();
        let change = host.apply(Edit {
            project,
            label: "Import".to_owned(),
            author: Author::User { via: None },
            reads: Vec::new(),
            ops: vec![
                Op::AddFile {
                    media_type: "image/png".to_owned(),
                    bytes: png.clone(),
                },
                Op::AddPage {
                    id: Some(label("page")),
                    index: 0,
                    size: Size {
                        width: 256,
                        height: 256,
                    },
                },
                Op::AddLayer {
                    id: None,
                    page: label("page"),
                    parent: None,
                    index: 0,
                    layer: serde_json::from_value(
                        json!({"kind": "image", "name": "Scan", "file": Hash::of(&png)}),
                    )?,
                    ingredients: Vec::new(),
                },
            ],
        })?;
        let page = change.pages.added[0];
        let mut context = Context {
            project,
            config: PipelineConfig::default(),
            redo: false,
            dialogue: None,
        };
        let scope = Scope::Page;
        let view = |host: &Local| host.read(|session| session.page(page)).unwrap();
        let layers = |host: &Local| -> Vec<LayerView> {
            walk(&view(host).layers).into_iter().cloned().collect()
        };

        // Detection, then a redo that writes into the same layers.
        for redo in [false, true] {
            context.redo = redo;
            let page = view(&host);
            let work =
                prepared(host.read(|session| detection::read(session, &page, &scope, &context))?);
            let output = Detections {
                image_width: 256,
                image_height: 256,
                detections: vec![
                    detected("bubble", [28, 68, 229, 189], balloon),
                    detected("text", [100, 90, 156, 110], ink),
                    detected("text", [100, 146, 156, 166], ink),
                ],
            };
            let scan = DynamicImage::ImageRgb8(scan.clone());
            host.apply(detection::edit(&context, work, &scan, output)?)?;
        }
        context.redo = false;
        let all = layers(&host);
        let all: Vec<&LayerView> = all.iter().collect();
        // The scan, the cleanup and its mask, the balloon, and two blocks.
        assert_eq!(all.len(), 6);
        assert_eq!(blocks(&all).len(), 2);
        assert_eq!(balloons(&all).len(), 1);
        let page_view = view(&host);
        let skipped =
            host.read(|session| detection::read(session, &page_view, &scope, &context))?;
        assert!(matches!(skipped, Prepared::Skip(_)));

        // The balloon's inside is one colour, so its text is filled with it
        // and the model never runs.
        let page_view = view(&host);
        let work =
            prepared(host.read(|session| inpainting::read(session, &page_view, &scope, &context))?);
        let mut calls = 0;
        let edit = inpainting::cleanup(&context, work, "stand-in", 128, false, |image, _| {
            calls += 1;
            Ok(RgbImage::new(image.width(), image.height()))
        })?;
        assert_eq!(calls, 0);
        host.apply(edit)?;
        let cleanup = all
            .iter()
            .find(|layer| matches!(layer.layer, Layer::Raster(_)))
            .unwrap()
            .id;
        let tiles = host.read(|session| session.tiles(cleanup, None))?;
        let pixel = |x: i32, y: i32| {
            tiles
                .tiles
                .iter()
                .find(|tile| (tile.x, tile.y) == (x / 64, y / 64))
                .map_or([0; 4], |tile| {
                    let at = (((y % 64) * 64 + x % 64) * 4) as usize;
                    tile.pixels[at..at + 4].try_into().unwrap()
                })
        };
        assert_eq!(pixel(101, 95), [255; 4]);
        assert_eq!(pixel(10, 10), [0; 4]);
        let page_view = view(&host);
        let skipped =
            host.read(|session| inpainting::read(session, &page_view, &scope, &context))?;
        assert!(matches!(skipped, Prepared::Skip(_)));

        let page_view = view(&host);
        let work = prepared(host.read(|session| ocr::read(session, &page_view, &scope, &context))?);
        let mut texts = ["こんにちは", "元気？"].map(String::from).into_iter();
        host.apply(ocr::edit(&context, work, &mut texts))?;

        // Each block its own thought, then, on a redo, both one.
        let page_view = view(&host);
        let work = prepared(
            host.read(|session| translation::read(session, &page_view, &scope, &context))?,
        );
        let answer = translation(&[(1, 1, "Hello."), (2, 2, "How are you?")]);
        translation::check(&work, &answer, "en")?;
        assert!(translation::check(&work, &translation(&[(1, 1, "Hello.")]), "en").is_err());
        assert!(translation::check(&work, &translation(&[(1, 2, "こんにちは")]), "en").is_err());
        host.apply(translation::edit(&context, work, answer)?)?;

        context.dialogue = Some(crate::run::dialogue(&host)?);
        let page_view = view(&host);
        let work = prepared(
            host.read(|session| typesetting::read(session, &page_view, &scope, &context))?,
        );
        host.apply(typesetting::run(project, false, work)?)?;
        let all = layers(&host);
        let all: Vec<&LayerView> = all.iter().collect();
        let typeset = translations(&all);
        assert_eq!(typeset.len(), 2);
        for (translation, covers) in &typeset {
            let Layer::Text(text) = &translation.layer else {
                unreachable!("a translation is a text");
            };
            assert!(matches!(text.frame, Frame::Path { .. }), "{:?}", text.frame);
            assert_eq!(text.text_style, context.dialogue);
            assert_eq!(covers.len(), 1);
        }
        let style = host.read(|session| session.style(context.dialogue.unwrap()))?;
        assert!(style.style.style.chars.size.is_some());

        context.redo = true;
        let page_view = view(&host);
        let work = prepared(
            host.read(|session| translation::read(session, &page_view, &scope, &context))?,
        );
        let answer = translation(&[(1, 2, "Hello, how are you?")]);
        host.apply(translation::edit(&context, work, answer)?)?;
        let all = layers(&host);
        let all: Vec<&LayerView> = all.iter().collect();
        let retranslated = translations(&all);
        assert_eq!(retranslated.len(), 1);
        assert_eq!(retranslated[0].1.len(), 2);
        Ok(())
    }
}
