//! The fill tiers that rebuild a lettering's surroundings without the
//! inpainting model. Each reads a ring of unmasked pixels just past the
//! lettering's grown mask, inside its balloon or panel, and the first tier
//! that can rebuild the ring fills the mask: one colour for plain paper, and
//! Telea's fast marching for a smooth gradient. Everything else goes to the
//! model: a lettering crossing its balloon's outline, a line or art against
//! it, and screentone, which LaMa redrew cleanly on Give My Regards to Black
//! Jack where tiles of the page, as Leptonica paints them, had copied art
//! over Blue Archive's colour pages.

use image::{GrayImage, Rgb, RgbImage, imageops};
use imageproc::{distance_transform::Norm, morphology::dilate};
use koharu_scene::Rect;
use kurbo::{BezPath, Point, Shape as _};

use super::{masked, telea};

/// How far past the grown mask its surroundings are read.
const RING: u32 = 4;
/// How far inside a balloon's outline its paper is read, past the line.
const EDGE: f64 = 3.0;
/// The fewest ring pixels a fill is judged from.
const RING_PIXELS: usize = 16;
/// The share of a lettering's mask that must lie inside its balloon.
const INSIDE: f64 = 0.9;
/// The most the ring may stray from its fit, on average, in any channel, to
/// be one colour or a gradient.
const FLAT: f64 = 10.0;
/// A ring pixel this far from its fit, in any channel, is a line or art.
const LINE_CONTRAST: f64 = 64.0;
/// How far Telea reads known pixels around each one it fills.
const RADIUS: u32 = 5;

/// Where a lettering sits: its balloon's outline, and its panel's bounds on
/// the page.
#[derive(Clone, Default)]
pub(super) struct Place {
    pub(super) balloon: Option<BezPath>,
    pub(super) panel: Option<Rect>,
}

/// The tier that filled a lettering, as its C2PA action records it.
#[derive(Clone, Copy, Debug, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub(super) enum Fill {
    Color,
    Smooth,
}

pub(super) enum Verdict {
    Filled(Fill),
    /// The model redraws it; `line` when a line or art runs against it.
    Model {
        line: bool,
    },
}

/// A page's scan as the tiers fill it, and every lettering's grown mask.
pub(super) struct Page<'a> {
    image: &'a mut RgbImage,
    covered: &'a GrayImage,
}

impl<'a> Page<'a> {
    pub(super) fn new(image: &'a mut RgbImage, covered: &'a GrayImage) -> Self {
        Self { image, covered }
    }

    /// Fills the lettering whose grown mask is `mask` over `rect`, if a tier
    /// can rebuild what's around it.
    pub(super) fn fill(&mut self, rect: Rect, mask: &GrayImage, place: &Place) -> Verdict {
        let pixels: Vec<(u32, u32)> = masked(rect, mask).collect();
        if let Some(outline) = &place.balloon {
            let inside = pixels
                .iter()
                .filter(|&&(x, y)| outline.contains(centre(x, y)))
                .count();
            if (inside as f64) < INSIDE * pixels.len() as f64 {
                return Verdict::Model { line: false };
            }
        }
        let ring = self.ring(rect, mask, place);
        if ring.len() < RING_PIXELS {
            return Verdict::Model { line: false };
        }
        let colors: Vec<[u8; 3]> = ring
            .iter()
            .map(|&(x, y)| self.image.get_pixel(x, y).0)
            .collect();
        let median = median(&colors);
        let flat = Stray::of(&ring, &colors, |_, _| median.map(f64::from));
        if flat.mean < FLAT && flat.most <= LINE_CONTRAST {
            for &(x, y) in &pixels {
                self.image.put_pixel(x, y, Rgb(median));
            }
            return Verdict::Filled(Fill::Color);
        }
        let plane = Plane::fit(&ring, &colors);
        let smooth = Stray::of(&ring, &colors, |x, y| plane.at(x, y));
        if smooth.mean < FLAT && smooth.most <= LINE_CONTRAST {
            self.smooth(rect, mask);
            return Verdict::Filled(Fill::Smooth);
        }
        Verdict::Model {
            line: smooth.most > LINE_CONTRAST,
        }
    }

    /// The unmasked pixels within `RING` of the mask, inside its balloon,
    /// away from the outline, or its panel.
    fn ring(&self, rect: Rect, mask: &GrayImage, place: &Place) -> Vec<(u32, u32)> {
        let (width, height) = self.image.dimensions();
        let outer = grown(rect, RING, width, height);
        let mut near = GrayImage::new(outer.width, outer.height);
        imageops::replace(
            &mut near,
            mask,
            i64::from(rect.x - outer.x),
            i64::from(rect.y - outer.y),
        );
        let near = dilate(&near, Norm::LInf, RING as u8);
        masked(outer, &near)
            .filter(|&(x, y)| self.covered.get_pixel(x, y).0[0] == 0)
            .filter(|&(x, y)| match (&place.balloon, place.panel) {
                (Some(outline), _) => {
                    let at = centre(x, y);
                    [
                        (0.0, 0.0),
                        (EDGE, 0.0),
                        (-EDGE, 0.0),
                        (0.0, EDGE),
                        (0.0, -EDGE),
                    ]
                    .into_iter()
                    .all(|(dx, dy)| outline.contains(Point::new(at.x + dx, at.y + dy)))
                }
                (None, Some(panel)) => contains(panel, x, y),
                (None, None) => true,
            })
            .collect()
    }

    /// Telea's fill from the mask's boundary, every lettering around held
    /// out of what it reads.
    fn smooth(&mut self, rect: Rect, mask: &GrayImage) {
        let (width, height) = self.image.dimensions();
        let window = grown(rect, RADIUS + 1, width, height);
        let (x, y) = (window.x as u32, window.y as u32);
        let image = imageops::crop_imm(&*self.image, x, y, window.width, window.height).to_image();
        let holes = imageops::crop_imm(self.covered, x, y, window.width, window.height).to_image();
        let filled = telea::inpaint(&image, &holes, RADIUS);
        for (px, py) in masked(rect, mask) {
            self.image
                .put_pixel(px, py, *filled.get_pixel(px - x, py - y));
        }
    }
}

/// How far a ring strays from what's fitted to it: the mean absolute
/// difference of its worst channel, and the largest in any.
struct Stray {
    mean: f64,
    most: f64,
}

impl Stray {
    fn of(ring: &[(u32, u32)], colors: &[[u8; 3]], fit: impl Fn(u32, u32) -> [f64; 3]) -> Self {
        let mut sums = [0.0; 3];
        let mut most: f64 = 0.0;
        for (&(x, y), color) in ring.iter().zip(colors) {
            let fitted = fit(x, y);
            for channel in 0..3 {
                let stray = (f64::from(color[channel]) - fitted[channel]).abs();
                sums[channel] += stray;
                most = most.max(stray);
            }
        }
        let mean = sums.into_iter().fold(0.0, f64::max) / ring.len() as f64;
        Self { mean, most }
    }
}

/// Each channel's median.
fn median(colors: &[[u8; 3]]) -> [u8; 3] {
    std::array::from_fn(|channel| {
        let mut values: Vec<u8> = colors.iter().map(|color| color[channel]).collect();
        let middle = values.len() / 2;
        *values.select_nth_unstable(middle).1
    })
}

/// A least-squares plane through each channel, about the ring's centre.
struct Plane {
    centre: (f64, f64),
    /// Each channel's value at the centre, and its slopes along x and y.
    channels: [[f64; 3]; 3],
}

impl Plane {
    fn fit(ring: &[(u32, u32)], colors: &[[u8; 3]]) -> Self {
        let count = ring.len() as f64;
        let centre = ring.iter().fold((0.0, 0.0), |(sx, sy), &(x, y)| {
            (sx + f64::from(x) / count, sy + f64::from(y) / count)
        });
        let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
        let mut sums = [[0.0; 3]; 3];
        for (&(x, y), color) in ring.iter().zip(colors) {
            let (dx, dy) = (f64::from(x) - centre.0, f64::from(y) - centre.1);
            xx += dx * dx;
            xy += dx * dy;
            yy += dy * dy;
            for (sum, &value) in sums.iter_mut().zip(color) {
                let value = f64::from(value);
                sum[0] += value;
                sum[1] += value * dx;
                sum[2] += value * dy;
            }
        }
        // About the centre, the constant term separates from the slopes.
        let det = xx * yy - xy * xy;
        let channels = sums.map(|[sum, sx, sy]| {
            let mean = sum / count;
            if det.abs() < f64::EPSILON {
                [mean, 0.0, 0.0]
            } else {
                [mean, (sx * yy - sy * xy) / det, (sy * xx - sx * xy) / det]
            }
        });
        Self { centre, channels }
    }

    fn at(&self, x: u32, y: u32) -> [f64; 3] {
        let (dx, dy) = (f64::from(x) - self.centre.0, f64::from(y) - self.centre.1);
        self.channels
            .map(|[value, along_x, along_y]| value + along_x * dx + along_y * dy)
    }
}

fn centre(x: u32, y: u32) -> Point {
    Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5)
}

fn contains(rect: Rect, x: u32, y: u32) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    (i64::from(rect.x)..i64::from(rect.x) + i64::from(rect.width)).contains(&x)
        && (i64::from(rect.y)..i64::from(rect.y) + i64::from(rect.height)).contains(&y)
}

/// `rect` grown by `by` on every side, within a `width` by `height` page.
fn grown(rect: Rect, by: u32, width: u32, height: u32) -> Rect {
    let x = (i64::from(rect.x) - i64::from(by)).max(0);
    let y = (i64::from(rect.y) - i64::from(by)).max(0);
    let right = (i64::from(rect.x) + i64::from(rect.width) + i64::from(by)).min(i64::from(width));
    let bottom =
        (i64::from(rect.y) + i64::from(rect.height) + i64::from(by)).min(i64::from(height));
    Rect {
        x: x as i32,
        y: y as i32,
        width: (right - x) as u32,
        height: (bottom - y) as u32,
    }
}

#[cfg(test)]
mod tests {
    use image::Luma;

    use super::*;

    /// A page of paper `paper`, with a lettering's mask of 10 by 6 at (20, 20).
    fn page(paper: impl Fn(u32, u32) -> [u8; 3]) -> (RgbImage, GrayImage, Rect, GrayImage) {
        let image = RgbImage::from_fn(60, 50, |x, y| Rgb(paper(x, y)));
        let rect = Rect {
            x: 20,
            y: 20,
            width: 10,
            height: 6,
        };
        let mask = GrayImage::from_pixel(10, 6, Luma([255]));
        let mut covered = GrayImage::new(60, 50);
        imageops::replace(&mut covered, &mask, 20, 20);
        (image, covered, rect, mask)
    }

    fn verdict(paper: impl Fn(u32, u32) -> [u8; 3]) -> (Verdict, RgbImage) {
        let (mut image, covered, rect, mask) = page(paper);
        let verdict = Page::new(&mut image, &covered).fill(rect, &mask, &Place::default());
        (verdict, image)
    }

    #[test]
    fn plain_paper_takes_its_colour_and_a_gradient_a_smooth_fill() {
        let (filled, image) = verdict(|_, _| [240, 238, 230]);
        assert!(matches!(filled, Verdict::Filled(_)));
        assert_eq!(image.get_pixel(25, 23).0, [240, 238, 230]);

        let (filled, image) = verdict(|x, _| [(60 + 3 * x) as u8; 3]);
        assert!(matches!(filled, Verdict::Filled(_)));
        let [value, ..] = image.get_pixel(25, 23).0;
        assert!(value.abs_diff((60 + 3 * 25) as u8) <= 4, "{value}");
    }

    #[test]
    fn a_line_against_the_lettering_goes_to_the_model() {
        let (verdict, _) = verdict(|x, _| if x == 31 { [10; 3] } else { [240; 3] });
        assert!(matches!(verdict, Verdict::Model { line: true }));
    }
}
