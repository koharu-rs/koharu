//! OCR: each text block's text, read from the scan at the block's box, and
//! written into the block.

use anyhow::{Result, anyhow, bail};
use image::{DynamicImage, imageops};
use imageproc::geometric_transformations::{Border, Interpolation, rotate_about_center};
use koharu_ml::paddle_ocr_vl_quantized::Task;
use koharu_scene::{
    Bounds, Edit, Frame, Layer, LayerId, LayerView, Op, PageView, Pointer, Read, Session, Transform,
};
use rayon::prelude::*;
use serde_json::{Map, json};
use tokio::task::block_in_place;

use super::{Context, Prepared, Scope, detection};
use crate::{OcrModel, Step, residency::Loaded};

pub(crate) struct Work {
    scan: Vec<u8>,
    from_scan: Read,
    blocks: Vec<Block>,
}

struct Block {
    id: LayerId,
    bounds: Bounds,
    transform: Transform,
    /// The block's `/frame`, `/transform`, and `/direction`.
    reads: Vec<Read>,
}

pub(super) fn read(
    session: &Session,
    view: &PageView,
    scope: &Scope,
    context: &Context,
) -> Result<Prepared<Work>> {
    let Some(scan) = super::scan(view) else {
        return Ok(Prepared::Skip("the page has no scan".to_owned()));
    };
    let candidates: Vec<&LayerView> = super::walk(&view.layers)
        .into_iter()
        .filter(|layer| {
            matches!(layer.layer, Layer::Text(_)) && super::made_by(layer, detection::MODEL)
        })
        .filter(|layer| match scope {
            Scope::Page => true,
            Scope::Layers(layers) => layers.contains(&layer.id),
            Scope::Area(area) => match &layer.layer {
                Layer::Text(text) => match text.frame {
                    Frame::Box { bounds, .. } => {
                        bounds.x < f64::from(area.x) + f64::from(area.width)
                            && bounds.x + bounds.width > f64::from(area.x)
                            && bounds.y < f64::from(area.y) + f64::from(area.height)
                            && bounds.y + bounds.height > f64::from(area.y)
                    }
                    _ => false,
                },
                _ => false,
            },
        })
        .collect();
    if candidates.is_empty() {
        return Ok(Prepared::Skip("the page has no text blocks".to_owned()));
    }
    let agent = agent(context.config.ocr);
    let mut blocks = Vec::new();
    for layer in candidates {
        let Layer::Text(text) = &layer.layer else {
            continue;
        };
        let Frame::Box { bounds, .. } = text.frame else {
            continue;
        };
        // Reruns cover what's missing or out of date, unless the run asked for
        // these blocks or for everything.
        let pending = context.redo
            || matches!(scope, Scope::Layers(_))
            || text.content.text().trim().is_empty()
            || super::out_of_date(layer, agent);
        if !pending {
            continue;
        }
        let mut reads = Vec::new();
        for path in ["/frame", "/transform", "/direction"] {
            reads.push(session.read(layer.id, Pointer::parse(path)?)?.1);
        }
        blocks.push(Block {
            id: layer.id,
            bounds,
            transform: text.transform,
            reads,
        });
    }
    if blocks.is_empty() {
        return Ok(Prepared::Skip(
            "every text block already has its text".to_owned(),
        ));
    }
    let (scan_file, from_scan) = super::scan_file(session, view.layers[scan].id)?;
    Ok(Prepared::Work(Work {
        scan: scan_file.bytes,
        from_scan,
        blocks,
    }))
}

/// The software agent an OCR model's text names.
fn agent(model: OcrModel) -> &'static str {
    match model {
        OcrModel::PaddleOcrVl1_6 => "paddleocr-vl-1.6",
        OcrModel::MangaOcr => "manga-ocr",
        OcrModel::BaberuOcr => "baberu-ocr",
        OcrModel::HayaiOcr => "hayai-ocr",
    }
}

/// Reads every page's blocks in one call to the model, and gives the text
/// back by page and block.
pub(super) async fn run(model: &Loaded, works: Vec<Work>, context: &Context) -> Vec<Result<Edit>> {
    let crops: Vec<Result<Vec<DynamicImage>>> = works
        .par_iter()
        .map(|work| {
            let scan = image::load_from_memory(&work.scan)?;
            work.blocks.iter().map(|block| crop(&scan, block)).collect()
        })
        .collect();
    let images: Vec<DynamicImage> = crops.iter().flatten().flatten().cloned().collect();
    let read = match model {
        Loaded::MangaOcr(model) => block_in_place(|| model.inference(&images)),
        Loaded::BaberuOcr(model) => block_in_place(|| model.inference(&images)),
        Loaded::HayaiOcr(model) => block_in_place(|| model.inference(&images)),
        Loaded::PaddleOcrVl(model) => model.inference(&images, Task::Ocr).await,
        _ => unreachable!("only OCR models read text"),
    };
    let mut texts = match read {
        Ok(texts) if texts.len() == images.len() => texts.into_iter(),
        Ok(texts) => {
            let reason = format!(
                "the model read {} texts for {} crops",
                texts.len(),
                images.len()
            );
            return works.iter().map(|_| Err(anyhow!(reason.clone()))).collect();
        }
        Err(error) => {
            let reason = format!("{error:#}");
            return works.iter().map(|_| Err(anyhow!(reason.clone()))).collect();
        }
    };
    works
        .into_iter()
        .zip(crops)
        .map(|(work, crops)| {
            crops?;
            Ok(edit(context, work, &mut texts))
        })
        .collect()
}

/// Writes each block's text, taken from `texts` in order.
pub(super) fn edit(
    context: &Context,
    work: Work,
    texts: &mut impl Iterator<Item = String>,
) -> Edit {
    // Only the Japanese models know their text's language.
    let language = match context.config.ocr {
        OcrModel::MangaOcr | OcrModel::BaberuOcr => "ja",
        OcrModel::HayaiOcr | OcrModel::PaddleOcrVl1_6 => "und",
    };
    let mut ops = Vec::new();
    for block in work.blocks {
        let text = normalize(texts.next().expect("a text for every crop"));
        let ingredients: Vec<Read> = std::iter::once(work.from_scan.clone())
            .chain(block.reads)
            .collect();
        ops.push(Op::SetText {
            layer: block.id.into(),
            text,
            ingredients: ingredients.clone(),
        });
        ops.push(Op::UpdateLayer {
            layer: block.id.into(),
            patch: json!({"language": language}),
            ingredients,
        });
    }
    let action = super::action("c2pa.created", agent(context.config.ocr), Map::new());
    super::edit(context.project, Step::Ocr, action, Vec::new(), ops)
}

/// The block's box from the scan, turned upright by its transform's
/// rotation. The block's direction, not the crop's shape, says how it reads,
/// and the models read either.
fn crop(scan: &DynamicImage, block: &Block) -> Result<DynamicImage> {
    let Transform([[a, _, _], [d, _, _]]) = block.transform;
    let angle = d.atan2(a);
    let affine = kurbo::Affine::from(block.transform);
    let Bounds {
        x,
        y,
        width,
        height,
    } = block.bounds;
    // The box's corners on the page, and the page area around them.
    let corners = [
        (x, y),
        (x + width, y),
        (x + width, y + height),
        (x, y + height),
    ]
    .map(|(x, y)| affine * kurbo::Point::new(x, y));
    let left = corners
        .iter()
        .map(|point| point.x)
        .fold(f64::MAX, f64::min)
        .floor()
        .max(0.0);
    let top = corners
        .iter()
        .map(|point| point.y)
        .fold(f64::MAX, f64::min)
        .floor()
        .max(0.0);
    let right = corners
        .iter()
        .map(|point| point.x)
        .fold(f64::MIN, f64::max)
        .ceil()
        .min(f64::from(scan.width()));
    let bottom = corners
        .iter()
        .map(|point| point.y)
        .fold(f64::MIN, f64::max)
        .ceil()
        .min(f64::from(scan.height()));
    if right <= left || bottom <= top {
        bail!("text block {} is outside the scan", block.id);
    }
    let around = scan.crop_imm(
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
    );
    if angle.abs() < f64::EPSILON {
        return Ok(around);
    }
    let upright = rotate_about_center(
        &around.to_rgba8(),
        -angle as f32,
        Interpolation::Bilinear,
        Border::Replicate,
    );
    let (w, h) = (width.round() as u32, height.round() as u32);
    let (cx, cy) = (upright.width() / 2, upright.height() / 2);
    let crop = imageops::crop_imm(
        &upright,
        cx.saturating_sub(w / 2),
        cy.saturating_sub(h / 2),
        w.min(upright.width()),
        h.min(upright.height()),
    )
    .to_image();
    Ok(DynamicImage::ImageRgba8(crop))
}

/// Manga OCR can read an isolated Japanese ellipsis as replacement boxes;
/// only a run of nothing but them becomes the ellipsis.
fn normalize(text: String) -> String {
    let visible: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if visible.len() >= 2
        && visible
            .iter()
            .all(|c| matches!(c, '☐' | '□' | '▢' | '▣' | '�'))
    {
        "…".to_owned()
    } else {
        text
    }
}
