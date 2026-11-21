//! The ops a run writes: each block's clean-up, or the area's raster and its
//! mask, with the C2PA action recording what made them.

use anyhow::Result;
use image::{DynamicImage, RgbImage, imageops};
use koharu_scene::{Content, Op, PageId, Project, Rect, Ref, Tile};
use serde_json::{Value, json};

use super::{
    Cleanup, Target, Work,
    windows::{self, Window},
};
use crate::stages::action;

/// The ops for `work`; `made` is the model's action, `name` the area's layer
/// name.
pub(super) fn ops(
    project: &Project,
    page: PageId,
    work: Work,
    made: &Value,
    name: &str,
) -> Result<Vec<Op>> {
    let Work {
        image,
        target,
        windows,
        ..
    } = work;
    match target {
        Target::Blocks(cleanups) => Ok(cleanups
            .iter()
            .filter(|cleanup| current(project, cleanup))
            .flat_map(|cleanup| cleaned(cleanup, &image, &windows, made))
            .collect()),
        Target::Area { rect, mask } => {
            // An area is one region, so an edit model drew it in one window.
            let drawn = match windows.as_slice() {
                [(window, drawn)] => (window.rect, opaque(drawn)),
                _ => (rect, rgba(&image, rect, |_, _| 255)),
            };
            layer(project, page, name, made, drawn, (rect, mask.into_raw()))
        }
    }
}

/// Whether the block still holds the clean-up the run read, as a block
/// detected again since holds another.
fn current(project: &Project, cleanup: &Cleanup) -> bool {
    project.layer(cleanup.block).is_ok_and(|layer| {
        matches!(&layer.content, Content::Block(block) if block.cleanup.id == cleanup.cleanup)
    })
}

/// The ops replacing a block's clean-up and recording what made it.
fn cleaned(
    cleanup: &Cleanup,
    image: &RgbImage,
    windows: &[(Window, RgbImage)],
    made: &Value,
) -> Vec<Op> {
    let mut ops = cleared(cleanup);
    ops.push(pixels(cleanup, image, windows));
    let made = match cleanup.fill {
        Some(fill) => action(
            "c2pa.created",
            "Koharu",
            Some(json!({"fill": fill.to_string()})),
        ),
        None => made.clone(),
    };
    ops.push(Op::SetAttribute {
        layer: cleanup.block.into(),
        namespace: "c2pa".to_owned(),
        key: "inpainting".to_owned(),
        value: Some(made),
    });
    ops
}

/// The ops clearing every tile the clean-up holds now.
fn cleared(cleanup: &Cleanup) -> Vec<Op> {
    let size = Tile::SIZE;
    cleanup
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
        .collect()
}

/// The clean-up's new pixels: an edit model's window around the lettering,
/// or the fill under its grown mask.
fn pixels(cleanup: &Cleanup, image: &RgbImage, windows: &[(Window, RgbImage)]) -> Op {
    let rect = cleanup.rect;
    let window = cleanup.region.and_then(|region| {
        windows
            .iter()
            .find(|(window, _)| window.regions.contains(&region))
    });
    let (rect, pixels) = match window {
        Some((Window { rect: window, .. }, drawn)) => {
            let kept = windows::around(rect, *window);
            let cut = imageops::crop_imm(
                drawn,
                (kept.x - window.x) as u32,
                (kept.y - window.y) as u32,
                kept.width,
                kept.height,
            )
            .to_image();
            (kept, opaque(&cut))
        }
        None => (
            rect,
            rgba(image, rect, |x, y| {
                cleanup
                    .mask
                    .get_pixel(x - rect.x as u32, y - rect.y as u32)
                    .0[0]
            }),
        ),
    };
    Op::WritePixels {
        layer: cleanup.cleanup.into(),
        rect,
        pixels,
    }
}

/// The area's new raster, named after its prompt, on top of the page, with
/// `drawn` as its pixels and `limit`, the area, as its mask.
fn layer(
    project: &Project,
    page: PageId,
    name: &str,
    made: &Value,
    drawn: (Rect, Vec<u8>),
    limit: (Rect, Vec<u8>),
) -> Result<Vec<Op>> {
    let Ok(at) = project.page(page) else {
        return Ok(Vec::new());
    };
    let label = |label: &str| Ref::try_from(label.to_owned()).map_err(anyhow::Error::msg);
    Ok(vec![
        serde_json::from_value(json!({
            "op": "add_layer",
            "id": "$inpainted",
            "page": page,
            "index": at.layers.len(),
            "layer": {"kind": "raster", "name": name, "attributes": {
                "c2pa": {"inpainting": made},
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
            rect: drawn.0,
            pixels: drawn.1,
        },
        Op::WritePixels {
            layer: label("$limit")?,
            rect: limit.0,
            pixels: limit.1,
        },
    ])
}

/// All of `image` as opaque RGBA.
fn opaque(image: &RgbImage) -> Vec<u8> {
    DynamicImage::ImageRgb8(image.clone())
        .into_rgba8()
        .into_raw()
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
