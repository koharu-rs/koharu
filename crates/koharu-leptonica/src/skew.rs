//! Leptonica's skew finder, `pixFindSkew` and
//! `pixFindSkewSweepAndSearchScorePivot` about the corner:
//! <https://github.com/DanBloomberg/leptonica/blob/13275a2/src/skew.c>
//! with `pixVShear` from src/shear.c and `pixReduceRankBinary2` from
//! src/binreduce.c. A shear only moves pixels between rows, so the port sums
//! each sheared row straight from the source instead of making the image.

use image::GrayImage;

use crate::{Binary, Error, Result};

const DEG2RAD: f32 = std::f32::consts::PI / 180.0;
/// `MinValidMaxscore`.
const MIN_VALID_MAX_SCORE: f32 = 10000.0;
/// `MinscoreThreshFactor`.
const MIN_SCORE_THRESHOLD_FACTOR: f32 = 0.000_002;

/// A text skew, as Leptonica measures it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skew {
    /// The rotation that deskews the image, in degrees, clockwise positive.
    pub angle: f32,
    /// The best score over the worst; zero when the measurement isn't valid.
    pub confidence: f32,
}

/// `pixFindSkew`: a ±7° sweep in 1° steps at 4× reduction, then a binary
/// search down to 0.01° at 2×. `None` when no foreground survives the
/// reduction.
pub fn find_skew(image: &GrayImage) -> Result<Option<Skew>> {
    find_skew_sweep_and_search(image, 4, 2, 7.0, 1.0, 0.01)
}

/// `pixFindSkewSweepAndSearch`: a sweep over ±`sweep_range` degrees in
/// `sweep_delta` steps at `sweep_reduction`, then a binary search at
/// `search_reduction` until the step falls below `min_bs_delta`.
/// Reductions are 1, 2, 4, or 8.
pub fn find_skew_sweep_and_search(
    image: &GrayImage,
    sweep_reduction: u32,
    search_reduction: u32,
    sweep_range: f32,
    sweep_delta: f32,
    min_bs_delta: f32,
) -> Result<Option<Skew>> {
    for reduction in [sweep_reduction, search_reduction] {
        if ![1, 2, 4, 8].contains(&reduction) {
            return Err(Error::Invalid("a skew reduction isn't 1, 2, 4, or 8"));
        }
    }
    if search_reduction > sweep_reduction {
        return Err(Error::Invalid("the search reduction exceeds the sweep's"));
    }
    // Leptonica doesn't check these. Its shear never finishes at 45° or more,
    // and a search step that isn't positive never ends.
    if !(sweep_delta > 0.0 && min_bs_delta > 0.0 && sweep_range > 0.0)
        || sweep_range + sweep_delta >= 45.0
    {
        return Err(Error::Invalid("the skew sweep is out of range"));
    }

    let image = Binary::from_image(image);
    let search = reduce_cascade(
        &image,
        match search_reduction {
            1 => &[],
            2 => &[1],
            4 => &[1, 1],
            _ => &[1, 1, 2],
        },
    )?;
    if search.count() == 0 {
        return Ok(None);
    }
    let sweep = reduce_cascade(
        &search,
        match sweep_reduction / search_reduction {
            1 => &[],
            2 => &[1],
            4 => &[1, 2],
            _ => &[1, 2, 2],
        },
    )?;

    let angles = ((2.0 * f64::from(sweep_range)) / f64::from(sweep_delta) + 1.0) as i32;
    let range_left = 0.0 - sweep_range;
    let (mut max_score, mut max_index) = (-1_000_000_000.0, 0);
    for index in 0..angles {
        let theta = range_left + index as f32 * sweep_delta;
        let score = score(&sweep, DEG2RAD * theta);
        if score > max_score {
            max_score = score;
            max_index = index;
        }
    }
    if max_index == 0 || max_index == angles - 1 {
        // Leptonica warns that the maximum is at the sweep's edge.
        return Ok(Some(Skew {
            angle: 0.0,
            confidence: 0.0,
        }));
    }

    let mut center = range_left + max_index as f32 * sweep_delta;
    let mut scores = [
        score(&search, DEG2RAD * (center - sweep_delta)),
        0.0,
        score(&search, DEG2RAD * center),
        0.0,
        score(&search, DEG2RAD * (center + sweep_delta)),
    ];
    let mut all = vec![scores[2], scores[0], scores[4]];
    let mut delta = 0.5 * sweep_delta;
    while delta >= min_bs_delta {
        scores[1] = score(&search, DEG2RAD * (center - delta));
        all.push(scores[1]);
        scores[3] = score(&search, DEG2RAD * (center + delta));
        all.push(scores[3]);
        max_score = scores[1];
        max_index = 1;
        for index in 2..4 {
            if scores[index as usize] > max_score {
                max_score = scores[index as usize];
                max_index = index;
            }
        }
        let (left, right) = (
            scores[max_index as usize - 1],
            scores[max_index as usize + 1],
        );
        scores = [left, 0.0, max_score, 0.0, right];
        center += delta * (max_index - 2) as f32;
        delta *= 0.5;
    }

    let min_score =
        all.iter().copied().fold(
            1_000_000_000.0,
            |min, score| {
                if score < min { score } else { min }
            },
        );
    let threshold = MIN_SCORE_THRESHOLD_FACTOR
        * search.width as f32
        * search.width as f32
        * search.height as f32;
    let mut confidence = if min_score > threshold {
        max_score / min_score
    } else {
        0.0
    };
    if center > range_left + 2.0 * sweep_range - sweep_delta
        || center < range_left + sweep_delta
        || max_score < MIN_VALID_MAX_SCORE
    {
        confidence = 0.0;
    }
    Ok(Some(Skew {
        angle: center,
        confidence,
    }))
}

/// `pixFindDifferentialSquareSum` of `image` sheared about its top-left
/// corner by `radians` (`pixVShearCorner`, white brought in).
fn score(image: &Binary, radians: f32) -> f32 {
    let (width, height) = (image.width as i32, image.height as i32);
    let mut rows = vec![0u32; image.height];
    let mut add = |columns: std::ops::Range<i32>, shift: i32| {
        for x in columns.start.max(0)..columns.end.min(width) {
            for y in 0..height {
                let to = y + shift;
                if (0..height).contains(&to) && image.get(x as usize, y as usize) != 0 {
                    rows[to as usize] += 1;
                }
            }
        }
    };
    if radians == 0.0 || f64::from(radians).tan() == 0.0 {
        add(0..width, 0);
    } else {
        let sign = if radians < 0.0 { -1 } else { 1 };
        let tangent = f64::from(radians).tan() as f32;
        let inverse = (1.0 / f64::from(tangent)).abs() as f32;
        let first = (f64::from(inverse) / 2.0) as i32;
        add(-first..first, 0);
        let (mut x, mut shift) = (first, 1);
        while x < width {
            let mut increment = (f64::from(inverse) * (f64::from(shift) + 0.5) + 0.5) as i32 - x;
            increment = increment.min(width - x);
            add(x..x + increment, sign * shift);
            x += increment;
            shift += 1;
        }
    }

    let skip = (height / 10).min((0.05 * f64::from(width)) as i32);
    let edge = (skip / 2).max(1);
    let mut sum = 0.0f32;
    for row in edge..height - edge {
        let difference = rows[row as usize] as f32 - rows[row as usize - 1] as f32;
        sum += difference * difference;
    }
    sum
}

/// `pixReduceRankBinaryCascade`: one 2× reduction for each level, where a
/// pixel is on when at least `level` of its 2 × 2 sources are.
fn reduce_cascade(image: &Binary, levels: &[u8]) -> Result<Binary> {
    let mut reduced = image.clone();
    for &level in levels {
        if reduced.width < 2 || reduced.height < 2 {
            return Err(Error::Invalid("the image is too small to reduce"));
        }
        let mut next = Binary::new(reduced.width / 2, reduced.height / 2);
        for y in 0..next.height {
            for x in 0..next.width {
                let on = reduced.get(2 * x, 2 * y)
                    + reduced.get(2 * x + 1, 2 * y)
                    + reduced.get(2 * x, 2 * y + 1)
                    + reduced.get(2 * x + 1, 2 * y + 1);
                next.pixels[y * next.width + x] = u8::from(on >= level);
            }
        }
        reduced = next;
    }
    Ok(reduced)
}
