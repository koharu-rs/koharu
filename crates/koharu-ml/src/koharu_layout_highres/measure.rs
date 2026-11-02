//! The typography a detection shows, on the page under its mask: the lines'
//! direction and tilt from the mask's shape, and the ink, its stroke width,
//! and its colours from Leptonica's algorithms. RF-DETR's postprocessing
//! stays as upstream has it; this runs after it, when a caller asks.

use image::{GrayImage, Luma, RgbImage, imageops};
use imageproc::{
    filter::gaussian_blur_f32,
    gradients::{horizontal_sobel, vertical_sobel},
    region_labelling::{Connectivity, connected_components},
};
use koharu_leptonica::{color_segment, find_stroke_width, otsu_adaptive_threshold};
use serde::Serialize;

use super::Detection;

/// What a detection's lettering shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Typography {
    /// Whether the lines run down the page: the mask is longer that way, at
    /// its tilt.
    pub vertical: bool,
    /// How far the lines turn clockwise from horizontal, or from vertical for
    /// vertical lines, in degrees within ±45°: the commonest direction of the
    /// mask's edges. A turn that moves the lines' far end by less than a
    /// stroke is 0: it can't be told from the glyphs' ragged edges.
    pub tilt: f32,
    pub stroke_width: Option<f32>,
    /// The commonest colour segment under the ink, and around it.
    pub ink: Option<[u8; 3]>,
    pub paper: Option<[u8; 3]>,
}

impl Detection {
    /// The detection's typography on `page`, or `None` when its mask holds
    /// no ink. The ink is the side of an Otsu split of the mask's crop that
    /// covers less of the mask, so light lettering on dark counts too.
    pub fn typography(&self, page: &RgbImage) -> Option<Typography> {
        let mask = &self.mask;
        let (width, height) = (
            mask.width.min(page.width().saturating_sub(mask.x)),
            mask.height.min(page.height().saturating_sub(mask.y)),
        );
        if width == 0 || height == 0 {
            return None;
        }
        let crop = imageops::crop_imm(page, mask.x, mask.y, width, height).to_image();
        let inside = |x: u32, y: u32| mask.pixels[(y * mask.width + x) as usize] != 0;

        // One tile covers the crop, so this is a single Otsu split.
        let gray = imageops::grayscale(&crop);
        let dark = otsu_adaptive_threshold(&gray, width.max(16), height.max(16), 0, 0, 0.1).ok()?;
        // The mask hugs the lettering, so the paper is what the crop shows
        // around it; with no paper around, the lettering is the smaller side.
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
        if ink.pixels().all(|pixel| pixel.0[0] == 0) {
            return None;
        }

        // The lettering mask is closed and hole-filled, so its edges are the
        // sides and ends of its lines, and their commonest direction, modulo
        // 90°, is the lines' tilt. The edges' gradients are averaged at four
        // times their angle, which folds sides and ends onto one direction.
        // Leptonica's skew finder, scoring the ink's profiles, flipped
        // direction on 14% of blocks turned by up to 20° and followed only
        // 82% of the turn; the mask's hull, by its minimum-area rectangle or
        // narrowest side, tilted wherever columns of uneven length step.
        // Only the mask's body counts, the parts at least a tenth the size of
        // its largest: a stray fragment of a neighbour's mask is no line.
        let shape = GrayImage::from_raw(mask.width, mask.height, mask.pixels.clone())
            .expect("a mask holds a byte a pixel");
        let parts = connected_components(&shape, Connectivity::Eight, Luma([0]));
        let mut sizes = std::collections::HashMap::<u32, u32>::new();
        for part in parts.pixels().filter(|part| part.0[0] != 0) {
            *sizes.entry(part.0[0]).or_default() += 1;
        }
        let largest = sizes.values().copied().max().unwrap_or(0);
        let body = |part: u32| part != 0 && 10 * sizes[&part] >= largest;
        // A border, so the edges at the crop's sides have gradients too.
        let border = 4;
        let body_image =
            GrayImage::from_fn(mask.width + 2 * border, mask.height + 2 * border, |x, y| {
                let inside = (border..border + mask.width).contains(&x)
                    && (border..border + mask.height).contains(&y)
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
