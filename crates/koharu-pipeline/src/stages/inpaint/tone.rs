//! An edit's colours matched to the window it redrew. An edit model redraws
//! the whole window and drifts its tone, so the redraw is mapped onto the
//! original where both should agree, away from the mask: one colour transform
//! for the window, then the original's local differences from it, spread
//! across the mask. Without it a fill pasted under a mask showed its seam.

use image::{GrayImage, ImageBuffer, Luma, Rgb, RgbImage, imageops};
use imageproc::{distance_transform::Norm, morphology::dilate};

/// How far from the mask a pixel must be to count as one the edit should
/// have kept.
const CLEAR: u8 = 8;
/// How far a kept pixel's channels may drift. Past it the edit changed what's
/// drawn there, as it erases lettering the mask leaves out, and matching that
/// tinted the window and smeared the fill.
const DRIFT: u8 = 32;
/// How far the local differences spread, as a Gaussian's sigma.
const SPREAD: f32 = 24.0;

type Field = ImageBuffer<Luma<f32>, Vec<f32>>;

/// `edited` with its colours matched to `original` outside `mask`.
pub(super) fn matched(original: &RgbImage, edited: &RgbImage, mask: &GrayImage) -> RgbImage {
    let near = dilate(mask, Norm::LInf, CLEAR);
    let kept = |x: u32, y: u32| {
        let (original, edited) = (original.get_pixel(x, y).0, edited.get_pixel(x, y).0);
        near.get_pixel(x, y).0[0] == 0
            && (0..3).all(|channel| original[channel].abs_diff(edited[channel]) <= DRIFT)
    };
    if !edited.enumerate_pixels().any(|(x, y, _)| kept(x, y)) {
        return edited.clone();
    }
    let transform = fit(original, edited, &kept);
    let global = RgbImage::from_fn(edited.width(), edited.height(), |x, y| {
        let input = features(edited.get_pixel(x, y));
        Rgb(std::array::from_fn(|channel| {
            byte((0..4).map(|row| input[row] * transform[row][channel]).sum())
        }))
    });
    let (width, height) = edited.dimensions();
    let weight = imageops::blur(
        &Field::from_fn(width, height, |x, y| {
            Luma([if kept(x, y) { 1.0 } else { 0.0 }])
        }),
        SPREAD,
    );
    let residuals: [Field; 3] = std::array::from_fn(|channel| {
        imageops::blur(
            &Field::from_fn(width, height, |x, y| {
                Luma([if kept(x, y) {
                    f32::from(original.get_pixel(x, y).0[channel])
                        - f32::from(global.get_pixel(x, y).0[channel])
                } else {
                    0.0
                }])
            }),
            SPREAD,
        )
    });
    RgbImage::from_fn(width, height, |x, y| {
        let weight = weight.get_pixel(x, y).0[0].max(1e-3);
        let pixel = global.get_pixel(x, y).0;
        Rgb(std::array::from_fn(|channel| {
            byte(f64::from(
                f32::from(pixel[channel]) + residuals[channel].get_pixel(x, y).0[0] / weight,
            ))
        }))
    })
}

/// The affine colour transform taking `edited` to `original` on the pixels
/// `kept`, by least squares, pulled slightly toward leaving colours as they
/// are, so a window whose kept pixels are one flat colour still maps sanely.
fn fit(original: &RgbImage, edited: &RgbImage, kept: &impl Fn(u32, u32) -> bool) -> [[f64; 3]; 4] {
    let mut normal = [[0.0f64; 4]; 4];
    let mut target = [[0.0f64; 3]; 4];
    for (x, y, pixel) in edited.enumerate_pixels() {
        if !kept(x, y) {
            continue;
        }
        let input = features(pixel);
        let output = original.get_pixel(x, y).0;
        for row in 0..4 {
            for column in 0..4 {
                normal[row][column] += input[row] * input[column];
            }
            for channel in 0..3 {
                target[row][channel] += input[row] * f64::from(output[channel]);
            }
        }
    }
    let pull = 1e-4 * (0..4).map(|row| normal[row][row]).sum::<f64>() / 4.0;
    for row in 0..4 {
        normal[row][row] += pull;
        if row < 3 {
            target[row][row] += pull;
        }
    }
    solve(normal, target)
}

/// A pixel's channels and a constant, for the transform's offset.
fn features(pixel: &Rgb<u8>) -> [f64; 4] {
    let [red, green, blue] = pixel.0;
    [f64::from(red), f64::from(green), f64::from(blue), 1.0]
}

/// `a x = b` by Gauss-Jordan elimination with partial pivoting; `a` is
/// symmetric positive definite once pulled toward the identity.
fn solve(mut a: [[f64; 4]; 4], mut b: [[f64; 3]; 4]) -> [[f64; 3]; 4] {
    for column in 0..4 {
        let pivot = (column..4)
            .max_by(|&i, &j| a[i][column].abs().total_cmp(&a[j][column].abs()))
            .unwrap_or(column);
        a.swap(column, pivot);
        b.swap(column, pivot);
        let (pivot_a, pivot_b) = (a[column], b[column]);
        let lead = pivot_a[column];
        for (row, (a_row, b_row)) in a.iter_mut().zip(&mut b).enumerate() {
            if row == column {
                continue;
            }
            let factor = a_row[column] / lead;
            for (value, pivot) in a_row.iter_mut().zip(pivot_a) {
                *value -= factor * pivot;
            }
            for (value, pivot) in b_row.iter_mut().zip(pivot_b) {
                *value -= factor * pivot;
            }
        }
    }
    std::array::from_fn(|row| std::array::from_fn(|channel| b[row][channel] / a[row][row]))
}

fn byte(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An edit that darkened one side of the window and brightened the other,
    /// which one colour transform can't undo, comes back to the original's
    /// colours under a mask on each side, and a black bar it erased by the
    /// left one leaves no smear there.
    #[test]
    fn a_tone_shift_that_varies_is_undone_under_the_mask() {
        let paper = |y: u32| 64 + 2 * y as u8;
        let original = RgbImage::from_fn(256, 64, |x, y| {
            Rgb([if (52..60).contains(&x) { 0 } else { paper(y) }; 3])
        });
        let edited = RgbImage::from_fn(256, 64, |x, y| {
            let shade = if x < 128 {
                paper(y) - 20
            } else {
                paper(y) + 20
            };
            Rgb([shade; 3])
        });
        let mask = GrayImage::from_fn(256, 64, |x, y| {
            Luma([
                if ((24..40).contains(&x) || (216..232).contains(&x)) && (24..40).contains(&y) {
                    255
                } else {
                    0
                },
            ])
        });
        let matched = matched(&original, &edited, &mask);
        for x in [32, 224] {
            let got = matched.get_pixel(x, 32).0;
            assert!(
                got.iter().all(|got| got.abs_diff(paper(32)) <= 2),
                "{x}: {got:?}"
            );
        }
    }

    /// An edit that brightened the whole window comes back to the original's
    /// colours, inside the mask as well as out.
    #[test]
    fn a_tone_shift_is_undone_under_the_mask() {
        let original = RgbImage::from_fn(64, 64, |x, y| Rgb([(x * 3) as u8, (y * 3) as u8, 90]));
        let edited = RgbImage::from_fn(64, 64, |x, y| {
            let pixel = original.get_pixel(x, y).0;
            Rgb(pixel.map(|channel| channel.saturating_add(20)))
        });
        let mask = GrayImage::from_fn(64, 64, |x, y| {
            Luma([if (24..40).contains(&x) && (24..40).contains(&y) {
                255
            } else {
                0
            }])
        });
        let matched = matched(&original, &edited, &mask);
        for (x, y) in [(32, 32), (5, 5)] {
            let (got, want) = (matched.get_pixel(x, y).0, original.get_pixel(x, y).0);
            for channel in 0..3 {
                assert!(
                    got[channel].abs_diff(want[channel]) <= 2,
                    "{x},{y}: {got:?} vs {want:?}"
                );
            }
        }
    }
}
