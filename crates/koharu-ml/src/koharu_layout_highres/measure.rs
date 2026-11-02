//! The shape and typography of a detection's lettering, on the page under
//! its mask. RF-DETR's postprocessing stays as upstream has it: `glyph_mask`
//! reshapes a lettering mask after it, and `Detection::typography` measures
//! one when a caller asks.

use std::collections::{HashMap, HashSet};

use image::{GrayImage, Luma, RgbImage, imageops};
use imageproc::{
    distance_transform::Norm,
    filter::gaussian_blur_f32,
    gradients::{horizontal_sobel, vertical_sobel},
    morphology::{close, dilate},
    region_labelling::{Connectivity, connected_components},
};
use koharu_leptonica::{color_segment, find_stroke_width, otsu_adaptive_threshold};
use serde::Serialize;

use super::{Detection, Mask};

/// What a detection's lettering shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Typography {
    /// Whether the lines run down the page: the lettering's block is longer
    /// that way, at its tilt.
    pub vertical: bool,
    /// How far the lines turn clockwise from horizontal, or from vertical for
    /// vertical lines, in degrees within ±45°: the commonest direction of the
    /// block's edges. A turn that moves the lines' far end by less than a
    /// stroke is 0: it can't be told from the glyphs' ragged edges.
    pub tilt: f32,
    pub stroke_width: Option<f32>,
    /// The commonest colour segment under the ink, and around it.
    pub ink: Option<[u8; 3]>,
    pub paper: Option<[u8; 3]>,
}

impl Detection {
    /// The detection's typography on `page`, or `None` when its block holds
    /// no ink.
    pub fn typography(&self, page: &RgbImage) -> Option<Typography> {
        let block = block(&self.mask, page.width(), page.height());
        let (crop, ink, _) = ink(&block, page, 0, None)?;
        let (width, height) = (block.width, block.height);
        let inside = |x: u32, y: u32| block.pixels[(y * width + x) as usize] != 0;

        // The block's edges are the sides and ends of its lines, and their
        // commonest direction, modulo 90°, is the lines' tilt. The edges'
        // gradients are averaged at four times their angle, which folds sides
        // and ends onto one direction. Leptonica's skew finder, scoring the
        // ink's profiles, flipped direction on 14% of blocks turned by up to
        // 20° and followed only 82% of the turn; the block's hull, by its
        // minimum-area rectangle or narrowest side, tilted wherever columns
        // of uneven length step. Only the block's body counts, the parts at
        // least a tenth the size of its largest: a stray fragment of a
        // neighbour's mask is no line.
        let shape = GrayImage::from_raw(width, height, block.pixels.clone())
            .expect("a mask holds a byte a pixel");
        let parts = connected_components(&shape, Connectivity::Eight, Luma([0]));
        let mut sizes = HashMap::<u32, u32>::new();
        for part in parts.pixels().filter(|part| part.0[0] != 0) {
            *sizes.entry(part.0[0]).or_default() += 1;
        }
        let largest = sizes.values().copied().max().unwrap_or(0);
        let body = |part: u32| part != 0 && 10 * sizes[&part] >= largest;
        // A border, so the edges at the crop's sides have gradients too.
        let border = 4;
        let body_image = GrayImage::from_fn(width + 2 * border, height + 2 * border, |x, y| {
            let inside = (border..border + width).contains(&x)
                && (border..border + height).contains(&y)
                && body(parts.get_pixel(x - border, y - border).0[0]);
            Luma([if inside { 255 } else { 0 }])
        });
        let smooth = gaussian_blur_f32(&body_image, 1.5);
        let (across, down) = (horizontal_sobel(&smooth), vertical_sobel(&smooth));
        let (mut cosine, mut sine) = (0.0f64, 0.0f64);
        for (gx, gy) in across.pixels().zip(down.pixels()) {
            let (gx, gy) = (f64::from(gx.0[0]), f64::from(gy.0[0]));
            let strength = gx.hypot(gy);
            if strength > 0.0 {
                let angle = 4.0 * gy.atan2(gx);
                cosine += strength * angle.cos();
                sine += strength * angle.sin();
            }
        }
        let angle = (sine.atan2(cosine) / 4.0).to_degrees() as f32;

        // The lines run along the body's longer extent at that angle.
        let (sin, cos) = angle.to_radians().sin_cos();
        let (mut along_x, mut along_y) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN));
        for (x, y, part) in parts.enumerate_pixels() {
            if body(part.0[0]) {
                let (x, y) = (x as f32, y as f32);
                let (u, v) = (x * cos + y * sin, y * cos - x * sin);
                along_x = (along_x.0.min(u), along_x.1.max(u));
                along_y = (along_y.0.min(v), along_y.1.max(v));
            }
        }
        let (extent_x, extent_y) = (along_x.1 - along_x.0 + 1.0, along_y.1 - along_y.0 + 1.0);
        let vertical = if (extent_x - extent_y).abs() < 1.0 {
            height > width
        } else {
            extent_y > extent_x
        };
        let stroke_width = find_stroke_width(&ink, 0.15);
        let length = extent_x.max(extent_y);
        let resolution = (stroke_width.unwrap_or(1.0) / length).atan().to_degrees();
        let tilt = if angle.abs() < resolution { 0.0 } else { angle };

        let (ink_color, paper_color) = match color_segment(&crop, 60, 8, 1, 4) {
            Ok(Some(segments)) => {
                let commonest = |wanted: bool| {
                    let mut counts = vec![0u32; segments.colors.len()];
                    for (x, y, index) in segments.indices.enumerate_pixels() {
                        if inside(x, y) && (ink.get_pixel(x, y).0[0] != 0) == wanted {
                            counts[usize::from(index.0[0])] += 1;
                        }
                    }
                    let (index, &count) =
                        counts.iter().enumerate().max_by_key(|&(_, count)| count)?;
                    (count > 0).then(|| segments.colors[index])
                };
                (commonest(true), commonest(false))
            }
            _ => (None, None),
        };

        Some(Typography {
            vertical,
            tilt,
            stroke_width,
            ink: ink_color,
            paper: paper_color,
        })
    }
}

/// A text or onomatopoeia mask fitted to its glyphs: RF-DETR's mask within
/// its box, which hugs each stroke and its outline but misses counters and
/// parts of strokes, with its block's ink, widened by 2 pixels per 1024 of the
/// page's longer side for anti-aliased edges. The paper or art between glyphs
/// stays out, so inpainting redraws only the lettering.
pub(super) fn glyph_mask(mask: &Mask, bbox: [f32; 4], page: &RgbImage) -> Mask {
    let (page_width, page_height) = (page.width(), page.height());
    // RF-DETR's mask can stop a few pixels short of a glyph's ends, and LaMa
    // grows even a sliver of ink left outside into a stroke, so the ink may
    // reach as far past the block as the block's closing radius.
    let reach = scaled(6.0, page_width, page_height);
    // RF-DETR predicts each query's mask over the whole page, and a lettering
    // mask can take in specks of other lettering far from its box, whose
    // surroundings then sway the ink split. Its own glyphs can run past the
    // box too, but by less than a glyph: the box, as much again as its
    // narrower side around it, bounds the mask.
    let [left, top, right, bottom] = bbox;
    let slack = (right - left).min(bottom - top);
    let clipped = GrayImage::from_fn(mask.width, mask.height, |x, y| {
        let (page_x, page_y) = ((mask.x + x) as f32, (mask.y + y) as f32);
        let kept = mask.pixels[(y * mask.width + x) as usize] != 0
            && (left - slack..right + slack).contains(&page_x)
            && (top - slack..bottom + slack).contains(&page_y);
        Luma([if kept { 255 } else { 0 }])
    });
    let mask = crop(
        &clipped,
        i64::from(mask.x),
        i64::from(mask.y),
        page_width,
        page_height,
    );
    let block = block(&mask, page_width, page_height);
    let Some((_, ink, [left, top])) = ink(&block, page, reach, Some(bbox)) else {
        return mask;
    };
    let margin = scaled(2.0, page_width, page_height);
    let (left, top) = (
        i64::from(left) - i64::from(margin),
        i64::from(top) - i64::from(margin),
    );
    let mut canvas = GrayImage::new(ink.width() + 2 * margin, ink.height() + 2 * margin);
    for (x, y, pixel) in ink.enumerate_pixels() {
        if pixel.0[0] != 0 {
            canvas.put_pixel(x + margin, y + margin, Luma([255]));
        }
    }
    for (index, _) in mask
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, value)| **value != 0)
    {
        let x = i64::from(mask.x + index as u32 % mask.width) - left;
        let y = i64::from(mask.y + index as u32 / mask.width) - top;
        if (0..i64::from(canvas.width())).contains(&x)
            && (0..i64::from(canvas.height())).contains(&y)
        {
            canvas.put_pixel(x as u32, y as u32, Luma([255]));
        }
    }
    // Inside the box, what the glyphs enclose is a counter or, in lettering
    // outlined in a lighter colour, the glyph's own fill; the ink split takes
    // only the outline there.
    let [box_left, box_top, box_right, box_bottom] = bbox;
    let mut enclosed = canvas.clone();
    fill_holes(&mut enclosed);
    for (x, y, pixel) in enclosed.enumerate_pixels() {
        let (page_x, page_y) = ((left + i64::from(x)) as f32, (top + i64::from(y)) as f32);
        if pixel.0[0] != 0
            && (box_left..box_right).contains(&page_x)
            && (box_top..box_bottom).contains(&page_y)
        {
            canvas.put_pixel(x, y, Luma([255]));
        }
    }
    let fitted = dilate(&canvas, Norm::L2, margin as u8);
    crop(&fitted, left, top, page_width, page_height)
}

/// Fills what `canvas`'s marked pixels enclose: unmarked areas its edge
/// can't reach.
fn fill_holes(canvas: &mut GrayImage) {
    let (width, height) = (canvas.width(), canvas.height());
    if width == 0 || height == 0 {
        return;
    }
    let background = GrayImage::from_fn(width, height, |x, y| {
        Luma([u8::MAX - canvas.get_pixel(x, y).0[0]])
    });
    let regions = connected_components(&background, Connectivity::Four, Luma([0]));
    let mut outside = HashSet::new();
    for x in 0..width {
        outside.insert(regions.get_pixel(x, 0).0[0]);
        outside.insert(regions.get_pixel(x, height - 1).0[0]);
    }
    for y in 0..height {
        outside.insert(regions.get_pixel(0, y).0[0]);
        outside.insert(regions.get_pixel(width - 1, y).0[0]);
    }
    for (x, y, region) in regions.enumerate_pixels() {
        if region.0[0] != 0 && !outside.contains(&region.0[0]) {
            canvas.put_pixel(x, y, Luma([255]));
        }
    }
}

/// The lettering's block: `mask` closed and with its holes filled, so its
/// lines are solid and its outline is their sides and ends. The radius is
/// the pipeline's text-mask rule, 6 pixels per 1024 of the page's longer
/// side.
fn block(mask: &Mask, page_width: u32, page_height: u32) -> Mask {
    if mask.width == 0 || mask.height == 0 {
        return mask.clone();
    }
    let radius = scaled(6.0, page_width, page_height);
    // Twice the radius, so the dilation never reaches the canvas's edge,
    // where imageproc's erosion would keep it.
    let pad = 2 * radius;
    let mut canvas = GrayImage::new(mask.width + 2 * pad, mask.height + 2 * pad);
    for (index, _) in mask
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, value)| **value != 0)
    {
        let (x, y) = (index as u32 % mask.width, index as u32 / mask.width);
        canvas.put_pixel(x + pad, y + pad, Luma([255]));
    }
    let mut closed = close(&canvas, Norm::LInf, radius as u8);
    fill_holes(&mut closed);
    let (left, top) = (
        i64::from(mask.x) - i64::from(pad),
        i64::from(mask.y) - i64::from(pad),
    );
    crop(&closed, left, top, page_width, page_height)
}

/// The page's crop around the block and the detection's box, `reach` pixels
/// wider than both on every side, its ink, and the crop's place on the page.
/// The ink is the side of one Otsu split of the crop that isn't the paper:
/// inside the block, in the glyph ends it cut off, and on plain paper in the
/// glyph-sized parts wholly inside the box or in line with it. The paper is
/// what the crop shows around the block, or with none around, the side
/// covering more of the block, so light lettering on dark counts too. `None`
/// when the block holds no ink.
fn ink(
    block: &Mask,
    page: &RgbImage,
    reach: u32,
    bbox: Option<[f32; 4]>,
) -> Option<(RgbImage, GrayImage, [u32; 2])> {
    if block.width == 0 || block.height == 0 {
        return None;
    }
    let boxed = bbox.map(|[left, top, right, bottom]| {
        [
            left.floor().max(0.0) as u32,
            top.floor().max(0.0) as u32,
            (right.ceil().max(0.0) as u32).min(page.width()),
            (bottom.ceil().max(0.0) as u32).min(page.height()),
        ]
    });
    // The box, lengthened at both ends of its line by the line's width: a
    // glyph the box cuts off lies in line with the others.
    let lined = boxed.map(|[box_left, box_top, box_right, box_bottom]| {
        let (box_width, box_height) = (
            box_right.saturating_sub(box_left),
            box_bottom.saturating_sub(box_top),
        );
        let line = box_width.min(box_height);
        if box_height >= box_width {
            [
                box_left,
                box_top.saturating_sub(line),
                box_right,
                (box_bottom + line).min(page.height()),
            ]
        } else {
            [
                box_left.saturating_sub(line),
                box_top,
                (box_right + line).min(page.width()),
                box_bottom,
            ]
        }
    });
    let (mut left, mut top) = (block.x.saturating_sub(reach), block.y.saturating_sub(reach));
    let (mut right, mut bottom) = (
        (block.x + block.width + reach).min(page.width()),
        (block.y + block.height + reach).min(page.height()),
    );
    // `reach` past the lengthened box too, so ink crossing its edge shows
    // outside it.
    if let Some([box_left, box_top, box_right, box_bottom]) = lined {
        (left, top) = (
            left.min(box_left.saturating_sub(reach)),
            top.min(box_top.saturating_sub(reach)),
        );
        (right, bottom) = (
            right.max((box_right + reach).min(page.width())),
            bottom.max((box_bottom + reach).min(page.height())),
        );
    }
    let (width, height) = (right - left, bottom - top);
    let crop = imageops::crop_imm(page, left, top, width, height).to_image();
    let (offset_x, offset_y) = (block.x - left, block.y - top);
    let inside = |x: u32, y: u32| {
        (offset_x..offset_x + block.width).contains(&x)
            && (offset_y..offset_y + block.height).contains(&y)
            && block.pixels[((y - offset_y) * block.width + x - offset_x) as usize] != 0
    };
    // One Otsu split of the block and the detection's box, without the
    // reach around them, where art dark enough to take the split from faint
    // lettering can lie. One tile covers the region, so the split is a
    // threshold, read back from it to divide the whole crop.
    let gray = imageops::grayscale(&crop);
    let [region_left, region_top, region_right, region_bottom] = boxed.map_or(
        [left, top, right, bottom],
        |[box_left, box_top, box_right, box_bottom]| {
            [
                box_left.min(block.x).max(left),
                box_top.min(block.y).max(top),
                box_right.max(block.x + block.width).min(right),
                box_bottom.max(block.y + block.height).min(bottom),
            ]
        },
    );
    let (region_width, region_height) = (region_right - region_left, region_bottom - region_top);
    let region = imageops::crop_imm(
        &gray,
        region_left - left,
        region_top - top,
        region_width,
        region_height,
    )
    .to_image();
    let split = otsu_adaptive_threshold(
        &region,
        region_width.max(16),
        region_height.max(16),
        0,
        0,
        0.1,
    )
    .ok()?;
    let threshold = region
        .pixels()
        .zip(split.pixels())
        .filter(|(_, below)| below.0[0] != 0)
        .map(|(value, _)| u16::from(value.0[0]) + 1)
        .max()
        .unwrap_or(0);
    let dark = GrayImage::from_fn(width, height, |x, y| {
        Luma([if u16::from(gray.get_pixel(x, y).0[0]) < threshold {
            255
        } else {
            0
        }])
    });
    let (mut around, mut dark_around, mut covered, mut dark_covered) = (0u32, 0u32, 0u32, 0u32);
    for (x, y, pixel) in dark.enumerate_pixels() {
        let is_dark = u32::from(pixel.0[0] != 0);
        if inside(x, y) {
            covered += 1;
            dark_covered += is_dark;
        } else {
            around += 1;
            dark_around += is_dark;
        }
    }
    let light_ink = if around > 0 {
        2 * dark_around > around
    } else {
        2 * dark_covered > covered
    };
    let is_ink = |x: u32, y: u32| (dark.get_pixel(x, y).0[0] != 0) != light_ink;

    // Past the block, ink that joins the block's and ends within `reach` is a
    // glyph's end the mask cut off; ink that runs on is an outline, a border,
    // or art.
    let shape = GrayImage::from_fn(width, height, |x, y| {
        Luma([if inside(x, y) { 255 } else { 0 }])
    });
    let near = dilate(&shape, Norm::LInf, reach as u8);
    let ring = GrayImage::from_fn(width, height, |x, y| {
        Luma([
            if near.get_pixel(x, y).0[0] != 0 && !inside(x, y) && is_ink(x, y) {
                255
            } else {
                0
            },
        ])
    });
    let parts = connected_components(&ring, Connectivity::Eight, Luma([0]));
    let (mut joins, mut runs_on) = (HashSet::new(), HashSet::new());
    for (x, y, part) in parts.enumerate_pixels() {
        let part = part.0[0];
        if part == 0 {
            continue;
        }
        for (dx, dy) in (-1i64..=1).flat_map(|dx| (-1i64..=1).map(move |dy| (dx, dy))) {
            let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
            if !(0..i64::from(width)).contains(&nx) || !(0..i64::from(height)).contains(&ny) {
                runs_on.insert(part);
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            if near.get_pixel(nx, ny).0[0] == 0 {
                runs_on.insert(part);
            } else if inside(nx, ny) && is_ink(nx, ny) {
                joins.insert(part);
            }
        }
    }
    // On plain paper every glyph-sized ink part wholly inside the box is
    // lettering: RF-DETR's mask can miss a thin glyph, such as 一, whole. So
    // is one in line with the box, past its ends, when the paper there is
    // plain too: the box can miss it as well. The paper is plain when nearly
    // all of it away from the ink's edges is near its median colour; on art
    // no part counts.
    let all_ink = GrayImage::from_fn(width, height, |x, y| {
        Luma([if is_ink(x, y) { 255 } else { 0 }])
    });
    let pieces = connected_components(&all_ink, Connectivity::Eight, Luma([0]));
    let mut lettering = HashSet::new();
    if let (Some(boxed), Some(lined)) = (boxed, lined) {
        let within = |[box_left, box_top, box_right, box_bottom]: [u32; 4], x: u32, y: u32| {
            (box_left..box_right).contains(&(x + left))
                && (box_top..box_bottom).contains(&(y + top))
        };
        let edges = dilate(&all_ink, Norm::LInf, 2);
        let plain = |region: [u32; 4]| {
            let mut channels = [Vec::new(), Vec::new(), Vec::new()];
            for (x, y, pixel) in crop.enumerate_pixels() {
                if within(region, x, y) && edges.get_pixel(x, y).0[0] == 0 {
                    for (channel, value) in channels.iter_mut().zip(pixel.0) {
                        channel.push(value);
                    }
                }
            }
            channels[0].len() >= 16 && {
                let medians = channels.each_ref().map(|values| {
                    let mut sorted = values.clone();
                    let middle = sorted.len() / 2;
                    sorted.select_nth_unstable(middle).1.to_owned()
                });
                let close = (0..channels[0].len())
                    .filter(|&index| {
                        (0..3).all(|channel| {
                            channels[channel][index].abs_diff(medians[channel]) <= PAPER_TOLERANCE
                        })
                    })
                    .count();
                close as f32 >= PLAIN_PAPER * channels[0].len() as f32
            }
        };
        let (plain_box, plain_line) = (plain(boxed), plain(lined));
        if plain_box || plain_line {
            // A glyph is no longer than its line is wide: a longer part, such
            // as a ruled line under the text, is art.
            let [box_left, box_top, box_right, box_bottom] = boxed;
            let line = box_right
                .saturating_sub(box_left)
                .min(box_bottom.saturating_sub(box_top));
            let mut extents = HashMap::<u32, [u32; 4]>::new();
            let (mut leaves_box, mut leaves_line) = (HashSet::new(), HashSet::new());
            for (x, y, piece) in pieces.enumerate_pixels() {
                let piece = piece.0[0];
                if piece == 0 {
                    continue;
                }
                if !within(boxed, x, y) {
                    leaves_box.insert(piece);
                }
                if !within(lined, x, y) {
                    leaves_line.insert(piece);
                }
                let extent = extents.entry(piece).or_insert([x, y, x, y]);
                *extent = [
                    extent[0].min(x),
                    extent[1].min(y),
                    extent[2].max(x),
                    extent[3].max(y),
                ];
            }
            lettering = extents
                .into_iter()
                .filter(|(piece, [x0, y0, x1, y1])| {
                    (x1 - x0 + 1).max(y1 - y0 + 1) <= line
                        && ((plain_box && !leaves_box.contains(piece))
                            || (plain_line && !leaves_line.contains(piece)))
                })
                .map(|(piece, _)| piece)
                .collect();
        }
    }
    let ink = GrayImage::from_fn(width, height, |x, y| {
        let part = parts.get_pixel(x, y).0[0];
        let cut_off = joins.contains(&part) && !runs_on.contains(&part);
        Luma([
            if (inside(x, y) && is_ink(x, y))
                || cut_off
                || lettering.contains(&pieces.get_pixel(x, y).0[0])
            {
                255
            } else {
                0
            },
        ])
    });
    ink.pixels()
        .any(|pixel| pixel.0[0] != 0)
        .then_some((crop, ink, [left, top]))
}

/// How far from the paper's median colour, in every channel, a pixel of
/// plain paper lies, and how much of it must.
const PAPER_TOLERANCE: u8 = 32;
const PLAIN_PAPER: f32 = 0.99;

/// `per_1024` pixels for every 1024 of the page's longer side.
fn scaled(per_1024: f32, page_width: u32, page_height: u32) -> u32 {
    ((page_width.max(page_height) as f32 / 1024.0) * per_1024)
        .round()
        .clamp(1.0, 127.0) as u32
}

/// The covered part of `canvas`, placed at (`left`, `top`) on a page, cut to
/// the page.
fn crop(canvas: &GrayImage, left: i64, top: i64, page_width: u32, page_height: u32) -> Mask {
    let on_page = |x: u32, y: u32| {
        let (page_x, page_y) = (left + i64::from(x), top + i64::from(y));
        (0..i64::from(page_width)).contains(&page_x)
            && (0..i64::from(page_height)).contains(&page_y)
    };
    let covered: Vec<(u32, u32)> = canvas
        .enumerate_pixels()
        .filter(|&(x, y, pixel)| pixel.0[0] != 0 && on_page(x, y))
        .map(|(x, y, _)| (x, y))
        .collect();
    let Some(min_x) = covered.iter().map(|&(x, _)| x).min() else {
        return Mask {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    };
    let min_y = covered.iter().map(|&(_, y)| y).min().unwrap_or(0);
    let max_x = covered.iter().map(|&(x, _)| x).max().unwrap_or(0);
    let max_y = covered.iter().map(|&(_, y)| y).max().unwrap_or(0);
    let (width, height) = (max_x + 1 - min_x, max_y + 1 - min_y);
    let mut pixels = vec![0u8; (width * height) as usize];
    for &(x, y) in &covered {
        pixels[((y - min_y) * width + x - min_x) as usize] = 255;
    }
    Mask {
        x: (left + i64::from(min_x)) as u32,
        y: (top + i64::from(min_y)) as u32,
        width,
        height,
        pixels,
    }
}
