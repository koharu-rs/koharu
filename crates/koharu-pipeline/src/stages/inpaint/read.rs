//! What a run inpaints: every block's mask, with the fill tiers run over the
//! scan first, or a painted area over the page as the app draws it.

use std::collections::HashMap;

use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma, RgbImage, imageops};
use koharu_renderer::Renderer;
use koharu_scene::{Content, Layer, LayerId, PageId, Project, Rect};
use kurbo::{BezPath, Shape as _};

use super::{
    Cleanup, Target, Work,
    flat::{self, Place, Verdict},
    masked,
    windows::Region,
};
use crate::{Area, stages::Member};

/// Every block's mask, the fill tiers run, and the regions they left for the
/// model.
pub(super) fn blocks(member: &Member) -> Result<Option<Work>> {
    let Some((_, scan)) = member.scan()? else {
        return Ok(None);
    };
    let mut image = scan.to_rgb8();
    let (width, height) = image.dimensions();
    let (project, mut cleanups, left) = cleanups(member, width, height)?;
    if cleanups.is_empty() {
        return Ok(None);
    }
    let places = places(&project, member.page_id(), width, height);
    let (mask, regions) = tiers(&mut image, left, &mut cleanups, &places);
    Ok(Some(Work {
        image,
        mask,
        regions,
        target: Target::Blocks(cleanups),
        windows: Vec::new(),
    }))
}

/// The painted area, all of it for the model, over its page as the app draws
/// it but for its lettering: the clean-ups, paint, and fills already there
/// show, as Photoshop's Remove tool samples all layers, so another stroke
/// builds on them. Text is left out, as Clip Studio Paint's tools that refer
/// to other layers can leave out text layers, since it changes after and
/// would stay baked into the fill. A page without a scan has nothing to fill.
pub(super) fn area(member: &Member, area: &Area) -> Result<Option<Work>> {
    let Some(image) = drawn(member)? else {
        return Ok(None);
    };
    let (width, height) = image.dimensions();
    let Some((rect, mask)) = painted(area, width, height) else {
        return Ok(None);
    };
    let limit =
        imageops::crop_imm(&mask, rect.x as u32, rect.y as u32, rect.width, rect.height).to_image();
    Ok(Some(Work {
        image,
        mask,
        regions: vec![Region { rect, panel: None }],
        target: Target::Area { rect, mask: limit },
        windows: Vec::new(),
    }))
}

/// The project as it is once the scan is decoded, and its page's blocks with
/// a mask: each the run cleans, with its mask's coverage and its clean-up's
/// tiles, and the masks of the others, whose lettering the run leaves on the
/// scan. A clean-up locked by itself is left.
fn cleanups(
    member: &Member,
    width: u32,
    height: u32,
) -> Result<(Project, Vec<Cleanup>, GrayImage)> {
    let session = member.session.lock();
    let project = session.project();
    let cleaned: Vec<LayerId> = member
        .blocks(&project)
        .into_iter()
        .filter(|(_, block)| !block.cleanup.locked)
        .map(|(id, _)| id)
        .collect();
    let mut cleanups = Vec::new();
    let mut left = GrayImage::new(width, height);
    for (id, block) in project.blocks(member.page_id()).unwrap_or_default() {
        let tiles = session.tiles(block.mask.id, None)?;
        let Some((rect, mask)) = tiles.coverage(width, height) else {
            continue;
        };
        if !cleaned.contains(&id) {
            stamp(&mut left, rect, &mask);
            continue;
        }
        cleanups.push(Cleanup {
            block: id,
            cleanup: block.cleanup.id,
            old: session.tiles(block.cleanup.id, None)?,
            rect,
            mask,
            fill: None,
            region: None,
        });
    }
    Ok((project, cleanups, left))
}

/// Runs the fill tiers over `image`, recording each tier that filled a
/// lettering; the rest, the model's, come back as its mask over the page and
/// a region each. The letterings the run leaves, `left`, are held out of what
/// the tiers read, and shown to the model as its own are.
fn tiers(
    image: &mut RgbImage,
    left: GrayImage,
    cleanups: &mut [Cleanup],
    places: &HashMap<LayerId, Place>,
) -> (GrayImage, Vec<Region>) {
    let mut covered = left.clone();
    for cleanup in cleanups.iter() {
        stamp(&mut covered, cleanup.rect, &cleanup.mask);
    }
    let mut mask = left;
    let mut regions = Vec::new();
    let _span = tracing::info_span!("fill", letterings = cleanups.len()).entered();
    let mut page = flat::Page::new(image, &covered);
    for cleanup in cleanups.iter_mut() {
        let place = places.get(&cleanup.block).cloned().unwrap_or_default();
        let redrawn = match page.fill(cleanup.rect, &cleanup.mask, &place) {
            Verdict::Filled(fill) => {
                cleanup.fill = Some(fill);
                continue;
            }
            Verdict::Model(redrawn) => redrawn,
        };
        stamp(&mut mask, cleanup.rect, &redrawn);
        cleanup.region = Some(regions.len());
        regions.push(Region {
            rect: cleanup.rect,
            panel: place.panel,
        });
    }
    (mask, regions)
}

/// The page as the app draws it without its lettering, or `None` without a
/// scan.
fn drawn(member: &Member) -> Result<Option<RgbImage>> {
    let frame = {
        let session = member.session.lock();
        let project = session.project();
        let Some(page) = project
            .page(member.page_id())
            .ok()
            .filter(|page| page.background.is_some())
        else {
            return Ok(None);
        };
        let mut lettering = Vec::new();
        letterings(&page.layers, &mut lettering);
        Renderer::get()
            .render(&session, member.page_id())?
            .without(&lettering)
    };
    Ok(Some(
        DynamicImage::ImageRgba8(frame.rasterize()?.image).to_rgb8(),
    ))
}

/// The area's pixels on a `width` by `height` page: the rectangle they cover
/// and their mask over the page, or `None` when none land on it.
fn painted(area: &Area, width: u32, height: u32) -> Option<(Rect, GrayImage)> {
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
    let rect = Rect::covering(covered?, width, height)?;
    Some((rect, mask))
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

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use image::{ImageFormat, Rgb};
    use koharu_renderer::Gpu;
    use koharu_scene::{Edit, Hash, Op, Session};
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
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())
                .expect("a project");
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
            page: Some(page),
            index: 0,
            blocks: None,
            session: Arc::new(Mutex::new(session)),
            cancel: CancellationToken::new(),
            started: Default::default(),
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
        let work = super::area(&member, &area)
            .expect("the area read")
            .expect("work on a page with a scan");
        assert_eq!(work.image.get_pixel(32, 32).0, [0; 3], "the paint shows");
        assert_eq!(dark(&work.image, 64), 0, "the translation is left out");
    }
}
