//! Inpainting: the cleanup raster's pixels under the text mask, from the
//! scan.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, ensure};
use image::{DynamicImage, GrayImage, ImageBuffer, Luma, Rgb, RgbImage, imageops::FilterType};
use imageproc::{
    distance_transform::{Norm, distance_transform},
    morphology::dilate,
    region_labelling::{Connectivity, connected_components},
};
use koharu_ml::{aot_inpainting, flux3_klein, lama, rorem_mixed};
use koharu_scene::{Edit, Layer, LayerId, MaskContent, Op, PageView, Read, Rect, Session, Tiles};
use serde::{Deserialize, Serialize};
use serde_json::Map;

use super::{Context, Prepared, Scope, detection};
use crate::{Step, residency::Loaded};

/// Pixels the mask grows by, as AOT-GAN "tends to propagate the boundaries"
/// left outside it.
const GROW: u8 = 3;
/// A crop's core around a group of masked areas, and the context around it,
/// IOPaint's default margin.
const CORE: u32 = 512;
const CONTEXT: u32 = 128;
/// Diffusion inpainters' context, and the feather they blend with.
const DIFFUSION_CONTEXT: u32 = 32;
const FEATHER: f32 = 4.0;
/// A balloon is one colour when its unmasked inside, this far from its edge,
/// has at least this many pixels.
const FLAT_EDGE: f32 = 3.0;
const FLAT_PIXELS: usize = 16;
/// An unmasked pixel this far from a balloon's paper colour, in any channel,
/// is a line or art, not paper.
const LINE_CONTRAST: u8 = 64;
/// How much of a masked area may lie outside a balloon's shape while the
/// balloon still holds it: on Blue Archive's comics, text the outline holds
/// lay under a hundredth outside, and text crossing it a fifth or more.
const OUTSIDE: f32 = 0.1;
/// The scene's tile size.
const TILE: i32 = 64;

/// `[pipeline.processor."flux3-klein"]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flux3KleinConfig {
    pub prompt: String,
}

impl Default for Flux3KleinConfig {
    fn default() -> Self {
        Self {
            prompt: "Remove the text and reconstruct the background.".to_owned(),
        }
    }
}

/// `[pipeline.processor."rorem-mixed"]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoremMixedConfig {
    pub prompt: String,
    pub negative_prompt: String,
}

impl Default for RoremMixedConfig {
    fn default() -> Self {
        let options = rorem_mixed::Options::default();
        Self {
            prompt: options.prompt,
            negative_prompt: options.negative_prompt,
        }
    }
}

pub(crate) struct Work {
    scan: Vec<u8>,
    mask: Tiles,
    cleanup: LayerId,
    /// The cleanup's pixels now, so only the tiles that change are written.
    current: Tiles,
    /// Detection's balloons, as polygons, for the flat fill.
    balloons: Vec<Vec<(f32, f32)>>,
    area: Option<Rect>,
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
                "inpainting runs on pages and areas, not text blocks".to_owned(),
            ));
        }
    };
    let Some(scan) = super::scan(view) else {
        return Ok(Prepared::Skip("the page has no scan".to_owned()));
    };
    let layers = super::walk(&view.layers);
    let cleanup = layers.iter().find_map(|layer| {
        let mask = layer.masks.iter().find(|mask| {
            matches!(&mask.layer, Layer::Mask(mask) if matches!(mask.content, MaskContent::Pixels { .. }))
        })?;
        (matches!(layer.layer, Layer::Raster(_)) && super::made_by(layer, detection::MODEL))
            .then_some((*layer, mask))
    });
    let Some((cleanup, mask)) = cleanup else {
        return Ok(Prepared::Skip(
            "the page has no cleanup with a text mask".to_owned(),
        ));
    };
    let current = session.tiles(cleanup.id, None)?;
    if area.is_none() && !context.redo && !current.tiles.is_empty() {
        return Ok(Prepared::Skip("the page is already cleaned".to_owned()));
    }
    let mask = session.tiles(mask.id, None)?;
    if mask.tiles.is_empty() {
        return Ok(Prepared::Skip("the text mask is empty".to_owned()));
    }
    let (scan_file, _) = super::scan_file(session, view.layers[scan].id)?;
    let balloons = layers
        .iter()
        .filter(|layer| super::made_by(layer, detection::MODEL))
        .filter_map(|layer| match &layer.layer {
            Layer::Shape(shape) => polygon(shape.path.as_str()),
            _ => None,
        })
        .collect();
    Ok(Prepared::Work(Work {
        scan: scan_file.bytes,
        mask,
        cleanup: cleanup.id,
        current,
        balloons,
        area,
        reads: vec![super::whole(cleanup)],
    }))
}

/// A closed path's outline as a polygon, its curves flattened.
fn polygon(path: &str) -> Option<Vec<(f32, f32)>> {
    let path = kurbo::BezPath::from_svg(path).ok()?;
    let mut points = Vec::new();
    kurbo::flatten(&path, 0.25, |element| match element {
        kurbo::PathEl::MoveTo(point) | kurbo::PathEl::LineTo(point) => {
            points.push((point.x as f32, point.y as f32));
        }
        _ => {}
    });
    (points.len() >= 3).then_some(points)
}

/// Inpaints each page in turn; the crops of one page go to the model one at a
/// time.
pub(super) fn run(model: &Loaded, works: Vec<Work>, context: &Context) -> Vec<Result<Edit>> {
    works
        .into_iter()
        .map(|work| inpaint(model, work, context))
        .collect()
}

fn inpaint(model: &Loaded, work: Work, context: &Context) -> Result<Edit> {
    let processor = &context.config.processor;
    match model {
        Loaded::Lama(model) => cleanup(context, work, "lama", CONTEXT, false, |image, mask| {
            model.inference(image, mask, &lama::Options::default())
        }),
        Loaded::Aot(model) => cleanup(
            context,
            work,
            "aot-inpainting",
            CONTEXT,
            false,
            |image, mask| model.inference(image, mask, &aot_inpainting::Options::default()),
        ),
        Loaded::Flux3Klein(model) => {
            let prompt = &processor.flux3_klein.prompt;
            cleanup(
                context,
                work,
                "flux3-klein",
                DIFFUSION_CONTEXT,
                true,
                |image, mask| {
                    let mask = DynamicImage::ImageLuma8(mask.clone());
                    let options = flux3_klein::InpaintOptions::default();
                    model.inpaint(prompt, image, None, &mask, &options)
                },
            )
        }
        Loaded::RoremMixed(model) => {
            let settings = &processor.rorem_mixed;
            let options = rorem_mixed::Options {
                prompt: settings.prompt.clone(),
                negative_prompt: settings.negative_prompt.clone(),
                ..rorem_mixed::Options::default()
            };
            cleanup(
                context,
                work,
                "rorem-mixed",
                DIFFUSION_CONTEXT,
                true,
                |image, mask| model.inference(image, mask, &options),
            )
        }
        _ => unreachable!("only inpainters inpaint"),
    }
}

/// The cleanup's pixels inside the run's area: what `infer` paints in each
/// crop of the grown mask, `margin` around it, written opaque on the masked
/// pixels, and nothing elsewhere.
pub(super) fn cleanup(
    context: &Context,
    work: Work,
    agent: &str,
    margin: u32,
    feather: bool,
    infer: impl FnMut(&DynamicImage, &GrayImage) -> Result<RgbImage>,
) -> Result<Edit> {
    let scan = image::load_from_memory(&work.scan)?.to_rgb8();
    let (width, height) = scan.dimensions();
    let page = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let area = work.area.map_or(page, |area| clamp(area, page));
    let mut painted = GrayImage::new(width, height);
    paint(&work.mask, 1, width, area, &mut painted);
    let mask = dilate(&painted, Norm::LInf, GROW);
    let mut output = scan;
    let mut pending = mask.clone();
    fill_flat(&mut output, &mut pending, &painted, &work.balloons);
    crops(&mut output, &pending, margin, feather, infer)?;

    let offset = work.current.offset;
    let mut current = vec![0; (width * height * 4) as usize];
    paint(&work.current, 4, width, page, &mut current);
    let mut next = current.clone();
    for y in area.y as u32..(area.y as u32 + area.height) {
        for x in area.x as u32..(area.x as u32 + area.width) {
            let at = ((y * width + x) * 4) as usize;
            next[at..at + 4].copy_from_slice(&match mask.get_pixel(x, y)[0] {
                0 => [0; 4],
                _ => {
                    let [r, g, b] = output.get_pixel(x, y).0;
                    [r, g, b, u8::MAX]
                }
            });
        }
    }
    let mut ops = Vec::new();
    // Tiles are aligned to the raster's own pixels.
    let first = |start: i32, offset: i32| (start - offset).div_euclid(TILE) * TILE + offset;
    let mut ty = first(area.y, offset.y);
    while ty < area.y + area.height as i32 {
        let mut tx = first(area.x, offset.x);
        while tx < area.x + area.width as i32 {
            let tile = clamp(
                Rect {
                    x: tx,
                    y: ty,
                    width: TILE as u32,
                    height: TILE as u32,
                },
                page,
            );
            let rows = (tile.y..tile.y + tile.height as i32).map(|y| {
                let at = ((y as u32 * width + tile.x as u32) * 4) as usize;
                at..at + (tile.width * 4) as usize
            });
            if rows.clone().any(|row| current[row.clone()] != next[row]) {
                ops.push(Op::WritePixels {
                    layer: work.cleanup.into(),
                    rect: Rect {
                        x: tile.x - offset.x,
                        y: tile.y - offset.y,
                        ..tile
                    },
                    pixels: rows.flat_map(|row| next[row].to_vec()).collect(),
                });
            }
            tx += TILE;
        }
        ty += TILE;
    }
    let action = super::action("c2pa.edited", agent, Map::new());
    Ok(super::edit(
        context.project,
        Step::Inpainting,
        action,
        work.reads,
        ops,
    ))
}

/// Draws decoded `tiles` of `channels` bytes a pixel onto a buffer for a
/// page `width` wide, inside `area`.
fn paint(tiles: &Tiles, channels: usize, width: u32, area: Rect, page: &mut [u8]) {
    for tile in &tiles.tiles {
        for (index, pixel) in tile.pixels.chunks_exact(channels).enumerate() {
            let x = tiles.offset.x + tile.x * TILE + index as i32 % TILE;
            let y = tiles.offset.y + tile.y * TILE + index as i32 / TILE;
            if x >= area.x
                && y >= area.y
                && x < area.x + area.width as i32
                && y < area.y + area.height as i32
            {
                let at = (y as usize * width as usize + x as usize) * channels;
                page[at..at + channels].copy_from_slice(pixel);
            }
        }
    }
}

/// Fills each balloon's masked text with its inside's one colour, when it
/// has one, so the model never sees it. BallonsTranslator fills only text a
/// balloon's outline encloses:
/// https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/utils/textblock_mask.py#L365-L414
/// A detected balloon's shape is approximate and takes in its outline, so
/// here a masked area more than `OUTSIDE` outside the shape runs past the
/// outline and goes to the model whole, which redraws the line under it,
/// when a lettering in it, a part of `painted`, the mask before it grew,
/// lies on both sides of the outline. Grown, a lettering inside can join one
/// outside: then only the letterings inside are filled. In every area, a
/// line or art the mask covers part of goes to the model, with a pixel
/// around it.
fn fill_flat(
    output: &mut RgbImage,
    pending: &mut GrayImage,
    painted: &GrayImage,
    balloons: &[Vec<(f32, f32)>],
) {
    let (width, height) = output.dimensions();
    let areas = connected_components(&*pending, Connectivity::Eight, Luma([0]));
    let mut sizes = HashMap::<u32, u32>::new();
    for area in areas.pixels().filter(|area| area[0] != 0) {
        *sizes.entry(area[0]).or_default() += 1;
    }
    let parts = connected_components(painted, Connectivity::Eight, Luma([0]));
    let (mut part_sizes, mut part_areas) = (HashMap::<u32, u32>::new(), HashMap::new());
    for (x, y, part) in parts.enumerate_pixels().filter(|(_, _, part)| part[0] != 0) {
        *part_sizes.entry(part[0]).or_default() += 1;
        part_areas
            .entry(part[0])
            .or_insert_with(|| areas.get_pixel(x, y)[0]);
    }
    for polygon in balloons {
        let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in polygon {
            (left, top, right, bottom) = (left.min(x), top.min(y), right.max(x), bottom.max(y));
        }
        let [left, top] = [left, top].map(|value| value.floor().max(0.0) as u32);
        let right = (right.ceil().max(0.0) as u32).min(width);
        let bottom = (bottom.ceil().max(0.0) as u32).min(height);
        let mut targets = Vec::new();
        let mut channels = [Vec::new(), Vec::new(), Vec::new()];
        for y in top..bottom {
            for x in left..right {
                let point = (x as f32 + 0.5, y as f32 + 0.5);
                if !inside(point, polygon) {
                    continue;
                }
                if pending.get_pixel(x, y)[0] != 0 {
                    targets.push((x, y));
                } else if edge_distance_squared(point, polygon) >= FLAT_EDGE * FLAT_EDGE {
                    for (channel, value) in channels.iter_mut().zip(output.get_pixel(x, y).0) {
                        channel.push(value);
                    }
                }
            }
        }
        if targets.is_empty() || channels[0].len() < FLAT_PIXELS {
            continue;
        }
        let medians = channels.each_mut().map(|values| median(values));
        let deviations: [f64; 3] = std::array::from_fn(|channel| {
            deviation(&channels[channel], f64::from(medians[channel]))
        });
        let mean = deviations.iter().sum::<f64>() / 3.0;
        let spread = (deviations
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / 3.0)
            .sqrt();
        let threshold = if spread > 1.0 { 7.0 } else { 10.0 };
        if deviations.iter().copied().fold(0.0, f64::max) >= threshold {
            continue;
        }
        let (mut held, mut parts_held) = (HashMap::<u32, u32>::new(), HashMap::<u32, u32>::new());
        for &(x, y) in &targets {
            *held.entry(areas.get_pixel(x, y)[0]).or_default() += 1;
            let part = parts.get_pixel(x, y)[0];
            if part != 0 {
                *parts_held.entry(part).or_default() += 1;
            }
        }
        let inside_enough = |held: u32, size: u32| (1.0 - OUTSIDE) * size as f32 <= held as f32;
        let straddling: HashSet<u32> = parts_held
            .iter()
            .filter(|&(part, &held)| {
                let size = part_sizes[part];
                OUTSIDE * (size as f32) < held as f32 && !inside_enough(held, size)
            })
            .map(|(part, _)| part_areas[part])
            .collect();
        let paper = |x: u32, y: u32| {
            let pixel = output.get_pixel(x, y).0;
            (0..3).all(|channel| pixel[channel].abs_diff(medians[channel]) <= LINE_CONTRAST)
        };
        let masked = |x: u32, y: u32| pending.get_pixel(x, y)[0] != 0;
        let mixed =
            |area: u32| !inside_enough(held[&area], sizes[&area]) && !straddling.contains(&area);
        // A pixel the mask grew to belongs to every lettering it grew from.
        let grow = i64::from(GROW);
        let owners = |x: u32, y: u32| {
            (-grow..=grow)
                .flat_map(|dy| (-grow..=grow).map(move |dx| (dx, dy)))
                .map(move |(dx, dy)| (i64::from(x) + dx, i64::from(y) + dy))
                .filter(|&(nx, ny)| {
                    (0..i64::from(width)).contains(&nx) && (0..i64::from(height)).contains(&ny)
                })
                .map(|(nx, ny)| parts.get_pixel(nx as u32, ny as u32)[0])
                .filter(|&part| part != 0)
        };
        // In a mixed area, a lettering inside is filled only when paper alone
        // borders it: one on the outline or on art the shape takes in goes
        // to the model with the rest.
        let mut bordered = HashSet::new();
        for &(x, y) in &targets {
            if !mixed(areas.get_pixel(x, y)[0]) {
                continue;
            }
            let off_paper = [(0i64, -1i64), (0, 1), (-1, 0), (1, 0)]
                .into_iter()
                .map(|(dx, dy)| (i64::from(x) + dx, i64::from(y) + dy))
                .filter(|&(nx, ny)| {
                    (0..i64::from(width)).contains(&nx) && (0..i64::from(height)).contains(&ny)
                })
                .any(|(nx, ny)| !masked(nx as u32, ny as u32) && !paper(nx as u32, ny as u32));
            if off_paper {
                bordered.extend(owners(x, y));
            }
        }
        targets.retain(|&(x, y)| {
            let area = areas.get_pixel(x, y)[0];
            if inside_enough(held[&area], sizes[&area]) {
                return true;
            }
            if straddling.contains(&area) {
                return false;
            }
            owners(x, y).all(|part| {
                inside_enough(
                    parts_held.get(&part).copied().unwrap_or(0),
                    part_sizes[&part],
                ) && !bordered.contains(&part)
            })
        });
        let neighbours = |x: u32, y: u32| {
            (-1i64..=1)
                .flat_map(move |dy| (-1i64..=1).map(move |dx| (dx, dy)))
                .map(move |(dx, dy)| (i64::from(x) + dx, i64::from(y) + dy))
                .filter(|&(nx, ny)| {
                    (0..i64::from(width)).contains(&nx) && (0..i64::from(height)).contains(&ny)
                })
                .map(|(nx, ny)| (nx as u32, ny as u32))
        };
        // Off-paper pixels under the mask that meet off-paper ones past it,
        // and those they lead to under the mask.
        let mut line: HashSet<(u32, u32)> = targets
            .iter()
            .copied()
            .filter(|&(x, y)| {
                !paper(x, y) && neighbours(x, y).any(|(nx, ny)| !masked(nx, ny) && !paper(nx, ny))
            })
            .collect();
        let mut queue: Vec<(u32, u32)> = line.iter().copied().collect();
        while let Some((x, y)) = queue.pop() {
            for (nx, ny) in neighbours(x, y) {
                if (left..right).contains(&nx)
                    && (top..bottom).contains(&ny)
                    && masked(nx, ny)
                    && !paper(nx, ny)
                    && line.insert((nx, ny))
                {
                    queue.push((nx, ny));
                }
            }
        }
        let filled: Vec<(u32, u32)> = targets
            .into_iter()
            .filter(|&(x, y)| !neighbours(x, y).any(|point| line.contains(&point)))
            .collect();
        for (x, y) in filled {
            output.put_pixel(x, y, Rgb(medians));
            pending.put_pixel(x, y, Luma([0]));
        }
    }
}

/// Inpaints `mask` on `output` in crops: each groups masked areas into a core
/// of at most `CORE` pixels, with `context` around it. An area larger than a
/// core gets a crop of its own, downscaled to fit. The result is written on
/// the group's masked pixels only, feathered when `feather`.
fn crops(
    output: &mut RgbImage,
    mask: &GrayImage,
    context: u32,
    feather: bool,
    mut infer: impl FnMut(&DynamicImage, &GrayImage) -> Result<RgbImage>,
) -> Result<()> {
    let (width, height) = output.dimensions();
    let (labels, groups) = groups(mask);
    let inside = feather.then(|| {
        let unmasked = GrayImage::from_fn(width, height, |x, y| {
            Luma([u8::from(mask.get_pixel(x, y)[0] == 0) * u8::MAX])
        });
        distance_transform(&unmasked, Norm::LInf)
    });
    let fit = CORE + 2 * context;
    for group in &groups {
        let [left, top, right, bottom] = group.core;
        let [left, top] = [left, top].map(|value| value.saturating_sub(context));
        let right = (right + context).min(width);
        let bottom = (bottom + context).min(height);
        let (crop_width, crop_height) = (right - left, bottom - top);
        let mut image =
            image::imageops::crop_imm(output, left, top, crop_width, crop_height).to_image();
        let mut crop_mask =
            image::imageops::crop_imm(mask, left, top, crop_width, crop_height).to_image();
        let scale = f64::from(fit) / f64::from(crop_width.max(crop_height));
        if scale < 1.0 {
            let (w, h) = (
                ((f64::from(crop_width) * scale).round() as u32).max(1),
                ((f64::from(crop_height) * scale).round() as u32).max(1),
            );
            image = image::imageops::resize(&image, w, h, FilterType::Lanczos3);
            crop_mask = image::imageops::resize(&crop_mask, w, h, FilterType::Nearest);
        }
        let mut generated = infer(&DynamicImage::ImageRgb8(image), &crop_mask)?;
        if generated.dimensions() != (crop_width, crop_height) {
            generated =
                image::imageops::resize(&generated, crop_width, crop_height, FilterType::Lanczos3);
        }
        for y in top..bottom {
            for x in left..right {
                if !group.components.contains(&labels.get_pixel(x, y)[0]) {
                    continue;
                }
                let new = generated.get_pixel(x - left, y - top).0;
                let alpha = inside.as_ref().map_or(1.0, |inside| {
                    (f32::from(inside.get_pixel(x, y)[0]) / FEATHER).min(1.0)
                });
                let old = output.get_pixel(x, y).0;
                let blend = |index: usize| {
                    (f32::from(old[index]) * (1.0 - alpha) + f32::from(new[index]) * alpha).round()
                        as u8
                };
                output.put_pixel(x, y, Rgb([blend(0), blend(1), blend(2)]));
            }
        }
    }
    ensure!(
        output.dimensions() == (width, height),
        "the inpainted page changed size"
    );
    Ok(())
}

struct Group {
    components: Vec<u32>,
    core: [u32; 4],
}

/// The mask's connected areas, labelled, and grouped into cores of at most
/// `CORE` pixels a side, each growing the least it can; an area larger than a
/// core is a group of its own.
fn groups(mask: &GrayImage) -> (ImageBuffer<Luma<u32>, Vec<u32>>, Vec<Group>) {
    let labels = connected_components(mask, Connectivity::Eight, Luma([0]));
    let mut bounds = Vec::<Option<[u32; 4]>>::new();
    for (x, y, pixel) in labels.enumerate_pixels() {
        let label = pixel[0] as usize;
        if label == 0 {
            continue;
        }
        if label >= bounds.len() {
            bounds.resize(label + 1, None);
        }
        let area = bounds[label].get_or_insert([x, y, x + 1, y + 1]);
        *area = [
            area[0].min(x),
            area[1].min(y),
            area[2].max(x + 1),
            area[3].max(y + 1),
        ];
    }
    let union = |a: [u32; 4], b: [u32; 4]| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    };
    let size =
        |[left, top, right, bottom]: [u32; 4]| u64::from(right - left) * u64::from(bottom - top);
    let fits = |[left, top, right, bottom]: [u32; 4]| right - left <= CORE && bottom - top <= CORE;
    let mut groups: Vec<Group> = Vec::new();
    for (label, area) in bounds.into_iter().enumerate().skip(1) {
        let Some(area) = area else { continue };
        let label = label as u32;
        let best = fits(area)
            .then(|| {
                groups
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| fits(group.core) && fits(union(group.core, area)))
                    .min_by_key(|(index, group)| {
                        (size(union(group.core, area)) - size(group.core), *index)
                    })
                    .map(|(index, _)| index)
            })
            .flatten();
        match best {
            Some(index) => {
                groups[index].components.push(label);
                groups[index].core = union(groups[index].core, area);
            }
            None => groups.push(Group {
                components: vec![label],
                core: area,
            }),
        }
    }
    (labels, groups)
}

fn inside(point: (f32, f32), polygon: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        if (current.1 > point.1) != (previous.1 > point.1) {
            let x = (previous.0 - current.0) * (point.1 - current.1) / (previous.1 - current.1)
                + current.0;
            if point.0 < x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn edge_distance_squared(point: (f32, f32), polygon: &[(f32, f32)]) -> f32 {
    let mut minimum = f32::INFINITY;
    let mut start = polygon[polygon.len() - 1];
    for &end in polygon {
        let segment = (end.0 - start.0, end.1 - start.1);
        let length = segment.0 * segment.0 + segment.1 * segment.1;
        let along = if length > 0.0 {
            ((point.0 - start.0) * segment.0 + (point.1 - start.1) * segment.1) / length
        } else {
            0.0
        }
        .clamp(0.0, 1.0);
        let (dx, dy) = (
            point.0 - start.0 - segment.0 * along,
            point.1 - start.1 - segment.1 * along,
        );
        minimum = minimum.min(dx * dx + dy * dy);
        start = end;
    }
    minimum
}

fn median(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        ((u16::from(values[middle - 1]) + u16::from(values[middle])) / 2) as u8
    } else {
        values[middle]
    }
}

fn deviation(values: &[u8], center: f64) -> f64 {
    (values
        .iter()
        .map(|value| (f64::from(*value) - center).powi(2))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

/// `area` inside `page`.
fn clamp(area: Rect, page: Rect) -> Rect {
    let x = area.x.clamp(page.x, page.x + page.width as i32);
    let y = area.y.clamp(page.y, page.y + page.height as i32);
    let right = (area.x + area.width as i32).clamp(x, page.x + page.width as i32);
    let bottom = (area.y + area.height as i32).clamp(y, page.y + page.height as i32);
    Rect {
        x,
        y,
        width: (right - x) as u32,
        height: (bottom - y) as u32,
    }
}
