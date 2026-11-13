//! Telea's inpainting by fast marching, ported from OpenCV 4.13.0's
//! `cv::inpaint` with `INPAINT_TELEA` on 8-bit, 3-channel images:
//! <https://github.com/opencv/opencv/blob/fe38fc608f6acb8b68953438a62305d8318f4fcd/modules/photo/src/inpaint.cpp>
//!
//! It keeps OpenCV's arithmetic, its queue's order, and its quirks, such as the
//! image gradient's doubled central difference and its row offset, so it gives
//! OpenCV's result on the same input. The flags and arrival times carry
//! OpenCV's one-pixel border, and the image is read one pixel in from them.

// The file it's ported from carries this notice, which its license asks a
// redistribution of its source to keep:
//
//                        Intel License Agreement
//                For Open Source Computer Vision Library
//
// Copyright (C) 2000, Intel Corporation, all rights reserved.
// Third party copyrights are property of their respective icvers.
//
// Redistribution and use in source and binary forms, with or without modification,
// are permitted provided that the following conditions are met:
//
//   * Redistribution's of source code must retain the above copyright notice,
//     this list of conditions and the following disclaimer.
//
//   * Redistribution's in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//
//   * The name of Intel Corporation may not be used to endorse or promote products
//     derived from this software without specific prior written permission.
//
// This software is provided by the copyright holders and contributors "as is" and
// any express or implied warranties, including, but not limited to, the implied
// warranties of merchantability and fitness for a particular purpose are disclaimed.
// In no event shall the Intel Corporation or contributors be liable for any direct,
// indirect, incidental, special, exemplary, or consequential damages
// (including, but not limited to, procurement of substitute goods or services;
// loss of use, data, or profits; or business interruption) however caused
// and on any theory of liability, whether in contract, strict liability,
// or tort (including negligence or otherwise) arising in any way out of
// the use of this software, even if advised of the possibility of such damage.

use std::{cmp::Ordering, collections::BinaryHeap};

use image::{GrayImage, RgbImage};

/// Known outside the narrow band.
const KNOWN: u8 = 0;
/// The narrow band, known.
const BAND: u8 = 1;
/// Unknown.
const INSIDE: u8 = 2;
/// Known, marked while the outer distances are computed.
const CHANGE: u8 = 3;

/// `image` with the pixels `mask` marks, nonzero, filled from the known pixels
/// within `radius` of each, OpenCV's `inpaintRadius` clamped to 1..=100.
pub(super) fn inpaint(image: &RgbImage, mask: &GrayImage, radius: u32) -> RgbImage {
    let range = radius.clamp(1, 100) as isize;
    let (cols, rows) = (image.width() as usize, image.height() as usize);
    let grid = Grid {
        rows: rows + 2,
        cols: cols + 2,
    };

    // `COPY_MASK_BORDER1_C1` then `SET_BORDER1_C1(mask, 0)`.
    let mut unknown = vec![KNOWN; grid.len()];
    for (x, y, pixel) in mask.enumerate_pixels() {
        if pixel.0[0] != 0 {
            unknown[grid.at(y as usize + 1, x as usize + 1)] = INSIDE;
        }
    }
    grid.set_border(&mut unknown, 0);

    let mut flags = vec![KNOWN; grid.len()];
    let mut times = vec![1.0e6_f32; grid.len()];
    let mut band = subtract(&grid.dilate_cross(&unknown), &unknown);
    grid.set_border(&mut band, 0);
    let mut heap = Queue::default();
    heap.add(&band, grid.cols);
    for index in 0..grid.len() {
        if band[index] != 0 {
            flags[index] = BAND;
            times[index] = 0.0;
        }
        if unknown[index] != 0 {
            flags[index] = INSIDE;
        }
    }

    // The arrival times outside the band, within `range`, negated.
    let mut outside = subtract(&grid.dilate_square(&unknown, range), &unknown);
    let mut outer = Queue::default();
    outer.add(&band, grid.cols);
    outside = subtract(&outside, &band);
    grid.set_border(&mut outside, 0);
    grid.march(&mut outside, &mut times, &mut outer, true);

    let mut output = image.clone();
    grid.fill(&mut flags, &mut times, &mut output, range, &mut heap);
    output
}

/// `subtract`: saturating, as on `uchar`.
fn subtract(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().zip(b).map(|(a, b)| a.saturating_sub(*b)).collect()
}

/// A queue entry: its arrival time and insertion order, `CvHeapElem`.
#[derive(Clone, Copy, Debug)]
struct Element {
    time: f32,
    i: usize,
    j: usize,
    order: u64,
}

impl PartialEq for Element {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Element {}

impl PartialOrd for Element {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Element {
    /// Reversed, so the max-heap pops the earliest time first and, among equal
    /// times, the first pushed, as `std::greater<CvHeapElem>` orders them. The
    /// times are never NaN.
    fn cmp(&self, other: &Self) -> Ordering {
        if self.time < other.time {
            Ordering::Greater
        } else if self.time > other.time {
            Ordering::Less
        } else {
            other.order.cmp(&self.order)
        }
    }
}

/// `CvPriorityQueueFloat`.
#[derive(Default)]
struct Queue {
    heap: BinaryHeap<Element>,
    next: u64,
}

impl Queue {
    /// Every nonzero pixel, in raster order, at time 0.
    fn add(&mut self, pixels: &[u8], cols: usize) {
        for (index, &pixel) in pixels.iter().enumerate() {
            if pixel != 0 {
                self.push(index / cols, index % cols, 0.0);
            }
        }
    }

    fn push(&mut self, i: usize, j: usize, time: f32) {
        self.heap.push(Element {
            time,
            i,
            j,
            order: self.next,
        });
        self.next += 1;
    }

    fn pop(&mut self) -> Option<(usize, usize)> {
        self.heap.pop().map(|element| (element.i, element.j))
    }
}

/// The bordered grid's dimensions.
#[derive(Clone, Copy)]
struct Grid {
    rows: usize,
    cols: usize,
}

impl Grid {
    fn len(self) -> usize {
        self.rows * self.cols
    }

    fn at(self, i: usize, j: usize) -> usize {
        i * self.cols + j
    }

    /// `SET_BORDER1_C1`.
    fn set_border(self, pixels: &mut [u8], value: u8) {
        for j in 0..self.cols {
            pixels[self.at(0, j)] = value;
            pixels[self.at(self.rows - 1, j)] = value;
        }
        for i in 1..self.rows - 1 {
            pixels[self.at(i, 0)] = value;
            pixels[self.at(i, self.cols - 1)] = value;
        }
    }

    /// `cv::dilate` with a 3 × 3 cross: what lies outside adds nothing.
    fn dilate_cross(self, pixels: &[u8]) -> Vec<u8> {
        let mut output = pixels.to_vec();
        for i in 0..self.rows {
            for j in 0..self.cols {
                let mut value = pixels[self.at(i, j)];
                if i > 0 {
                    value = value.max(pixels[self.at(i - 1, j)]);
                }
                if i + 1 < self.rows {
                    value = value.max(pixels[self.at(i + 1, j)]);
                }
                if j > 0 {
                    value = value.max(pixels[self.at(i, j - 1)]);
                }
                if j + 1 < self.cols {
                    value = value.max(pixels[self.at(i, j + 1)]);
                }
                output[self.at(i, j)] = value;
            }
        }
        output
    }

    /// `cv::dilate` with a square `2 * range + 1` a side, separably.
    fn dilate_square(self, pixels: &[u8], range: isize) -> Vec<u8> {
        let window = |center: usize, length: usize| {
            let low = (center as isize - range).max(0) as usize;
            let high = ((center as isize + range) as usize).min(length - 1);
            low..=high
        };
        let mut rows = vec![0; self.len()];
        for i in 0..self.rows {
            for j in 0..self.cols {
                rows[self.at(i, j)] = window(j, self.cols)
                    .map(|l| pixels[self.at(i, l)])
                    .max()
                    .unwrap_or(0);
            }
        }
        let mut output = vec![0; self.len()];
        for i in 0..self.rows {
            for j in 0..self.cols {
                output[self.at(i, j)] = window(i, self.rows)
                    .map(|k| rows[self.at(k, j)])
                    .max()
                    .unwrap_or(0);
            }
        }
        output
    }

    /// `FastMarching_solve`.
    fn solve(
        self,
        (i1, j1): (usize, usize),
        (i2, j2): (usize, usize),
        flags: &[u8],
        times: &[f32],
    ) -> f32 {
        let a11 = f64::from(times[self.at(i1, j1)]);
        let a22 = f64::from(times[self.at(i2, j2)]);
        let m12 = a11.min(a22);
        let solution = if flags[self.at(i1, j1)] != INSIDE {
            if flags[self.at(i2, j2)] != INSIDE {
                if (a11 - a22).abs() >= 1.0 {
                    1.0 + m12
                } else {
                    (a11 + a22 + (2.0 - (a11 - a22) * (a11 - a22)).sqrt()) * 0.5
                }
            } else {
                1.0 + a11
            }
        } else if flags[self.at(i2, j2)] != INSIDE {
            1.0 + a22
        } else {
            1.0 + m12
        };
        solution as f32
    }

    /// `min4` of the four quadrants' solutions at `(i, j)`.
    fn arrival(self, i: usize, j: usize, flags: &[u8], times: &[f32]) -> f32 {
        let a = self.solve((i - 1, j), (i, j - 1), flags, times);
        let b = self.solve((i + 1, j), (i, j - 1), flags, times);
        let c = self.solve((i - 1, j), (i, j + 1), flags, times);
        let d = self.solve((i + 1, j), (i, j + 1), flags, times);
        min(min(a, b), min(c, d))
    }

    /// The 4-neighbour `q` of `(ii, jj)`, in OpenCV's order: up, left, down,
    /// right.
    fn neighbour(ii: usize, jj: usize, q: usize) -> (isize, isize) {
        let (ii, jj) = (ii as isize, jj as isize);
        match q {
            0 => (ii - 1, jj),
            1 => (ii, jj - 1),
            2 => (ii + 1, jj),
            _ => (ii, jj + 1),
        }
    }

    /// `icvCalcFMM`.
    fn march(self, flags: &mut [u8], times: &mut [f32], heap: &mut Queue, negate: bool) {
        let known = if negate { CHANGE } else { KNOWN };
        while let Some((ii, jj)) = heap.pop() {
            flags[self.at(ii, jj)] = known;
            for q in 0..4 {
                let (i, j) = Self::neighbour(ii, jj, q);
                // OpenCV's bounds, which let the last row and column through.
                if i <= 0 || j <= 0 || i > self.rows as isize || j > self.cols as isize {
                    continue;
                }
                let (i, j) = (i as usize, j as usize);
                if flags[self.at(i, j)] == INSIDE {
                    let time = self.arrival(i, j, flags, times);
                    times[self.at(i, j)] = time;
                    flags[self.at(i, j)] = BAND;
                    heap.push(i, j, time);
                }
            }
        }
        if negate {
            for index in 0..self.len() {
                if flags[index] == CHANGE {
                    flags[index] = KNOWN;
                    times[index] = -times[index];
                }
            }
        }
    }

    /// `icvTeleaInpaintFMM`, three-channel.
    fn fill(
        self,
        flags: &mut [u8],
        times: &mut [f32],
        output: &mut RgbImage,
        range: isize,
        heap: &mut Queue,
    ) {
        let (rows, cols) = (self.rows as isize, self.cols as isize);
        while let Some((ii, jj)) = heap.pop() {
            flags[self.at(ii, jj)] = KNOWN;
            for q in 0..4 {
                let (i, j) = Self::neighbour(ii, jj, q);
                if i <= 0 || j <= 0 || i > rows - 1 || j > cols - 1 {
                    continue;
                }
                let (iu, ju) = (i as usize, j as usize);
                if flags[self.at(iu, ju)] != INSIDE {
                    continue;
                }
                let time = self.arrival(iu, ju, flags, times);
                times[self.at(iu, ju)] = time;

                let not_inside =
                    |i: isize, j: isize| flags[self.at(i as usize, j as usize)] != INSIDE;
                let t = |i: isize, j: isize| times[self.at(i as usize, j as usize)];
                let grad_t = (
                    gradient(
                        not_inside(i, j + 1),
                        not_inside(i, j - 1),
                        t(i, j + 1),
                        t(i, j),
                        t(i, j - 1),
                    ),
                    gradient(
                        not_inside(i + 1, j),
                        not_inside(i - 1, j),
                        t(i + 1, j),
                        t(i, j),
                        t(i - 1, j),
                    ),
                );

                let mut jx = [0.0_f32; 3];
                let mut jy = [0.0_f32; 3];
                let mut ia = [0.0_f32; 3];
                let mut s = [1.0e-20_f32; 3];
                let pixel = |row: isize, col: isize, color: usize| {
                    f32::from(output.get_pixel(col as u32, row as u32).0[color])
                };
                let value = |row: isize, col: isize, color: usize| {
                    i32::from(output.get_pixel(col as u32, row as u32).0[color])
                };
                for k in i - range..=i + range {
                    let km = k - 1 + isize::from(k == 1);
                    let kp = k - 1 - isize::from(k == rows - 2);
                    for l in j - range..=j + range {
                        let lm = l - 1 + isize::from(l == 1);
                        let lp = l - 1 - isize::from(l == cols - 2);
                        if !(k > 0 && l > 0 && k < rows - 1 && l < cols - 1) {
                            continue;
                        }
                        if !(not_inside(k, l)
                            && (l - j) * (l - j) + (k - i) * (k - i) <= range * range)
                        {
                            continue;
                        }
                        let r = ((j - l) as f32, (i - k) as f32);
                        let length = r.0 * r.0 + r.1 * r.1;
                        let dst = (1.0 / (f64::from(length) * f64::from(length).sqrt())) as f32;
                        let lev = (1.0 / f64::from(1.0 + (t(k, l) - t(i, j)).abs())) as f32;
                        let mut dir = r.0 * grad_t.0 + r.1 * grad_t.1;
                        if f64::from(dir.abs()) <= 0.01 {
                            dir = 0.000_001;
                        }
                        let w = (dst * lev * dir).abs();
                        for color in 0..3 {
                            let grad_x = if not_inside(k, l + 1) {
                                if not_inside(k, l - 1) {
                                    (value(km, lp + 1, color) - value(km, lm - 1, color)) as f32
                                        * 2.0
                                } else {
                                    (value(km, lp + 1, color) - value(km, lm, color)) as f32
                                }
                            } else if not_inside(k, l - 1) {
                                (value(km, lp, color) - value(km, lm - 1, color)) as f32
                            } else {
                                0.0
                            };
                            let grad_y = if not_inside(k + 1, l) {
                                if not_inside(k - 1, l) {
                                    (value(kp + 1, lm, color) - value(km - 1, lm, color)) as f32
                                        * 2.0
                                } else {
                                    (value(kp + 1, lm, color) - value(km, lm, color)) as f32
                                }
                            } else if not_inside(k - 1, l) {
                                (value(kp, lm, color) - value(km - 1, lm, color)) as f32
                            } else {
                                0.0
                            };
                            ia[color] += w * pixel(k - 1, l - 1, color);
                            jx[color] -= w * (grad_x * r.0);
                            jy[color] -= w * (grad_y * r.1);
                            s[color] += w;
                        }
                    }
                }
                let target = output.get_pixel_mut((j - 1) as u32, (i - 1) as u32);
                for color in 0..3 {
                    let sat = ia[color] / s[color]
                        + (jx[color] + jy[color])
                            / ((jx[color] * jx[color] + jy[color] * jy[color]).sqrt() + 1.0e-20);
                    target.0[color] = round_cast(sat);
                }

                flags[self.at(iu, ju)] = BAND;
                heap.push(iu, ju, time);
            }
        }
    }
}

/// The arrival times' central or one-sided difference along an axis.
fn gradient(next_known: bool, previous_known: bool, next: f32, here: f32, previous: f32) -> f32 {
    match (next_known, previous_known) {
        (true, true) => (next - previous) * 0.5,
        (true, false) => next - here,
        (false, true) => here - previous,
        (false, false) => 0.0,
    }
}

/// OpenCV's `MIN` on floats.
fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `round_cast<uchar>`: `saturate_cast<uchar>(val + 0.5)`, which rounds the
/// double half to even.
fn round_cast(value: f32) -> u8 {
    (f64::from(value) + 0.5).round_ties_even().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use image::{Luma, Rgb};

    use super::*;

    /// A linear congruential sequence.
    struct Random(u64);

    impl Random {
        fn below(&mut self, range: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) % u64::from(range)) as u32
        }
    }

    /// A two-way gradient with noise and a few dark strokes.
    fn scene(width: u32, height: u32, seed: u64) -> RgbImage {
        let mut random = Random(seed);
        RgbImage::from_fn(width, height, |x, y| {
            let noise = random.below(9) as i32 - 4;
            let stroke = (x + 2 * y) % 37 < 2 || x % 53 == 7;
            let base = if stroke {
                [30, 25, 40]
            } else {
                [
                    (230 * x / width) as i32,
                    (200 * y / height) as i32 + 30,
                    180 - (100 * (x + y) / (width + height)) as i32,
                ]
            };
            Rgb(base.map(|channel| (channel + noise).clamp(0, 255) as u8))
        })
    }

    /// Rectangles, a disk, a thin line, and a region touching the top-left
    /// edge.
    fn holes(width: u32, height: u32, seed: u64) -> GrayImage {
        let mut random = Random(seed);
        let rects: Vec<(u32, u32, u32, u32)> = (0..4)
            .map(|_| {
                let w = 3 + random.below(width / 5);
                let h = 3 + random.below(height / 5);
                (random.below(width - w), random.below(height - h), w, h)
            })
            .collect();
        let (cx, cy, r) = (width / 2, height / 2, 2 + height / 8);
        GrayImage::from_fn(width, height, |x, y| {
            let rect = rects
                .iter()
                .any(|&(rx, ry, w, h)| x >= rx && x < rx + w && y >= ry && y < ry + h);
            let (dx, dy) = (i64::from(x) - i64::from(cx), i64::from(y) - i64::from(cy));
            let disk = dx * dx + dy * dy <= i64::from(r * r);
            let line = y == height / 3 && x > width / 4 && x < 3 * width / 4;
            let edge = x < 4 && y < 6;
            Luma([if rect || disk || line || edge { 255 } else { 0 }])
        })
    }

    /// FNV-1a, to compare whole images against a recorded value.
    fn fingerprint(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// Each case against `cv2.inpaint(image, mask, radius, cv2.INPAINT_TELEA)`
    /// from opencv-python-headless 4.13.0.92 on the same inputs.
    #[test]
    fn telea_matches_opencv() {
        let cases = [
            (97, 71, 1, 3, 0xaff2_3853_f00e_ad05),
            (160, 120, 2, 5, 0x8665_8ca3_c881_947c),
            (64, 48, 3, 1, 0xd5ab_75a5_443a_33d1),
            (120, 90, 4, 10, 0x3e09_60a9_f55a_838b),
        ];
        for (width, height, seed, radius, expected) in cases {
            let output = inpaint(
                &scene(width, height, seed),
                &holes(width, height, seed),
                radius,
            );
            assert_eq!(
                fingerprint(output.as_raw()),
                expected,
                "{width}×{height}, radius {radius}"
            );
        }
    }
}
