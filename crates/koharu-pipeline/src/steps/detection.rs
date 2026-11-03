//! Detection: the layout model's balloons and text blocks as layers, and the
//! text mask its blocks paint.

mod measure;

use anyhow::Result;
use image::{DynamicImage, GrayImage};
use koharu_ml::koharu_layout_highres::{Detection, Detections, KoharuLayoutHighres, Options};
use koharu_scene::{
    Edit, Frame, Layer, LayerId, LayerView, MaskContent, Op, PageView, Read, Rect, Ref, Session,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use self::measure::{ImageSize, Measured};
use super::{Context, Prepared, Scope};
use crate::Step;

/// The software agent detection's layers name.
pub(super) const MODEL: &str = "koharu-layout-highres";
/// A text block at least this share inside a balloon's mask is the
/// balloon's dialogue.
const DIALOGUE: f32 = 0.9;

/// `[pipeline.processor."koharu-layout-highres"]`: confidence
/// thresholds from 0 to 1, the model's own where unset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KoharuLayoutHighresConfig {
    pub text_threshold: Option<f32>,
    pub bubble_threshold: Option<f32>,
    pub panel_threshold: Option<f32>,
}

pub(crate) struct Work {
    page: Ref,
    scan: Vec<u8>,
    from_scan: Read,
    area: Option<Rect>,
    /// Just above the scan, where a new cleanup goes.
    above_scan: usize,
    /// The page's top-level layers, after which new ones go.
    top: usize,
    /// What an earlier run made that this one replaces, in page order.
    balloons: Vec<LayerId>,
    blocks: Vec<LayerId>,
    cleanup: Option<(LayerId, Option<LayerId>)>,
    reads: Vec<Read>,
}

pub(super) fn read(
    session: &Session,
    view: &PageView,
    scope: &Scope,
    context: &Context,
) -> Result<Prepared<Work>> {
    let area = match scope {
        Scope::Page => None,
        Scope::Area(rect) => Some(*rect),
        Scope::Layers(_) => {
            return Ok(Prepared::Skip(
                "detection runs on pages and areas, not text blocks".to_owned(),
            ));
        }
    };
    let Some(scan) = super::scan(view) else {
        return Ok(Prepared::Skip("the page has no scan".to_owned()));
    };
    let made: Vec<&LayerView> = super::walk(&view.layers)
        .into_iter()
        .filter(|layer| super::made_by(layer, MODEL))
        .collect();
    if area.is_none()
        && !context.redo
        && !made.is_empty()
        && !made.iter().any(|layer| super::out_of_date(layer, MODEL))
    {
        return Ok(Prepared::Skip("the page already has detection".to_owned()));
    }
    let (scan_file, from_scan) = super::scan_file(session, view.layers[scan].id)?;
    // Inside an area, only what overlaps it is replaced.
    let replaced = |layer: &LayerView| {
        area.is_none_or(|area| bounds(&layer.layer).is_some_and(|bounds| overlaps(bounds, area)))
    };
    let of_kind = |shape: bool| -> Vec<&LayerView> {
        made.iter()
            .copied()
            .filter(|layer| match layer.layer {
                Layer::Shape(_) => shape,
                Layer::Text(_) => !shape,
                _ => false,
            })
            .filter(|layer| replaced(layer))
            .collect()
    };
    let (balloons, blocks) = (of_kind(true), of_kind(false));
    let cleanup = made
        .iter()
        .find(|layer| matches!(layer.layer, Layer::Raster(_)))
        .map(|raster| {
            let mask = raster.masks.iter().find(|mask| {
                matches!(&mask.layer, Layer::Mask(mask) if matches!(mask.content, MaskContent::Pixels { .. }))
            });
            (raster.id, mask.map(|mask| mask.id))
        });
    Ok(Prepared::Work(Work {
        page: view.id.into(),
        scan: scan_file.bytes,
        from_scan,
        area,
        above_scan: scan + 1,
        top: view.layers.len(),
        reads: balloons
            .iter()
            .chain(&blocks)
            .map(|layer| super::whole(layer))
            .collect(),
        balloons: balloons.iter().map(|layer| layer.id).collect(),
        blocks: blocks.iter().map(|layer| layer.id).collect(),
        cleanup,
    }))
}

/// Decodes every page's scan on the CPU pool, runs the model over them one at
/// a time, and builds the edits on the pool again.
pub(super) fn run(
    model: &KoharuLayoutHighres,
    works: Vec<Work>,
    context: &Context,
) -> Vec<Result<Edit>> {
    let settings = &context.config.processor.koharu_layout_highres;
    let options = Options {
        text: settings.text_threshold,
        onomatopoeia: None,
        bubble: settings.bubble_threshold,
        panel: settings.panel_threshold,
    };
    let decoded: Vec<Result<(Work, DynamicImage)>> = works
        .into_par_iter()
        .map(|work| {
            let scan = image::load_from_memory(&work.scan)?;
            Ok((work, scan))
        })
        .collect();
    let detected: Vec<Result<_>> = decoded
        .into_iter()
        .map(|decoded| {
            let (work, scan) = decoded?;
            let detections = model.inference(&scan, &options)?;
            Ok((work, scan, detections))
        })
        .collect();
    detected
        .into_par_iter()
        .map(|detected| {
            let (work, scan, detections) = detected?;
            edit(context, work, &scan, detections)
        })
        .collect()
}

pub(super) fn edit(
    context: &Context,
    work: Work,
    scan: &DynamicImage,
    output: Detections,
) -> Result<Edit> {
    let Detections {
        mut detections,
        image_width: width,
        image_height: height,
    } = output;
    if let Some(area) = work.area {
        detections.retain(|detection| overlaps(detection.bbox.map(f64::from), area));
    }
    measure::non_maximum_suppression(&mut detections, 0.5);
    let rgb = scan.to_rgb8();
    let of_label = |label: &str| -> Vec<&Detection> {
        detections
            .iter()
            .filter(|detection| detection.label == label)
            .collect()
    };
    let (balloons, blocks) = (of_label("bubble"), of_label("text"));
    let measured: Vec<Option<Measured>> = blocks
        .par_iter()
        .map(|block| measure::measure(&rgb, block))
        .collect();
    let mask = measure::text_mask(&detections, ImageSize { width, height });

    let ingredients = [work.from_scan];
    let mut ops = Vec::new();
    let mut top = work.top;
    let mask_layer = match work.cleanup {
        Some((_, Some(mask))) => Ref::from(mask),
        Some((cleanup, None)) => {
            ops.push(add(
                &work.page,
                Some(cleanup.into()),
                0,
                "mask",
                text_mask(),
                &ingredients,
            )?);
            label("mask")
        }
        None => {
            let raster = json!({"kind": "raster", "name": "Cleanup"});
            ops.push(add(
                &work.page,
                None,
                work.above_scan,
                "cleanup",
                raster,
                &ingredients,
            )?);
            ops.push(add(
                &work.page,
                Some(label("cleanup")),
                0,
                "mask",
                text_mask(),
                &ingredients,
            )?);
            top += 1;
            label("mask")
        }
    };
    let rect = work.area.map_or(
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        |area| clamp(area, width, height),
    );
    ops.push(Op::WritePixels {
        layer: mask_layer,
        rect,
        pixels: image::imageops::crop_imm(
            &mask,
            rect.x as u32,
            rect.y as u32,
            rect.width,
            rect.height,
        )
        .to_image()
        .into_raw(),
    });

    let balloons_out = balloons
        .iter()
        .map(|balloon| {
            let mask = &balloon.mask;
            let path = GrayImage::from_raw(mask.width, mask.height, mask.pixels.clone())
                .and_then(|pixels| super::outline(&pixels, mask.x, mask.y))
                .unwrap_or_else(|| box_path(balloon.bbox));
            json!({
                "kind": "shape",
                "name": "Balloon",
                "path": path,
                "fill": null,
                "stroke": null,
                "attributes": {"koharu": {"region": "bubble", "confidence": balloon.score}},
            })
        })
        .collect();
    replace(
        &mut ops,
        &work.page,
        &mut top,
        &work.balloons,
        balloons_out,
        &ingredients,
    )?;
    let blocks_out = blocks
        .iter()
        .zip(&measured)
        .map(|(block, measured)| {
            let dialogue = balloons.iter().any(|balloon| {
                measure::mask_containment(&balloon.mask, &block.mask, block.bbox) >= DIALOGUE
            });
            text_block(block, *measured, dialogue)
        })
        .collect();
    replace(
        &mut ops,
        &work.page,
        &mut top,
        &work.blocks,
        blocks_out,
        &ingredients,
    )?;

    let action = super::action("c2pa.created", MODEL, Map::new());
    Ok(super::edit(
        context.project,
        Step::Detection,
        action,
        work.reads,
        ops,
    ))
}

/// Writes `layers` into the layers an earlier run made, adds the rest, and
/// removes what this run no longer produces.
fn replace(
    ops: &mut Vec<Op>,
    page: &Ref,
    top: &mut usize,
    previous: &[LayerId],
    layers: Vec<Value>,
    ingredients: &[Read],
) -> Result<()> {
    let count = layers.len();
    for (index, mut layer) in layers.into_iter().enumerate() {
        match previous.get(index) {
            Some(&id) => {
                layer
                    .as_object_mut()
                    .expect("a layer is an object")
                    .remove("kind");
                ops.push(Op::UpdateLayer {
                    layer: id.into(),
                    patch: layer,
                    ingredients: ingredients.to_vec(),
                });
            }
            None => {
                ops.push(Op::AddLayer {
                    id: None,
                    page: page.clone(),
                    parent: None,
                    index: *top,
                    layer: serde_json::from_value(layer)?,
                    ingredients: ingredients.to_vec(),
                });
                *top += 1;
            }
        }
    }
    for &id in previous.iter().skip(count) {
        ops.push(Op::RemoveLayer { layer: id.into() });
    }
    Ok(())
}

/// A hidden text with no content yet: the block's box, its rotation about
/// the box's centre, and the direction and ink measured from the scan.
fn text_block(block: &Detection, measured: Option<Measured>, dialogue: bool) -> Value {
    let [left, top, right, bottom] = block.bbox.map(f64::from);
    let (width, height) = ((right - left).max(1.0), (bottom - top).max(1.0));
    let angle = measured.map_or(0.0, |measured| f64::from(measured.angle_degrees));
    let (sin, cos) = angle.to_radians().sin_cos();
    let (cx, cy) = (left + width / 2.0, top + height / 2.0);
    let transform = [
        [cos, -sin, cx - cos * cx + sin * cy],
        [sin, cos, cy - sin * cx - cos * cy],
    ];
    let color = |[r, g, b]: [u8; 3]| [r, g, b].map(|c| f32::from(c) / 255.0).to_vec();
    let chars = measured.map_or_else(
        || json!({"fill": null, "stroke": null, "stroke_width": null}),
        |measured| {
            json!({
                "fill": color(measured.color).into_iter().chain([1.0]).collect::<Vec<_>>(),
                "stroke": measured.stroke_color.map(|stroke| color(stroke).into_iter().chain([1.0]).collect::<Vec<_>>()),
                "stroke_width": measured.stroke_width,
            })
        },
    );
    json!({
        "kind": "text",
        "visible": false,
        "frame": {"box": {"bounds": {"x": left, "y": top, "width": width, "height": height}}},
        "transform": transform,
        "direction": measured.map(|measured| measured.direction).unwrap_or_default(),
        "style": {"chars": chars},
        "attributes": {"koharu": {
            "region": "text",
            "confidence": block.score,
            "role": if dialogue { "dialogue" } else { "free" },
        }},
    })
}

fn text_mask() -> Value {
    json!({"kind": "mask", "name": "Text mask", "attributes": {"koharu": {"mask": "text"}}})
}

/// Adds a layer named `$id`.
fn add(
    page: &Ref,
    parent: Option<Ref>,
    index: usize,
    id: &str,
    layer: Value,
    ingredients: &[Read],
) -> Result<Op> {
    Ok(Op::AddLayer {
        id: Some(label(id)),
        page: page.clone(),
        parent,
        index,
        layer: serde_json::from_value(layer)?,
        ingredients: ingredients.to_vec(),
    })
}

fn label(name: &str) -> Ref {
    Ref::try_from(format!("${name}")).expect("a valid label")
}

fn box_path([left, top, right, bottom]: [f32; 4]) -> String {
    format!("M{left} {top} L{right} {top} L{right} {bottom} L{left} {bottom} Z")
}

/// A layer's box on the page: a text's frame, a shape's path.
fn bounds(layer: &Layer) -> Option<[f64; 4]> {
    match layer {
        Layer::Text(text) => match text.frame {
            Frame::Box { bounds, .. } => Some([
                bounds.x,
                bounds.y,
                bounds.x + bounds.width,
                bounds.y + bounds.height,
            ]),
            _ => None,
        },
        Layer::Shape(shape) => {
            let rect =
                kurbo::Shape::bounding_box(&kurbo::BezPath::from_svg(shape.path.as_str()).ok()?);
            Some([rect.x0, rect.y0, rect.x1, rect.y1])
        }
        _ => None,
    }
}

fn overlaps([left, top, right, bottom]: [f64; 4], area: Rect) -> bool {
    left < f64::from(area.x) + f64::from(area.width)
        && right > f64::from(area.x)
        && top < f64::from(area.y) + f64::from(area.height)
        && bottom > f64::from(area.y)
}

/// `area` inside a page of `width` × `height`.
fn clamp(area: Rect, width: u32, height: u32) -> Rect {
    let x = area.x.clamp(0, width as i32);
    let y = area.y.clamp(0, height as i32);
    let right = (i64::from(area.x) + i64::from(area.width)).clamp(i64::from(x), i64::from(width));
    let bottom =
        (i64::from(area.y) + i64::from(area.height)).clamp(i64::from(y), i64::from(height));
    Rect {
        x,
        y,
        width: (right - i64::from(x)) as u32,
        height: (bottom - i64::from(y)) as u32,
    }
}
