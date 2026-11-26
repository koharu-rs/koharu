//! The ops a run writes: each block's clean-up, or the area's raster and its
//! mask, or the area of the layer it goes onto, with the C2PA action recording
//! what made them.

use anyhow::Result;
use image::{DynamicImage, GrayImage, RgbImage, imageops};
use koharu_scene::{LayerId, Op, PageId, Project, Rect, Ref, Session, Tile};
use serde_json::{Value, json};

use super::{
    Cleanup, Target, Work,
    windows::{self, Window},
};
use crate::stages::{action, writable};

/// The ops for `work`; an area's goes onto layer `to`, or a new one named
/// `name`, and `made` is the model's action.
pub(super) fn ops(
    session: &Session,
    page: PageId,
    work: Work,
    to: Option<LayerId>,
    made: &Value,
    name: &str,
) -> Result<Vec<Op>> {
    let Work {
        image,
        target,
        windows,
        ..
    } = work;
    let project = &session.project();
    match target {
        Target::Blocks(cleanups) => Ok(cleanups
            .iter()
            .filter(|cleanup| current(project, cleanup))
            .flat_map(|cleanup| cleaned(cleanup, &image, &windows, made))
            .collect()),
        Target::Area { rect, mask } => {
            if let Some(layer) = to {
                return onto(session, page, layer, (rect, &mask), &image, made);
            }
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
/// detected again since holds another, and neither is locked now.
fn current(project: &Project, cleanup: &Cleanup) -> bool {
    writable(project, cleanup.block)
        .is_some_and(|block| block.cleanup.id == cleanup.cleanup && !block.cleanup.locked)
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

/// The ops laying `image` on `layer` in an area, and opening a clean-up's mask
/// there; none when it was removed meanwhile.
fn onto(
    session: &Session,
    page: PageId,
    layer: LayerId,
    (rect, area): (Rect, &GrayImage),
    image: &RgbImage,
    made: &Value,
) -> Result<Vec<Op>> {
    if session.tiles(layer, Some(rect)).is_err() {
        return Ok(Vec::new());
    }
    let project = session.project();
    let blocks = project.blocks(page)?;
    let block = blocks.iter().find(|(_, block)| block.cleanup.id == layer);
    let drawn = rgba(image, rect, |_, _| 255);
    let mut ops = vec![laid(session, layer, rect, area, &drawn)?];
    if let Some((_, block)) = block {
        ops.push(laid(session, block.mask.id, rect, area, area)?);
    }
    ops.push(Op::SetAttribute {
        layer: block.map_or(layer, |(id, _)| *id).into(),
        namespace: "c2pa".to_owned(),
        key: "inpainting".to_owned(),
        value: Some(made.clone()),
    });
    Ok(ops)
}

/// The op giving `layer` the pixels `new` over `rect` of the page, in its own
/// format, where `mask` is set.
fn laid(session: &Session, layer: LayerId, rect: Rect, mask: &GrayImage, new: &[u8]) -> Result<Op> {
    let bytes = new.len() / mask.len();
    let offset = session.tiles(layer, Some(rect))?.offset;
    let own = Rect {
        x: rect.x - offset.x,
        y: rect.y - offset.y,
        ..rect
    };
    let mut pixels = session.pixels(layer, own)?;
    let new = new.chunks_exact(bytes).zip(mask.pixels());
    for (pixel, (new, set)) in pixels.chunks_exact_mut(bytes).zip(new) {
        if set.0[0] != 0 {
            pixel.copy_from_slice(new);
        }
    }
    Ok(Op::WritePixels {
        layer: layer.into(),
        rect: own,
        pixels,
    })
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

#[cfg(test)]
mod tests {
    use image::{GrayImage, Luma, Rgb};
    use koharu_scene::{Edit, Session};

    use super::*;

    /// A raster and a clean-up take an area's result in the area only, each
    /// in its own coordinates, and the clean-up's mask opens there.
    #[test]
    fn an_area_goes_onto_the_layer_it_names() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let change = session.apply(Edit {
            label: "test".to_owned(),
            ops: serde_json::from_value(json!([
                {"op": "add_page", "id": "$p", "index": 0, "size": {"width": 64, "height": 64}},
                {"op": "add_layer", "page": "$p", "index": 0,
                    "layer": {"kind": "raster", "offset": {"x": 10, "y": 10}}},
                {"op": "add_layer", "page": "$p", "index": 1,
                    "layer": {"kind": "block", "source": {}}},
            ]))?,
        })?;
        let page = change.pages.added[0];
        let project = session.project();
        let raster = project.layers_of(page)?[0].id;
        let block = project.blocks(page)?[0].1;
        let rect = Rect {
            x: 20,
            y: 20,
            width: 4,
            height: 2,
        };
        // All of it but its first pixel.
        let mut area = GrayImage::from_pixel(4, 2, Luma([255]));
        area.put_pixel(0, 0, Luma([0]));
        for layer in [raster, block.cleanup.id] {
            let work = Work {
                image: RgbImage::from_pixel(64, 64, Rgb([0, 0, 255])),
                mask: GrayImage::new(64, 64),
                regions: Vec::new(),
                target: Target::Area {
                    rect,
                    mask: area.clone(),
                },
                windows: Vec::new(),
            };
            let ops = ops(&session, page, work, Some(layer), &json!({}), "")?;
            session.apply(Edit {
                label: "inpaint".to_owned(),
                ops,
            })?;
        }
        let first = |layer, x| {
            let two = Rect {
                x,
                y: x,
                width: 2,
                height: 1,
            };
            session.pixels(layer, two)
        };
        assert_eq!(first(raster, 10)?, [0, 0, 0, 0, 0, 0, 255, 255]);
        assert_eq!(first(block.cleanup.id, 20)?, [0, 0, 0, 0, 0, 0, 255, 255]);
        assert_eq!(first(block.mask.id, 20)?, [0, 255]);
        assert_eq!(session.project().layers_of(page)?.len(), 2);
        Ok(())
    }
}
