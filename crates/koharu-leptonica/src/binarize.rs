//! Leptonica's `pixOtsuAdaptiveThreshold` and `pixSauvolaBinarize`:
//! <https://github.com/DanBloomberg/leptonica/blob/13275a2/src/binarize.c>
//! with `numaSplitDistribution` from src/numafunc2.c, the tiling of
//! src/pixtiling.c, and `pixBlockconvGray`, `pixWindowedMean`, and
//! `pixWindowedMeanSquare` from src/convolve.c.

use image::GrayImage;

use crate::{Binary, Error, Result};

/// `pixOtsuAdaptiveThreshold`: an Otsu threshold for each tile of about
/// `tile_width` × `tile_height`, smoothed over `smooth_x` × `smooth_y`
/// tiles when both are nonzero, binarizes the tile; pixels below it become
/// foreground. `score_fraction` near 0.1 picks the threshold at the
/// histogram's minimum among splits scoring that close to the best.
pub fn otsu_adaptive_threshold(
    image: &GrayImage,
    tile_width: u32,
    tile_height: u32,
    smooth_x: u32,
    smooth_y: u32,
    score_fraction: f32,
) -> Result<GrayImage> {
    if tile_width < 16 || tile_height < 16 {
        return Err(Error::Invalid("Otsu tiles are smaller than 16 pixels"));
    }
    let (width, height) = (image.width() as usize, image.height() as usize);
    let columns = (width / tile_width as usize).max(1);
    let rows = (height / tile_height as usize).max(1);
    let smooth_x = (smooth_x as usize).min((columns - 1) / 2);
    let smooth_y = (smooth_y as usize).min((rows - 1) / 2);
    // `pixTilingCreate`: the last column and row take the remainder.
    let (tile_width, tile_height) = (width / columns, height / rows);
    let span = |index: usize, count: usize, size: usize, total: usize| {
        let start = index * size;
        start..if index == count - 1 {
            total
        } else {
            start + size
        }
    };

    let mut thresholds = vec![0u8; columns * rows];
    for row in 0..rows {
        for column in 0..columns {
            let mut histogram = [0.0f32; 256];
            for y in span(row, rows, tile_height, height) {
                for x in span(column, columns, tile_width, width) {
                    histogram[usize::from(image.get_pixel(x as u32, y as u32).0[0])] += 1.0;
                }
            }
            thresholds[row * columns + column] = split_distribution(&histogram, score_fraction);
        }
    }
    let thresholds = block_convolve(&thresholds, columns, rows, smooth_x, smooth_y);

    let mut binary = Binary::new(width, height);
    for row in 0..rows {
        for column in 0..columns {
            let threshold = thresholds[row * columns + column];
            for y in span(row, rows, tile_height, height) {
                for x in span(column, columns, tile_width, width) {
                    let below = image.get_pixel(x as u32, y as u32).0[0] < threshold;
                    binary.pixels[y * width + x] = u8::from(below);
                }
            }
        }
    }
    Ok(binary.to_image())
}

/// `numaSplitDistribution`, plus one: the split with the best Otsu score,
/// moved to the histogram's minimum among the neighbouring splits scoring
/// within `score_fraction` of it.
fn split_distribution(histogram: &[f32; 256], score_fraction: f32) -> u8 {
    let n = histogram.len();
    let sum: f32 = histogram.iter().fold(0.0, |sum, &count| sum + count);
    let norm = 4.0f32 / ((n - 1) as f32 * (n - 1) as f32);
    let (mut moment, mut total) = (0.0f32, 0.0f32);
    for (index, &count) in histogram.iter().enumerate() {
        total += count;
        moment += index as f32 * count;
    }
    let (mut mean_below, mut mean_above) = (0.0f32, moment / total);
    let (mut below, mut above) = (0.0f32, sum);
    let mut scores = [0.0f32; 256];
    let (mut max_score, mut max_index) = (0.0f32, n / 2);
    for (index, &count) in histogram.iter().enumerate() {
        let next_below = below + count;
        let next_mean_below = if next_below == 0.0 {
            mean_below
        } else {
            (below * mean_below + index as f32 * count) / next_below
        };
        let next_above = above - count;
        let next_mean_above = if next_above == 0.0 {
            mean_above
        } else {
            (above * mean_above - index as f32 * count) / next_above
        };
        let fraction = next_below / sum;
        let score = norm
            * (fraction * (1.0 - fraction))
            * (next_mean_above - next_mean_below)
            * (next_mean_above - next_mean_below);
        scores[index] = score;
        if score > max_score {
            max_score = score;
            max_index = index;
        }
        (below, above, mean_below, mean_above) =
            (next_below, next_above, next_mean_below, next_mean_above);
    }

    let min_score = (1.0 - score_fraction) * max_score;
    let mut first = max_index;
    while first > 0 && scores[first - 1] >= min_score {
        first -= 1;
    }
    let mut last = max_index;
    while last + 1 < n && scores[last + 1] >= min_score {
        last += 1;
    }
    let mut split = first;
    for index in first + 1..=last {
        if histogram[index] < histogram[split] {
            split = index;
        }
    }
    (split + 1).min(255) as u8
}

/// `pixBlockconvGray`: the mean over a `(2 × half_width + 1)` ×
/// `(2 × half_height + 1)` window, renormalized where the window leaves the
/// image. Leptonica's window reaches one pixel less up and left, and its
/// renormalization counts that.
fn block_convolve(
    pixels: &[u8],
    width: usize,
    height: usize,
    half_width: usize,
    half_height: usize,
) -> Vec<u8> {
    if half_width == 0 || half_height == 0 {
        return pixels.to_vec();
    }
    let (wc, hc) = (
        half_width.min((width - 1) / 2) as i64,
        half_height.min((height - 1) / 2) as i64,
    );
    if wc == 0 || hc == 0 {
        return pixels.to_vec();
    }
    let (w, h) = (width as i64, height as i64);
    let mut accumulator = vec![0u32; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = u32::from(pixels[y * width + x]);
            if x > 0 {
                value = value.wrapping_add(accumulator[y * width + x - 1]);
            }
            if y > 0 {
                value = value.wrapping_add(accumulator[(y - 1) * width + x]);
            }
            if x > 0 && y > 0 {
                value = value.wrapping_sub(accumulator[(y - 1) * width + x - 1]);
            }
            accumulator[y * width + x] = value;
        }
    }
    let at = |y: i64, x: i64| accumulator[y as usize * width + x as usize];

    let (fwc, fhc) = (2 * wc + 1, 2 * hc + 1);
    let norm = (1.0 / (fwc as f32 * fhc as f32) as f64) as f32;
    let mut out = vec![0u8; pixels.len()];
    for i in 0..h {
        let (min_y, max_y) = ((i - 1 - hc).max(0), (i + hc).min(h - 1));
        for j in 0..w {
            let (min_x, max_x) = ((j - 1 - wc).max(0), (j + wc).min(w - 1));
            let value = at(max_y, max_x)
                .wrapping_sub(at(max_y, min_x))
                .wrapping_add(at(min_y, min_x))
                .wrapping_sub(at(min_y, max_x));
            out[i as usize * width + j as usize] = (f64::from(norm * value as f32) + 0.5) as u8;
        }
    }

    let scale = |value: u8, factor: f32| (f32::from(value) * factor).min(255.0) as u8;
    let column_factor = |j: i64, first: bool| {
        let span = if j <= wc {
            if first { (wc + j).max(1) } else { wc + j }
        } else {
            wc + w - j
        };
        fwc as f32 / span as f32
    };
    let (w_minus, h_minus) = (w - wc, h - hc);
    for i in (0..=hc).chain(h_minus..h) {
        let first = i <= hc;
        let span = if first { (hc + i).max(1) } else { hc + h - i };
        let row_factor = fhc as f32 / span as f32;
        for j in 0..w {
            let index = i as usize * width + j as usize;
            out[index] = if j <= wc || j >= w_minus {
                let value = f32::from(out[index]) * row_factor * column_factor(j, first);
                value.min(255.0) as u8
            } else {
                scale(out[index], row_factor)
            };
        }
    }
    for i in hc + 1..h_minus {
        for j in (0..=wc).chain(w_minus..w) {
            let index = i as usize * width + j as usize;
            out[index] = scale(out[index], column_factor(j, false));
        }
    }
    out
}

/// `pixSauvolaBinarize` with a mirrored border: the threshold is
/// `mean × (1 − factor × (1 − deviation / 128))` over a window reaching
/// `half_size` pixels each way; pixels below it become foreground.
pub fn sauvola_binarize(image: &GrayImage, half_size: u32, factor: f32) -> Result<GrayImage> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let half = half_size as usize;
    if half < 2 || width < 2 * half + 3 || height < 2 * half + 3 || factor < 0.0 || factor.is_nan()
    {
        return Err(Error::Invalid("the Sauvola window doesn't fit the image"));
    }
    // `pixAddMirroredBorder`, which repeats the edge pixel.
    let border = half + 1;
    let (padded_width, padded_height) = (width + 2 * border, height + 2 * border);
    let mirror = |position: usize, size: usize| {
        let position = position as isize - border as isize;
        if position < 0 {
            (-1 - position) as usize
        } else if position >= size as isize {
            (2 * size as isize - 1 - position) as usize
        } else {
            position as usize
        }
    };
    let mut sums = vec![0u32; padded_width * padded_height];
    let mut squares = vec![0f64; padded_width * padded_height];
    for y in 0..padded_height {
        for x in 0..padded_width {
            let value = image
                .get_pixel(mirror(x, width) as u32, mirror(y, height) as u32)
                .0[0];
            let index = y * padded_width + x;
            // Both sums are of integers small enough to stay exact, so the
            // order Leptonica adds them in doesn't matter.
            let (mut sum, mut square) = (u32::from(value), f64::from(value) * f64::from(value));
            if x > 0 {
                sum = sum.wrapping_add(sums[index - 1]);
                square += squares[index - 1];
            }
            if y > 0 {
                sum = sum.wrapping_add(sums[index - padded_width]);
                square += squares[index - padded_width];
            }
            if x > 0 && y > 0 {
                sum = sum.wrapping_sub(sums[index - padded_width - 1]);
                square -= squares[index - padded_width - 1];
            }
            sums[index] = sum;
            squares[index] = square;
        }
    }

    let window = 2 * half + 1;
    let count = window as f32 * window as f32;
    let mean_norm = (1.0 / f64::from(count)) as f32;
    let square_norm = 1.0 / f64::from(count);
    let mut binary = Binary::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let (top, bottom, left, right) = (y, y + window, x, x + window);
            let at = |y: usize, x: usize| y * padded_width + x;
            let sum = sums[at(bottom, right)]
                .wrapping_sub(sums[at(bottom, left)])
                .wrapping_sub(sums[at(top, right)])
                .wrapping_add(sums[at(top, left)]);
            let mean = (mean_norm * sum as f32) as u8;
            let square =
                squares[at(bottom, right)] - squares[at(bottom, left)] - squares[at(top, right)]
                    + squares[at(top, left)];
            let mean_square = (square_norm * square + 0.5) as u32;
            let variance = mean_square as i32 - i32::from(mean) * i32::from(mean);
            let deviation = (variance as f32).sqrt();
            let threshold = (f64::from(mean)
                * (1.0 - f64::from(factor) * (1.0 - f64::from(deviation) / 128.0)))
                as i32 as u8;
            let below = image.get_pixel(x as u32, y as u32).0[0] < threshold;
            binary.pixels[y * width + x] = u8::from(below);
        }
    }
    Ok(binary.to_image())
}
