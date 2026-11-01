//! Typesetting: the project's "Dialogue" style, and each translation's
//! frame, style, and direction, without a model.

use std::{cmp::Reverse, collections::BinaryHeap};

use anyhow::Result;
use image::{GrayImage, ImageBuffer, Luma};
use imageproc::distance_transform::euclidean_squared_distance_transform;
use koharu_scene::{
    Bounds, CharStyle, Direction, Edit, Frame, Layer, LayerId, LayerView, Op, PageView, Pointer,
    ProjectId, Read, Session, StyleId, Transform,
};
use kurbo::{BezPath, Point, Shape};
use serde_json::{Map, json};

use super::{Context, Prepared, Scope};
use crate::Step;

/// The software agent typesetting's values name.
pub(crate) const TYPESETTER: &str = "koharu-typesetting";
/// Dialogue's line height, a multiple of its size.
pub(crate) const LINE_HEIGHT: f64 = 1.05;

pub(crate) struct Work {
    dialogue: StyleId,
    /// The style's size, if something has set one.
    sized: bool,
    balloons: Vec<Balloon>,
    blocks: Vec<Block>,
    translations: Vec<Translation>,
}

struct Balloon {
    path: String,
    read: Read,
}

struct Block {
    bounds: Bounds,
    transform: Transform,
    direction: Direction,
    chars: CharStyle,
    /// Its text's length, for the size it's set at.
    length: usize,
    dialogue: bool,
    balloon: Option<usize>,
    /// Its `/frame`, `/style`, and `/content`.
    reads: [Read; 3],
}

struct Translation {
    id: LayerId,
    covers: Vec<usize>,
    vertical: bool,
    read: Read,
}

pub(super) fn read(
    session: &Session,
    view: &PageView,
    scope: &Scope,
    context: &Context,
) -> Result<Prepared<Work>> {
    let Some(dialogue) = context.dialogue else {
        return Ok(Prepared::Skip(
            "the project has no dialogue style".to_owned(),
        ));
    };
    let layers = super::walk(&view.layers);
    let blocks = super::blocks(&layers);
    let balloons = super::balloons(&layers);
    let translations: Vec<(&LayerView, Vec<usize>)> = super::translations(&layers)
        .into_iter()
        .filter_map(|(translation, covers)| {
            let covers: Option<Vec<usize>> = covers
                .iter()
                .map(|id| blocks.iter().position(|block| block.id == *id))
                .collect();
            Some((translation, covers.filter(|covers| !covers.is_empty())?))
        })
        .filter(|(translation, covers)| match scope {
            Scope::Layers(layers) => {
                layers.contains(&translation.id)
                    || covers
                        .iter()
                        .any(|&block| layers.contains(&blocks[block].id))
            }
            Scope::Page | Scope::Area(_) => true,
        })
        .collect();
    if translations.is_empty() {
        return Ok(Prepared::Skip("the page has no translations".to_owned()));
    }
    let pending: Vec<&(&LayerView, Vec<usize>)> = translations
        .iter()
        .filter(|(translation, _)| {
            context.redo
                || matches!(scope, Scope::Layers(_))
                || matches!(&translation.layer, Layer::Text(text) if text.frame == Frame::Point)
                || super::out_of_date(translation, TYPESETTER)
        })
        .collect();
    if pending.is_empty() {
        return Ok(Prepared::Skip(
            "every translation is already typeset".to_owned(),
        ));
    }
    let read = |layer: LayerId, path: &'static str| -> Result<Read> {
        Ok(session.read(layer, Pointer::parse(path)?)?.1)
    };
    let blocks = blocks
        .iter()
        .filter_map(|block| {
            let Layer::Text(text) = &block.layer else {
                return None;
            };
            let Frame::Box { bounds, .. } = text.frame else {
                return None;
            };
            let dialogue = text
                .attributes
                .get("koharu")
                .and_then(|koharu| koharu.get("role"))
                .is_some_and(|role| role == "dialogue");
            Some((block, text, bounds, dialogue))
        })
        .map(|(block, text, bounds, dialogue)| {
            Ok(Block {
                bounds,
                transform: text.transform,
                direction: text.direction,
                chars: text.style.chars.clone(),
                length: text
                    .content
                    .text()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .count(),
                dialogue,
                balloon: super::balloon_of(block, &balloons),
                reads: [
                    read(block.id, "/frame")?,
                    read(block.id, "/style")?,
                    read(block.id, "/content")?,
                ],
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let translations = pending
        .into_iter()
        .map(|(translation, covers)| {
            let vertical = match &translation.layer {
                Layer::Text(text) => {
                    matches!(text.language.language.as_str(), "ja" | "zh" | "ko" | "yue")
                }
                _ => false,
            };
            Ok(Translation {
                id: translation.id,
                covers: covers.clone(),
                vertical,
                read: read(translation.id, "/attributes/koharu/covers")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let sized = session.style(dialogue)?.style.style.chars.size.is_some();
    Ok(Prepared::Work(Work {
        dialogue,
        sized,
        balloons: balloons
            .iter()
            .map(|(balloon, _)| {
                let Layer::Shape(shape) = &balloon.layer else {
                    unreachable!("balloons are shapes")
                };
                Ok(Balloon {
                    path: shape.path.as_str().to_owned(),
                    read: read(balloon.id, "/path")?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        blocks,
        translations,
    }))
}

pub(super) fn run(project: ProjectId, redo: bool, work: Work) -> Result<Edit> {
    let mut ops = Vec::new();
    // The page's dialogue sets the size, unless another page already has.
    if redo || !work.sized {
        let mut sizes: Vec<f64> = work
            .blocks
            .iter()
            .filter(|block| block.dialogue && block.length > 0)
            .map(|block| {
                (block.bounds.width * block.bounds.height / (block.length as f64 * LINE_HEIGHT))
                    .sqrt()
            })
            .collect();
        sizes.sort_by(f64::total_cmp);
        if let Some(&size) = sizes.get(sizes.len() / 2) {
            ops.push(Op::UpdateTextStyle {
                style: work.dialogue.into(),
                patch: json!({"style": {"chars": {"size": size.round()}}}),
                ingredients: work
                    .blocks
                    .iter()
                    .filter(|block| block.dialogue)
                    .flat_map(|block| [block.reads[0].clone(), block.reads[2].clone()])
                    .collect(),
            });
        }
    }

    // Each balloon's regions, one per block, from a watershed seeded by its
    // blocks on its distance from its edge.
    let regions: Vec<Option<Regions>> = work
        .balloons
        .iter()
        .enumerate()
        .map(|(index, balloon)| {
            let seeds: Vec<usize> = (0..work.blocks.len())
                .filter(|&block| work.blocks[block].balloon == Some(index))
                .collect();
            (seeds.len() > 1)
                .then(|| regions(&balloon.path, &seeds, &work.blocks))
                .flatten()
        })
        .collect();

    for translation in &work.translations {
        let first = &work.blocks[translation.covers[0]];
        let balloon = first.balloon.filter(|&balloon| {
            translation
                .covers
                .iter()
                .all(|&block| work.blocks[block].balloon == Some(balloon))
        });
        let (frame, transform, parent) = match balloon {
            Some(balloon) => {
                let whole = || json!({"path": {"path": work.balloons[balloon].path, "anchor": "center"}, "box": null});
                let frame = match &regions[balloon] {
                    Some(regions) => regions.outline(&translation.covers).map_or_else(
                        whole,
                        |path| json!({"path": {"path": path, "anchor": "center"}, "box": null}),
                    ),
                    None => whole(),
                };
                (
                    frame,
                    Transform::default(),
                    work.balloons[balloon].read.clone(),
                )
            }
            None => {
                let Bounds {
                    x,
                    y,
                    width,
                    height,
                } = first.bounds;
                let frame = json!({
                    "box": {"bounds": {"x": x, "y": y, "width": width, "height": height}, "anchor": "center"},
                    "path": null,
                });
                (frame, first.transform, first.reads[0].clone())
            }
        };
        let direction = if translation.vertical && first.direction == Direction::Vertical {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };
        let ingredients = std::iter::once(parent.parent_of())
            .chain(
                translation
                    .covers
                    .iter()
                    .flat_map(|&block| work.blocks[block].reads[..2].to_vec()),
            )
            .chain([translation.read.clone()])
            .collect();
        // The source's ink.
        let chars = serde_json::to_value(&first.chars)?;
        ops.push(Op::UpdateLayer {
            layer: translation.id.into(),
            patch: json!({
                "frame": frame,
                "transform": transform,
                "text_style": work.dialogue,
                "direction": direction,
                "style": {"chars": {
                    "fill": chars["fill"],
                    "stroke": chars["stroke"],
                    "stroke_width": chars["stroke_width"],
                }},
            }),
            ingredients,
        });
    }
    let action = super::action("c2pa.edited", TYPESETTER, Map::new());
    Ok(super::edit(
        project,
        Step::Typesetting,
        action,
        Vec::new(),
        ops,
    ))
}

/// A balloon cut into one region per seed block.
struct Regions {
    /// The balloon's pixels, from `(x, y)` on the page, each labelled with
    /// its block's index in the page's blocks plus one; 0 is outside.
    labels: ImageBuffer<Luma<u32>, Vec<u32>>,
    x: u32,
    y: u32,
}

impl Regions {
    /// The outline of the union of `blocks`' regions.
    fn outline(&self, blocks: &[usize]) -> Option<String> {
        let mask = GrayImage::from_fn(self.labels.width(), self.labels.height(), |x, y| {
            let label = self.labels.get_pixel(x, y)[0] as usize;
            Luma([u8::from(label > 0 && blocks.contains(&(label - 1))) * u8::MAX])
        });
        super::outline(&mask, self.x, self.y)
    }
}

/// A seeded watershed on the balloon's distance from its edge, so each border
/// falls where the balloon narrows. Each block seeds the balloon's pixels
/// under its box.
fn regions(path: &str, seeds: &[usize], blocks: &[Block]) -> Option<Regions> {
    let path = BezPath::from_svg(path).ok()?;
    let rect = path.bounding_box();
    let (x, y) = (
        rect.x0.floor().max(0.0) as u32,
        rect.y0.floor().max(0.0) as u32,
    );
    let (width, height) = (
        (rect.x1.ceil() as u32).saturating_sub(x) + 2,
        (rect.y1.ceil() as u32).saturating_sub(y) + 2,
    );
    // Local pixel (lx, ly) is page pixel (x + lx - 1, y + ly - 1), so a
    // one-pixel margin outside the balloon rings it.
    let page = |lx: u32, ly: u32| Point::new(f64::from(x + lx) - 0.5, f64::from(y + ly) - 0.5);
    let outside = GrayImage::from_fn(width, height, |lx, ly| {
        Luma([u8::from(!path.contains(page(lx, ly))) * u8::MAX])
    });
    let distance = euclidean_squared_distance_transform(&outside);
    let mut labels = ImageBuffer::<Luma<u32>, Vec<u32>>::new(width, height);
    let mut queue = BinaryHeap::new();
    let mut order = 0u64;
    for &block in seeds {
        let Bounds {
            x: bx,
            y: by,
            width: bw,
            height: bh,
        } = blocks[block].bounds;
        for ly in 0..height {
            for lx in 0..width {
                let point = page(lx, ly);
                let inside_box =
                    point.x >= bx && point.x < bx + bw && point.y >= by && point.y < by + bh;
                if inside_box
                    && outside.get_pixel(lx, ly)[0] == 0
                    && labels.get_pixel(lx, ly)[0] == 0
                {
                    labels.put_pixel(lx, ly, Luma([block as u32 + 1]));
                    queue.push((distance.get_pixel(lx, ly)[0] as u64, Reverse(order), lx, ly));
                    order += 1;
                }
            }
        }
    }
    // Lowest on -distance first: the balloon's middle floods before its necks.
    while let Some((_, _, lx, ly)) = queue.pop() {
        let label = labels.get_pixel(lx, ly)[0];
        for (nx, ny) in [
            (lx.wrapping_sub(1), ly),
            (lx + 1, ly),
            (lx, ly.wrapping_sub(1)),
            (lx, ly + 1),
        ] {
            if nx < width
                && ny < height
                && outside.get_pixel(nx, ny)[0] == 0
                && labels.get_pixel(nx, ny)[0] == 0
            {
                labels.put_pixel(nx, ny, Luma([label]));
                queue.push((distance.get_pixel(nx, ny)[0] as u64, Reverse(order), nx, ny));
                order += 1;
            }
        }
    }
    Some(Regions {
        labels,
        x: x.saturating_sub(1),
        y: y.saturating_sub(1),
    })
}
