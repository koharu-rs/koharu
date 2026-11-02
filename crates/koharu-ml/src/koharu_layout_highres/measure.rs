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
        let (crop, ink) = ink(&block, page)?;
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

/// A text or onomatopoeia mask fitted to its glyphs: RF-DETR's mask, which
/// hugs each stroke and its outline but misses counters and parts of
/// strokes, with the ink its block holds, widened by 2 pixels per 1024 of the
/// page's longer side for anti-aliased edges. The paper or art between
/// glyphs stays out, so inpainting redraws only the lettering.
pub(super) fn glyph_mask(mask: &Mask, page: &RgbImage) -> Mask {
    let block = block(mask, page.width(), page.height());
    let Some((_, ink)) = ink(&block, page) else {
        return mask.clone();
    };
    let margin = scaled(2.0, page.width(), page.height());
    let (left, top) = (
        i64::from(block.x) - i64::from(margin),
        i64::from(block.y) - i64::from(margin),
    );
    let mut canvas = GrayImage::new(block.width + 2 * margin, block.height + 2 * margin);
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
    let fitted = dilate(&canvas, Norm::L2, margin as u8);
    crop(&fitted, left, top, page.width(), page.height())
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

    // Background the canvas's edge can't reach is a hole.
    let background = GrayImage::from_fn(closed.width(), closed.height(), |x, y| {
        Luma([u8::MAX - closed.get_pixel(x, y).0[0]])
    });
    let regions = connected_components(&background, Connectivity::Four, Luma([0]));
    let (width, height) = (closed.width(), closed.height());
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
            closed.put_pixel(x, y, Luma([255]));
        }
    }
    let (left, top) = (
        i64::from(mask.x) - i64::from(pad),
        i64::from(mask.y) - i64::from(pad),
    );
    crop(&closed, left, top, page_width, page_height)
}

/// The block's crop of the page, and its ink: the side of one Otsu split of
/// the crop that isn't the paper. The paper is what the crop shows around the
/// block, or with none around, the side covering more of the block, so light
/// lettering on dark counts too. `None` when the block holds no ink.
fn ink(block: &Mask, page: &RgbImage) -> Option<(RgbImage, GrayImage)> {
    let (width, height) = (block.width, block.height);
    if width == 0 || height == 0 {
        return None;
    }
    let crop = imageops::crop_imm(page, block.x, block.y, width, height).to_image();
    let inside = |x: u32, y: u32| block.pixels[(y * width + x) as usize] != 0;
    // One tile covers the crop, so this is a single Otsu split.
    let gray = imageops::grayscale(&crop);
    let dark = otsu_adaptive_threshold(&gray, width.max(16), height.max(16), 0, 0, 0.1).ok()?;
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
    let ink = GrayImage::from_fn(width, height, |x, y| {
        let is_dark = dark.get_pixel(x, y).0[0] != 0;
        Luma([if inside(x, y) && is_dark != light_ink {
            255
        } else {
            0
        }])
    });
    ink.pixels()
        .any(|pixel| pixel.0[0] != 0)
        .then_some((crop, ink))
}

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
