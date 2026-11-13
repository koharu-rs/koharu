//! Inpainting: each block's clean-up, the scan redrawn without its
//! lettering under its mask, and a painted area's new raster layer. The fill
//! tiers rebuild what they can; the inpainting model redraws the rest in
//! windows, and only the masked pixels come back.

mod flat;
mod telea;
mod windows;

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use image::{GrayImage, Luma, RgbImage, imageops};
use koharu_scene::{Content, Layer, LayerId, Op, PageId, Project, Rect, Ref, Tile, Tiles};
use kurbo::{BezPath, Shape as _};
use serde_json::json;

use self::{
    flat::{Fill, Place, Verdict},
    windows::Region,
};
use super::{Member, Stages, action, blocks, clamp, dilate, on_page, op, scan};
use crate::{Area, Inpainter, Step, models::Models};

/// A painted area to inpaint, and the prompt for an inpainter that takes one.
pub(crate) struct AreaWork {
    pub(crate) area: Area,
    pub(crate) prompt: Option<String>,
}

pub(crate) struct Inpaint {
    model: Inpainter,
    area: Option<Arc<AreaWork>>,
}

impl Inpaint {
    pub(crate) fn new(model: Inpainter, area: Option<Arc<AreaWork>>) -> Self {
        Self { model, area }
    }
}

/// A page's scan as it's being filled, and what the model redraws in it.
pub(crate) struct Work {
    page: PageId,
    image: RgbImage,
    /// What the model redraws, over the page.
    mask: GrayImage,
    windows: Vec<Rect>,
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

impl Stages for Inpaint {
    type Input = Work;
    type Output = Work;
    type Ready = Work;

    const STEP: Step = Step::Inpaint;

    fn read(&self, member: &Member) -> Result<Option<Work>> {
        match &self.area {
            Some(area) => read_area(member, &area.area, self.model.side()),
            None => read_blocks(member, self.model.side()),
        }
    }

    fn infer(&mut self, models: &mut Models, mut inputs: Vec<Work>) -> Result<Vec<Work>> {
        let prompt = self
            .area
            .as_ref()
            .and_then(|area| area.prompt.clone())
            .unwrap_or_default();
        for work in &mut inputs {
            if work.windows.is_empty() {
                continue;
            }
            let inpainter = models.inpainter(self.model)?;
            for window in &work.windows {
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
                let redrawn = inpainter.inpaint(&crop, &cut, &prompt)?;
                for (cx, cy, pixel) in cut.enumerate_pixels() {
                    if pixel.0[0] != 0 {
                        work.image
                            .put_pixel(x + cx, y + cy, *redrawn.get_pixel(cx, cy));
                    }
                }
            }
        }
        Ok(inputs)
    }

    fn finish(&mut self, work: Work) -> Result<Work> {
        Ok(work)
    }

    fn write(&self, project: &Project, work: Work) -> Result<Vec<Op>> {
        let model = self.model.to_string();
        match work.target {
            Target::Blocks(cleanups) => Ok(cleanups
                .iter()
                .filter(|cleanup| {
                    project.layer(cleanup.block).is_ok_and(|layer| {
                        matches!(&layer.content, Content::Block(block) if block.cleanup.id == cleanup.cleanup)
                    })
                })
                .flat_map(|cleanup| cleaned(cleanup, &work.image, &model))
                .collect()),
            Target::Area { rect, mask } => {
                let Ok(page) = project.page(work.page) else {
                    return Ok(Vec::new());
                };
                let prompt = self.area.as_ref().and_then(|area| area.prompt.as_ref());
                let parameters = prompt.map(|prompt| json!({"prompt": prompt}));
                Ok(vec![
                    op(json!({
                        "op": "add_layer",
                        "id": "$inpainted",
                        "page": work.page,
                        "index": page.layers.len(),
                        "layer": {"kind": "raster", "attributes": {
                            "c2pa": {"inpainting": action("c2pa.created", &model, parameters)},
                        }},
                    }))?,
                    op(json!({
                        "op": "add_layer",
                        "id": "$limit",
                        "page": work.page,
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
        }
    }
}

/// A layer an earlier op of the same edit labels.
fn label(label: &str) -> Result<Ref> {
    Ref::try_from(label.to_owned()).map_err(anyhow::Error::msg)
}

/// The ops replacing a block's clean-up with the fill under its grown mask.
fn cleaned(cleanup: &Cleanup, image: &RgbImage, model: &str) -> Vec<Op> {
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
        None => (model, None),
    };
    ops.push(Op::SetAttribute {
        layer: cleanup.block.into(),
        namespace: "c2pa".to_owned(),
        key: "inpainting".to_owned(),
        value: Some(action("c2pa.created", made_by, parameters)),
    });
    ops
}

/// Every block's mask, the fill tiers run, and the model's windows for what
/// they left.
fn read_blocks(member: &Member, side: u32) -> Result<Option<Work>> {
    let Some((project, scan)) = scan(member)? else {
        return Ok(None);
    };
    let mut image = scan.to_rgb8();
    let (width, height) = image.dimensions();
    let mut cleanups = Vec::new();
    {
        let session = member.session.lock();
        for (id, block) in blocks(&project, member.page, member.blocks.as_deref()) {
            let tiles = session.tiles(block.mask.id, None)?;
            if let Some((rect, mask)) = on_page(&tiles, width, height) {
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
            dilate(&cleanup.mask, 1)
        } else {
            cleanup.mask.clone()
        };
        stamp(&mut mask, cleanup.rect, &redrawn);
        regions.push(Region {
            rect: cleanup.rect,
            panel: place.panel,
        });
    }
    let windows = windows::plan(&regions, width, height, side);
    Ok(Some(Work {
        page: member.page,
        image,
        mask,
        windows,
        target: Target::Blocks(cleanups),
    }))
}

/// The painted area, all of it for the model.
fn read_area(member: &Member, area: &Area, side: u32) -> Result<Option<Work>> {
    let Some((_, scan)) = scan(member)? else {
        return Ok(None);
    };
    let image = scan.to_rgb8();
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
    let Some(rect) = covered.and_then(|covered| clamp(covered, width, height)) else {
        return Ok(None);
    };
    let windows = windows::plan(&[Region { rect, panel: None }], width, height, side);
    let limit =
        imageops::crop_imm(&mask, rect.x as u32, rect.y as u32, rect.width, rect.height).to_image();
    Ok(Some(Work {
        page: member.page,
        image,
        mask,
        windows,
        target: Target::Area { rect, mask: limit },
    }))
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
                        .and_then(|outline| clamp(outline.bounding_box(), size.0, size.1));
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
