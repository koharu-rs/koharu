//! Leptonica's `pixFindStrokeWidth`:
//! <https://github.com/DanBloomberg/leptonica/blob/13275a2/src/strokes.c>
//! with `pixExtractBoundary` from src/morphapp.c, the asymmetric brick
//! erosion of src/morph.c, and `distanceFunctionLow` from src/seedfill.c.

use image::GrayImage;

use crate::Binary;

/// The stroke width of `image`'s foreground: the average of its area over
/// half its boundary, and of a width read from its 8-connected distance
/// histogram, where distances holding less than `threshold` (about 0.15)
/// of the pixels at distance 1 count as noise. `None` without foreground.
pub fn find_stroke_width(image: &GrayImage, threshold: f32) -> Option<f32> {
    let image = Binary::from_image(image);
    let count = image.count();
    // Leptonica reads an unset ratio without foreground, and divides by a
    // zero boundary length for a lone pixel.
    if count == 0 {
        return None;
    }
    let length = (count - eroded(&image).count()) / 2;
    let area_width = count as f32 / length as f32;

    let mut histogram = [0.0f32; 256];
    for distance in distances(&image) {
        histogram[distance as usize] += 1.0;
    }
    let last = histogram.iter().rposition(|&count| count > 0.1)?;
    let histogram = &histogram[..=last];
    let n = histogram.len();
    let (mut index, mut ratio) = (n - 1, 0.0);
    while index > 0 {
        ratio = histogram[index] / histogram[1];
        if ratio > threshold {
            break;
        }
        index -= 1;
    }
    let extra = if index < n - 1 {
        histogram[index + 1] / histogram[1]
    } else {
        0.0
    };
    let distance_width = 2.0 * (index as f32 - 1.0 + ratio + extra);
    let width = (area_width + distance_width) / 2.0;
    width.is_finite().then_some(width)
}

/// A 3 × 3 brick erosion with everything outside the image off, so the
/// image's edge pixels always erode. The boundary `pixExtractBoundary` takes
/// is the image less this, which is all `find_stroke_width` counts.
fn eroded(image: &Binary) -> Binary {
    let mut eroded = Binary::new(image.width, image.height);
    for y in 1..image.height.saturating_sub(1) {
        for x in 1..image.width.saturating_sub(1) {
            let on = (y - 1..=y + 1).all(|y| (x - 1..=x + 1).all(|x| image.get(x, y) != 0));
            eroded.pixels[y * image.width + x] = u8::from(on);
        }
    }
    eroded
}

/// `pixDistanceFunction` at 8 bpp, 8-connected, with the background beyond
/// the edges: two raster scans that leave the 1-pixel edge as initialized.
fn distances(image: &Binary) -> Vec<u8> {
    let (width, height) = (image.width, image.height);
    let mut distances = image.pixels.clone();
    let at = |y: usize, x: usize| y * width + x;
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            if distances[at(y, x)] > 0 {
                let min = distances[at(y - 1, x - 1)]
                    .min(distances[at(y - 1, x)])
                    .min(distances[at(y - 1, x + 1)])
                    .min(distances[at(y, x - 1)])
                    .min(254);
                distances[at(y, x)] = min + 1;
            }
        }
    }
    for y in (1..height.saturating_sub(1)).rev() {
        for x in (1..width.saturating_sub(1)).rev() {
            let value = distances[at(y, x)];
            if value > 0 {
                let min = distances[at(y + 1, x + 1)]
                    .min(distances[at(y + 1, x)])
                    .min(distances[at(y + 1, x - 1)])
                    .min(distances[at(y, x + 1)]);
                distances[at(y, x)] = min.saturating_add(1).min(value);
            }
        }
    }
    distances
}
