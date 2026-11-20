//! Inpainting: each block's clean-up, the scan redrawn without its
//! lettering under its mask, and a painted area's new raster layer. The fill
//! tiers rebuild what they can; the inpainting model redraws the rest in
//! windows, and only the masked pixels come back.

mod flat;
mod model;
mod telea;
mod windows;

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma, RgbImage, imageops};
use imageproc::{distance_transform::Norm, morphology::dilate};
use indoc::indoc;
use koharu_renderer::Renderer;
use koharu_scene::{Content, Layer, LayerId, Op, PageId, Project, Rect, Ref, Tile, Tiles};
use kurbo::{BezPath, Shape as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

pub use self::model::Inpainter;
use self::{
    flat::{Fill, Place, Verdict},
    windows::Region,
};
use super::{Member, Stopped, Task, action, key};
use crate::{Area, PipelineConfig, Stage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct InpainterConfig {
    /// What cleans pages and the Remove brush's areas.
    pub model: Inpainter,
    /// What Generative Fill's bar picks and keeps for the next fill, as
    /// Photoshop's Generative Fill keeps its own model; a fill brings the one
    /// its bar shows, in `model`'s place for its run.
    pub fill: Inpainter,
    /// What the model is asked when a run brings no prompt of its own, as an
    /// area's can; LaMa, which takes none, ignores it.
    pub prompt: String,
}

impl Default for InpainterConfig {
    fn default() -> Self {
        Self {
            model: Inpainter::default(),
            // A fill draws what its prompt asks, which LaMa, the clean-up's
            // default, can't.
            fill: Inpainter::Flux3Klein,
            // The general removal FLUX.3 Klein's masked inpainting was measured
            // with on six Blue Archive pages, where it scored as one naming a
            // panel's own lettering did.
            prompt: indoc! {"
                Remove all the text, including dialogue, sound effects, and handwritten notes.
                Leave speech bubbles empty and keep everything else exactly the same."}
            .to_owned(),
        }
    }
}

/// A painted area to inpaint, and its own prompt.
pub(crate) struct AreaWork {
    pub(crate) area: Area,
    pub(crate) prompt: Option<String>,
}

/// A page's scan as it's being filled, and what the model redraws in it.
struct Work {
    image: RgbImage,
    /// What the model redraws, over the page.
    mask: GrayImage,
    regions: Vec<Region>,
    target: Target,
}

enum Target {
    Blocks(Vec<Cleanup>),
    /// The area's rectangle and its mask over it.
    Area {
        rect: Rect,
        mask: GrayImage,
    },
}

/// A block's clean-up: its grown mask over `rect`, the clean-up's tiles now,
/// and whether the model redraws it.
struct Cleanup {
    block: LayerId,
    cleanup: LayerId,
    old: Tiles,
    rect: Rect,
    mask: GrayImage,
    /// The tier that filled it, or `None` when the model redraws it.
    fill: Option<Fill>,
}

pub(crate) async fn inpaint(
    task: &Task,
    config: &PipelineConfig,
    area: Option<Arc<AreaWork>>,
) -> Result<()> {
    let member = &task.members[0];
    let (read, reading) = (member.clone(), area.clone());
    let work = tokio::task::spawn_blocking(move || {
        let _span =
            tracing::info_span!("read", stage = %Stage::Inpaint, page = read.index + 1).entered();
        match &reading {
            Some(area) => read_area(&read, &area.area),
            None => read_blocks(&read),
        }
    })
    .await??;
    let Some(mut work) = work else {
        return Ok(());
    };
    let key = key(Stage::Inpaint, config).expect("an inpainter is Koharu's");
    let model = config.inpainter.model;
    // An area's own prompt names its layer, as Photoshop names a Generative
    // Fill's layer after its prompt.
    let own = area.and_then(|area| area.prompt.clone());
    let name = own.clone().unwrap_or_default();
    let prompt = own.unwrap_or_else(|| config.inpainter.prompt.clone());
    let asked = prompt.clone();
    let (cancel, runtime) = (member.cancel.clone(), Handle::current());
    let work = task
        .inference(&[member], move |models| {
            // What the fill tiers rebuilt all of needs no model.
            if work.regions.is_empty() {
                return Ok(work);
            }
            let inpainter = models.get(&key, |device| model::load(model, device))?;
            // Stop ends the redraw under way as well as the windows after it.
            let _stop = inpainter.canceller().map(|canceller| {
                let cancel = cancel.clone();
                AbortOnDropHandle::new(runtime.spawn(async move {
                    cancel.cancelled().await;
                    canceller.cancel();
                }))
            });
            let (width, height) = work.image.dimensions();
            for window in windows::plan(&work.regions, width, height, inpainter.pass()) {
                if cancel.is_cancelled() {
                    return Err(Stopped.into());
                }
                let (x, y) = (window.x as u32, window.y as u32);
                let cut =
                    imageops::crop_imm(&work.mask, x, y, window.width, window.height).to_image();
                if cut.pixels().all(|pixel| pixel.0[0] == 0) {
                    continue;
                }
                let crop =
                    imageops::crop_imm(&work.image, x, y, window.width, window.height).to_image();
                let _span =
                    tracing::info_span!("redraw", width = window.width, height = window.height)
                        .entered();
                let redrawn = inpainter.inference(&crop, &cut, &asked)?;
                for (cx, cy, pixel) in cut.enumerate_pixels() {
                    if pixel.0[0] != 0 {
                        work.image
                            .put_pixel(x + cx, y + cy, *redrawn.get_pixel(cx, cy));
                    }
                }
            }
            Ok(work)
        })
        .await?;
    let page = member.page;
    let model = model.to_string();
    member
        .apply(Stage::Inpaint, move |project| match work.target {
            Target::Blocks(cleanups) => Ok(cleanups
                .iter()
                .filter(|cleanup| {
                    project.layer(cleanup.block).is_ok_and(|layer| {
                        matches!(&layer.content, Content::Block(block) if block.cleanup.id == cleanup.cleanup)
                    })
                })
                .flat_map(|cleanup| cleaned(cleanup, &work.image, &model, &prompt))
                .collect()),
            Target::Area { rect, mask } => {
                let Ok(at) = project.page(page) else {
                    return Ok(Vec::new());
                };
                let parameters = (!prompt.trim().is_empty()).then(|| json!({"prompt": prompt}));
                let label = |label: &str| Ref::try_from(label.to_owned()).map_err(anyhow::Error::msg);
                Ok(vec![
                    serde_json::from_value(json!({
                        "op": "add_layer",
                        "id": "$inpainted",
                        "page": page,
                        "index": at.layers.len(),
                        "layer": {"kind": "raster", "name": name, "attributes": {
                            "c2pa": {"inpainting": action("c2pa.created", &model, parameters)},
                        }},
                    }))?,
                    serde_json::from_value(json!({
                        "op": "add_layer",
                        "id": "$limit",
                        "page": page,
                        "parent": "$inpainted",
                        "index": 0,
                        "layer": {"kind": "mask"},
                    }))?,
                    Op::WritePixels {
                        layer: label("$inpainted")?,
                        rect,
                        pixels: rgba(&work.image, rect, |_, _| 255),
                    },
                    Op::WritePixels {
                        layer: label("$limit")?,
                        rect,
                        pixels: mask.into_raw(),
                    },
                ])
            }
        })
        .await
}

/// The ops replacing a block's clean-up with the fill under its grown mask.
fn cleaned(cleanup: &Cleanup, image: &RgbImage, model: &str, prompt: &str) -> Vec<Op> {
    let size = Tile::SIZE;
    let mut ops: Vec<Op> = cleanup
        .old
        .tiles
        .iter()
        .map(|tile| Op::WritePixels {
            layer: cleanup.cleanup.into(),
            rect: Rect {
                x: tile.x * size as i32 + cleanup.old.offset.x,
                y: tile.y * size as i32 + cleanup.old.offset.y,
                width: size,
                height: size,
            },
            pixels: vec![0; (size * size * 4) as usize],
        })
        .collect();
    let rect = cleanup.rect;
    ops.push(Op::WritePixels {
        layer: cleanup.cleanup.into(),
        rect,
        pixels: rgba(image, rect, |x, y| {
            cleanup
                .mask
                .get_pixel(x - rect.x as u32, y - rect.y as u32)
                .0[0]
        }),
    });
    let (made_by, parameters) = match cleanup.fill {
        Some(fill) => ("Koharu", Some(json!({"fill": fill.to_string()}))),
        None => (
            model,
            (!prompt.trim().is_empty()).then(|| json!({"prompt": prompt})),
        ),
    };
    ops.push(Op::SetAttribute {
        layer: cleanup.block.into(),
        namespace: "c2pa".to_owned(),
        key: "inpainting".to_owned(),
        value: Some(action("c2pa.created", made_by, parameters)),
    });
    ops
}

/// Every block's mask, the fill tiers run, and the regions they left for the
/// model.
fn read_blocks(member: &Member) -> Result<Option<Work>> {
    let Some((project, scan)) = member.scan()? else {
        return Ok(None);
    };
    let mut image = scan.to_rgb8();
    let (width, height) = image.dimensions();
    let mut cleanups = Vec::new();
    {
        let session = member.session.lock();
        for (id, block) in member.blocks(&project) {
            let tiles = session.tiles(block.mask.id, None)?;
            if let Some((rect, mask)) = tiles.coverage(width, height) {
                cleanups.push(Cleanup {
                    block: id,
                    cleanup: block.cleanup.id,
                    old: session.tiles(block.cleanup.id, None)?,
                    rect,
                    mask,
                    fill: None,
                });
            }
        }
    }
    if cleanups.is_empty() {
        return Ok(None);
    }
    let mut covered = GrayImage::new(width, height);
    for cleanup in &cleanups {
        stamp(&mut covered, cleanup.rect, &cleanup.mask);
    }
    let places = places(&project, member.page, width, height);
    let mut mask = GrayImage::new(width, height);
    let mut regions = Vec::new();
    let _span = tracing::info_span!("fill", letterings = cleanups.len()).entered();
    let mut page = flat::Page::new(&mut image, &covered);
    for cleanup in &mut cleanups {
        let place = places.get(&cleanup.block).cloned().unwrap_or_default();
        let line = match page.fill(cleanup.rect, &cleanup.mask, &place) {
            Verdict::Filled(fill) => {
                cleanup.fill = Some(fill);
                continue;
            }
            Verdict::Model { line } => line,
        };
        // A line under the mask goes with a pixel around it.
        let redrawn = if line {
            dilate(&cleanup.mask, Norm::LInf, 1)
        } else {
            cleanup.mask.clone()
        };
        stamp(&mut mask, cleanup.rect, &redrawn);
        regions.push(Region {
            rect: cleanup.rect,
            panel: place.panel,
        });
    }
    Ok(Some(Work {
        image,
        mask,
        regions,
        target: Target::Blocks(cleanups),
    }))
}

/// The painted area, all of it for the model, over its page as the app draws
/// it but for its lettering: the clean-ups, paint, and fills already there
/// show, as Photoshop's Remove tool samples all layers, so another stroke
/// builds on them. Text is left out, as Clip Studio Paint's tools that refer
/// to other layers can leave out text layers, since it changes after and
/// would stay baked into the fill. A page without a scan has nothing to fill.
fn read_area(member: &Member, area: &Area) -> Result<Option<Work>> {
    let frame = {
        let session = member.session.lock();
        let project = session.project();
        let Some(page) = project
            .page(member.page)
            .ok()
            .filter(|page| page.background.is_some())
        else {
            return Ok(None);
        };
        let mut lettering = Vec::new();
        letterings(&page.layers, &mut lettering);
        Renderer::get()
            .render(&session, member.page)?
            .without(&lettering)
    };
    let image = DynamicImage::ImageRgba8(frame.rasterize()?.image).to_rgb8();
    let (width, height) = image.dimensions();
    let mut mask = GrayImage::new(width, height);
    let (left, top) = (i64::from(area.rect.x), i64::from(area.rect.y));
    let mut covered: Option<kurbo::Rect> = None;
    for (at, value) in area.pixels.iter().enumerate() {
        let x = left + (at % area.rect.width as usize) as i64;
        let y = top + (at / area.rect.width as usize) as i64;
        if *value != 0 && (0..i64::from(width)).contains(&x) && (0..i64::from(height)).contains(&y)
        {
            mask.put_pixel(x as u32, y as u32, Luma([255]));
            let pixel = kurbo::Rect::new(x as f64, y as f64, x as f64 + 1.0, y as f64 + 1.0);
            covered = Some(covered.map_or(pixel, |covered| covered.union(pixel)));
        }
    }
    let Some(rect) = covered.and_then(|covered| Rect::covering(covered, width, height)) else {
        return Ok(None);
    };
    let limit =
        imageops::crop_imm(&mask, rect.x as u32, rect.y as u32, rect.width, rect.height).to_image();
    Ok(Some(Work {
        image,
        mask,
        regions: vec![Region { rect, panel: None }],
        target: Target::Area { rect, mask: limit },
    }))
}

/// The layers in `layers` that letter the page: each text layer, and each
/// block with a translation, which a frame leaves out but for its clean-up.
fn letterings(layers: &[Layer], found: &mut Vec<LayerId>) {
    for layer in layers {
        match &layer.content {
            Content::Text(_) => found.push(layer.id),
            Content::Block(block) if block.translation.is_some() => found.push(layer.id),
            _ => {}
        }
        letterings(&layer.children, found);
    }
}

/// Each block of the page and where it sits.
fn places(project: &Project, page: PageId, width: u32, height: u32) -> HashMap<LayerId, Place> {
    fn walk(layers: &[Layer], place: &Place, size: (u32, u32), out: &mut HashMap<LayerId, Place>) {
        for layer in layers {
            let mut inner = place.clone();
            match &layer.content {
                Content::Panel(panel) => {
                    inner.panel = BezPath::from_svg(panel.outline.as_str())
                        .ok()
                        .and_then(|outline| Rect::covering(outline.bounding_box(), size.0, size.1));
                }
                Content::Balloon(balloon) => {
                    inner.balloon = BezPath::from_svg(balloon.outline.as_str()).ok();
                }
                Content::Block(_) => {
                    out.insert(layer.id, place.clone());
                }
                _ => {}
            }
            walk(&layer.children, &inner, size, out);
        }
    }
    let mut out = HashMap::new();
    if let Ok(page) = project.page(page) {
        walk(&page.layers, &Place::default(), (width, height), &mut out);
    }
    out
}

/// `mask`, over `rect`, set on `target`.
fn stamp(target: &mut GrayImage, rect: Rect, mask: &GrayImage) {
    for (x, y) in masked(rect, mask) {
        target.put_pixel(x, y, Luma([255]));
    }
}

/// The page pixels `mask`, over `rect`, sets.
fn masked(rect: Rect, mask: &GrayImage) -> impl Iterator<Item = (u32, u32)> + '_ {
    let (x0, y0) = (rect.x as u32, rect.y as u32);
    mask.enumerate_pixels()
        .filter(|(_, _, pixel)| pixel.0[0] != 0)
        .map(move |(x, y, _)| (x0 + x, y0 + y))
}

/// `image`'s pixels in `rect` as RGBA, each with the alpha `alpha` gives.
fn rgba(image: &RgbImage, rect: Rect, alpha: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let (x0, y0) = (rect.x as u32, rect.y as u32);
    (y0..y0 + rect.height)
        .flat_map(|y| (x0..x0 + rect.width).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = image.get_pixel(x, y).0;
            [r, g, b, alpha(x, y)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{ImageFormat, Rgb};
    use koharu_renderer::Gpu;
    use koharu_scene::{Edit, Hash, Session};
    use parking_lot::Mutex;
    use petgraph::stable_graph::NodeIndex;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::RunId;

    /// The Remove brush fills over its page as drawn: a raster painted over
    /// the scan shows to the model, as an earlier removal does, while a
    /// block's translation doesn't.
    #[test]
    fn an_area_is_filled_over_the_page_as_drawn_without_its_lettering() {
        if Gpu::get().is_err() {
            eprintln!("skipped without a GPU");
            return;
        }
        // A white scan; a black square on its left, and a translation in
        // black on its right.
        let mut scan = Vec::new();
        RgbImage::from_pixel(128, 64, Rgb([255; 3]))
            .write_to(&mut Cursor::new(&mut scan), ImageFormat::Png)
            .expect("a PNG");
        let black = [0u8, 0, 0, 255].repeat(48 * 48);
        let ops = vec![
            json!({"op": "add_page", "id": "$page", "index": 0, "background": Hash::of(&scan)}),
            json!({"op": "add_layer", "id": "$paint", "page": "$page", "index": 0, "layer": {"kind": "raster"}}),
            json!({"op": "write_pixels", "layer": "$paint", "rect": {"x": 8, "y": 8, "width": 48, "height": 48},
                "pixels": black}),
            json!({"op": "add_layer", "page": "$page", "index": 1, "layer": {
                "kind": "block", "source": {"content": "やあ"}, "translation": {
                    "content": "HH",
                    "style": {"chars": {"family": "Arial", "size": 48.0}},
                    "frame": {"box": {"bounds": {"x": 0, "y": 0, "width": 64, "height": 64}, "fit": "none"}},
                    "transform": [[1.0, 0.0, 64.0], [0.0, 1.0, 0.0]],
                },
            }}),
        ];
        let mut ops: Vec<Op> = ops
            .into_iter()
            .map(|op| serde_json::from_value(op).expect("an op"))
            .collect();
        ops.insert(0, Op::AddFile { bytes: scan });
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
        let change = session
            .apply(Edit {
                label: "test".to_owned(),
                ops,
            })
            .expect("an edit");
        let page = change.pages.added[0];
        let dark = |image: &RgbImage, left: u32| {
            (left..left + 64)
                .flat_map(|x| (0..64).map(move |y| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y).0[0] < 128)
                .count()
        };
        let drawn = DynamicImage::ImageRgba8(
            Renderer::get()
                .render(&session, page)
                .and_then(|frame| frame.rasterize())
                .expect("the page drawn")
                .image,
        )
        .to_rgb8();
        assert!(dark(&drawn, 64) > 100, "the translation draws");

        let member = Member {
            run: RunId(0),
            job: NodeIndex::new(0),
            page,
            index: 0,
            blocks: None,
            session: Arc::new(Mutex::new(session)),
            cancel: CancellationToken::new(),
        };
        let area = Area {
            rect: Rect {
                x: 0,
                y: 0,
                width: 128,
                height: 64,
            },
            pixels: vec![255; 128 * 64],
        };
        let work = read_area(&member, &area)
            .expect("the area read")
            .expect("work on a page with a scan");
        assert_eq!(work.image.get_pixel(32, 32).0, [0; 3], "the paint shows");
        assert_eq!(dark(&work.image, 64), 0, "the translation is left out");
    }
}
