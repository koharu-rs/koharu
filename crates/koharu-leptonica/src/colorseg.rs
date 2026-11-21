//! Leptonica's `pixColorSegment`:
//! <https://github.com/DanBloomberg/leptonica/blob/13275a2/src/colorseg.c>
//! with the octcube tables of src/colorquant1.c (`makeRGBToIndexTables`,
//! `getRGBFromOctcube`, `pixcmapToOctcubeLUT`, `pixRemoveUnusedColors`),
//! `pixcmapGetIndex` and `pixcmapGetNearestIndex` from src/colormap.c,
//! `numaGetSortIndex` from src/numafunc1.c, and `pixCloseSafeCompBrick` with
//! `selectComposableSizes` from src/morph.c.

use image::{GrayImage, Luma, RgbImage};

use crate::{Binary, Error, Result};

/// `MAX_ALLOWED_ITERATIONS`.
const MAX_ITERATIONS: u32 = 20;
/// `DIST_EXPAND_FACT`.
const DISTANCE_EXPANSION: f32 = 1.3;
/// A colormap at 8 bpp holds this many colours.
const COLORMAP_SIZE: usize = 256;

/// An image reduced to a few colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segments {
    /// Each pixel's index into `colors`.
    pub indices: GrayImage,
    pub colors: Vec<[u8; 3]>,
}

/// `pixColorSegment`. Pixels join the first colour within `max_distance`,
/// up to `max_colors` colours, the distance growing 1.3× until they fit;
/// each pixel then takes its octcube's nearest colour; each colour, most
/// populous first, absorbs what a `sel_size` closing of its pixels adds;
/// and colours past the `final_colors` most populous give their pixels to
/// the nearest that remain. `None` when 20 distances don't fit the colours.
pub fn color_segment(
    image: &RgbImage,
    max_distance: u32,
    max_colors: u32,
    sel_size: u32,
    final_colors: u32,
) -> Result<Option<Segments>> {
    // Below 32, Leptonica's closing border is wide enough that its closing
    // equals `close`'s.
    if final_colors == 0 || !(1..32).contains(&sel_size) {
        return Err(Error::Invalid("the colour segmentation is out of range"));
    }
    let (Ok(max_distance), Ok(max_colors)) =
        (i32::try_from(max_distance), i32::try_from(max_colors))
    else {
        return Err(Error::Invalid("the colour segmentation is out of range"));
    };
    let Some(mut colors) = cluster(image, max_distance, max_colors) else {
        return Ok(None);
    };
    let pixels: Vec<[u8; 3]> = image.pixels().map(|pixel| pixel.0).collect();
    let (width, height) = (image.width() as usize, image.height() as usize);

    let table = octcube_table(&colors);
    let mut counts = [0u32; COLORMAP_SIZE];
    let mut indices: Vec<u8> = pixels
        .iter()
        .map(|&rgb| {
            let index = table[octcube(rgb)];
            counts[index as usize] += 1;
            index
        })
        .collect();

    if sel_size > 1 {
        let counts: Vec<f32> = counts[..colors.len()]
            .iter()
            .map(|&count| count as f32)
            .collect();
        for value in sort_decreasing(&counts) {
            let mut mask = Binary::new(width, height);
            for (pixel, &index) in mask.pixels.iter_mut().zip(&indices) {
                *pixel = u8::from(usize::from(index) == value);
            }
            let closed = close(&mask, sel_size as i32);
            let index = exact_index(&colors, colors[value]);
            for ((index_at, &added), &was) in
                indices.iter_mut().zip(&closed.pixels).zip(&mask.pixels)
            {
                if added != was {
                    *index_at = index;
                }
            }
        }
    }

    let final_colors = final_colors as usize;
    if final_colors < colors.len() {
        let mut histogram = [0.0f32; COLORMAP_SIZE];
        for &index in &indices {
            histogram[usize::from(index)] += 1.0;
        }
        let order = sort_decreasing(&histogram);
        let mut removed = [false; COLORMAP_SIZE];
        for &index in &order[final_colors..colors.len()] {
            removed[index] = true;
        }
        let masked: Vec<bool> = indices
            .iter()
            .map(|&index| removed[usize::from(index)])
            .collect();
        // Leptonica reads the last kept colour even when nothing is removed,
        // past the colormap if fewer colours hold pixels; it's only used
        // when a removed colour holds pixels, which then outnumber nothing.
        if masked.contains(&true) {
            let kept = exact_index(&colors, colors[order[final_colors - 1]]);
            for (index, &masked) in indices.iter_mut().zip(&masked) {
                if masked {
                    *index = kept;
                }
            }
        }

        // `pixRemoveUnusedColors`, keeping the colormap's order.
        let mut used = [false; COLORMAP_SIZE];
        for &index in &indices {
            used[usize::from(index)] = true;
        }
        let mut renumbered = [0u8; COLORMAP_SIZE];
        let mut remaining = Vec::new();
        for (index, &color) in colors.iter().enumerate() {
            if used[index] {
                renumbered[index] = remaining.len() as u8;
                remaining.push(color);
            }
        }
        if remaining.len() < colors.len() {
            for index in &mut indices {
                *index = renumbered[usize::from(*index)];
            }
            colors = remaining;
        }

        let table = octcube_table(&colors);
        for ((index, &masked), &rgb) in indices.iter_mut().zip(&masked).zip(&pixels) {
            if masked {
                *index = table[octcube(rgb)];
            }
        }
    }

    Ok(Some(Segments {
        indices: GrayImage::from_fn(width as u32, height as u32, |x, y| {
            Luma([indices[y as usize * width + x as usize]])
        }),
        colors,
    }))
}

/// `pixColorSegmentCluster`: the colours, as their members' integer means.
fn cluster(image: &RgbImage, max_distance: i32, max_colors: i32) -> Option<Vec<[u8; 3]>> {
    let mut distance = max_distance;
    for iteration in 1..=MAX_ITERATIONS {
        if let Some(colors) = try_cluster(image, distance, max_colors) {
            return Some(colors);
        }
        if iteration == MAX_ITERATIONS {
            break;
        }
        distance = (DISTANCE_EXPANSION * distance as f32) as i32;
    }
    None
}

/// `pixColorSegmentTryCluster`: greedy, in raster order, against each
/// colour's first pixel.
fn try_cluster(image: &RgbImage, max_distance: i32, max_colors: i32) -> Option<Vec<[u8; 3]>> {
    let limit = COLORMAP_SIZE.min(max_colors.max(0) as usize);
    let max_distance2 = max_distance.wrapping_mul(max_distance);
    let mut seeds: Vec<[i32; 3]> = Vec::new();
    let mut sums: Vec<[i32; 3]> = Vec::new();
    let mut counts: Vec<i32> = Vec::new();
    for pixel in image.pixels() {
        let rgb = pixel.0.map(i32::from);
        let found = seeds.iter().position(|seed| {
            (0..3)
                .map(|channel| (rgb[channel] - seed[channel]).pow(2))
                .sum::<i32>()
                <= max_distance2
        });
        match found {
            Some(index) => {
                counts[index] += 1;
                for channel in 0..3 {
                    sums[index][channel] += rgb[channel];
                }
            }
            None if seeds.len() < limit => {
                seeds.push(rgb);
                sums.push(rgb);
                counts.push(1);
            }
            None => return None,
        }
    }
    Some(
        sums.iter()
            .zip(&counts)
            .map(|(sum, &count)| sum.map(|sum| (sum / count) as u8))
            .collect(),
    )
}

/// The level 4 octcube of a colour: the top 4 bits of each channel,
/// interleaved red, green, blue from the most significant.
fn octcube([r, g, b]: [u8; 3]) -> usize {
    (0..4).fold(0, |cube, bit| {
        let shift = 7 - bit;
        let channel = |value: u8| usize::from(value >> shift & 1);
        cube << 3 | channel(r) << 2 | channel(g) << 1 | channel(b)
    })
}

/// `pixcmapToOctcubeLUT` at level 4 by Manhattan distance: each octcube's
/// nearest colour to its centre, with black and white taken by colours
/// close enough to them.
fn octcube_table(colors: &[[u8; 3]]) -> Vec<u8> {
    let mut table: Vec<u8> = (0..1 << 12)
        .map(|cube: usize| {
            let center = [2, 1, 0].map(|offset| {
                (0..4).fold(0i32, |value, bit| {
                    value << 1 | (cube >> (3 * (3 - bit) + offset) & 1) as i32
                }) << 4
                    | 8
            });
            let mut nearest = (1_000_000, 0);
            for (index, color) in colors.iter().enumerate() {
                let distance: i32 = (0..3)
                    .map(|channel| (center[channel] - i32::from(color[channel])).abs())
                    .sum();
                if distance < nearest.0 {
                    nearest = (distance, index);
                }
            }
            nearest.1 as u8
        })
        .collect();
    let black = nearest_index(colors, [0, 0, 0]);
    if colors[black].iter().all(|&channel| channel < 7) {
        table[0] = black as u8;
    }
    let white = nearest_index(colors, [255, 255, 255]);
    if colors[white].iter().all(|&channel| channel > 248) {
        table[(1 << 12) - 1] = white as u8;
    }
    table
}

/// `pixcmapGetNearestIndex`: the first colour nearest by Euclidean distance.
fn nearest_index(colors: &[[u8; 3]], rgb: [i32; 3]) -> usize {
    let (mut nearest, mut min) = (0, 3 * 255 * 255 + 1);
    for (index, color) in colors.iter().enumerate() {
        let distance: i32 = (0..3)
            .map(|channel| (i32::from(color[channel]) - rgb[channel]).pow(2))
            .sum();
        if distance < min {
            nearest = index;
            if distance == 0 {
                break;
            }
            min = distance;
        }
    }
    nearest
}

/// `pixcmapGetIndex`: where `pixSetMasked` finds a colour, the first entry
/// equal to it.
fn exact_index(colors: &[[u8; 3]], color: [u8; 3]) -> u8 {
    colors
        .iter()
        .position(|&entry| entry == color)
        .expect("the colour is in the colormap") as u8
}

/// `numaGetSortIndex`, decreasing: Leptonica's shell sort, which doesn't
/// stop an insertion early, so ties land where Leptonica puts them.
fn sort_decreasing(values: &[f32]) -> Vec<usize> {
    let mut values = values.to_vec();
    let mut order: Vec<usize> = (0..values.len()).collect();
    let mut gap = values.len() / 2;
    while gap > 0 {
        for i in gap..values.len() {
            let mut j = i as isize - gap as isize;
            while j >= 0 {
                let at = j as usize;
                if values[at] < values[at + gap] {
                    values.swap(at, at + gap);
                    order.swap(at, at + gap);
                }
                j -= gap as isize;
            }
        }
        gap /= 2;
    }
    order
}

/// `pixCloseSafeCompBrick` with a square `size`: the composite of a brick
/// and a comb, which is a brick of the composable size, on a border wide
/// enough that the closing loses nothing at the image's edges.
fn close(mask: &Binary, size: i32) -> Binary {
    let (first, second) = composable_sizes(size);
    let size = (first * second) as usize;
    // Leptonica's sels put the origin at `size / 2`, so the dilation reaches
    // `before` back and `after` forward, and the erosion the reverse.
    let (after, before) = (size / 2, size - 1 - size / 2);
    let border = after.max(before);
    let (width, height) = (mask.width + 2 * border, mask.height + 2 * border);
    let mut canvas = vec![0u8; width * height];
    for y in 0..mask.height {
        let row = (y + border) * width + border;
        canvas[row..row + mask.width]
            .copy_from_slice(&mask.pixels[y * mask.width..(y + 1) * mask.width]);
    }
    let canvas = pass(&canvas, width, height, true, before, after, true);
    let canvas = pass(&canvas, width, height, false, before, after, true);
    let canvas = pass(&canvas, width, height, true, after, before, false);
    let canvas = pass(&canvas, width, height, false, after, before, false);
    let mut closed = Binary::new(mask.width, mask.height);
    for y in 0..mask.height {
        let row = (y + border) * width + border;
        closed.pixels[y * mask.width..(y + 1) * mask.width]
            .copy_from_slice(&canvas[row..row + mask.width]);
    }
    closed
}

/// A 1-D dilation (any on) or erosion (all on) over `[at - back, at +
/// forward]`, along rows or columns, with everything past the canvas off.
fn pass(
    canvas: &[u8],
    width: usize,
    height: usize,
    horizontal: bool,
    back: usize,
    forward: usize,
    dilate: bool,
) -> Vec<u8> {
    let mut out = vec![0u8; canvas.len()];
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    for line in 0..lines {
        let at = |position: usize| {
            let index = if horizontal {
                line * width + position
            } else {
                position * width + line
            };
            canvas[index]
        };
        for position in 0..length {
            let window = position.saturating_sub(back)..=(position + forward).min(length - 1);
            let inside = position >= back && position + forward < length;
            let value = if dilate {
                window.into_iter().any(|position| at(position) != 0)
            } else {
                inside && window.into_iter().all(|position| at(position) != 0)
            };
            let index = if horizontal {
                line * width + position
            } else {
                position * width + line
            };
            out[index] = u8::from(value);
        }
    }
    out
}

/// `selectComposableSizes`: two factors whose product is `size`, or near it,
/// that keep the number of rasterops low.
fn composable_sizes(size: i32) -> (i32, i32) {
    const ACCEPTABLE_COST: i32 = 5;
    let mid = ((f64::from(size)).sqrt() + 0.001) as i32;
    if mid * mid == size {
        return (mid, mid);
    }
    let options: Vec<(i32, i32, i32, i32)> = (1..=mid + 1)
        .rev()
        .map(|first| {
            let below = size / first;
            let above = below + 1;
            let (cost_below, cost_above) = (first + below - 2 * mid, first + above - 2 * mid);
            let (difference_below, difference_above) =
                ((size - first * below).abs(), (size - first * above).abs());
            if difference_below <= difference_above {
                (
                    first.min(below),
                    first.max(below),
                    cost_below,
                    difference_below,
                )
            } else {
                (
                    first.min(above),
                    first.max(above),
                    cost_above,
                    difference_above,
                )
            }
        })
        .collect();
    let (mut min_cost, mut chosen) = (10000, 1);
    for (index, &(low, high, cost, difference)) in options.iter().enumerate() {
        if difference == 0 && cost < ACCEPTABLE_COST {
            return (high, low);
        }
        let total = 4 * difference + cost;
        if total < min_cost {
            min_cost = total;
            chosen = index;
        }
    }
    (options[chosen].1, options[chosen].0)
}
