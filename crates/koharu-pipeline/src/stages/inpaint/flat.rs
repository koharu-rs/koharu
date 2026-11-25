//! The fill tiers that rebuild a lettering's surroundings without the
//! inpainting model. Each reads a ring of unmasked pixels just past the
//! lettering's grown mask, inside its balloon or panel, and the first tier
//! that can rebuild the ring fills the mask: one colour for plain paper, and
//! Telea's fast marching for a smooth gradient. Everything else goes to the
//! model: a lettering crossing its balloon's outline, a line or art against
//! it, and screentone, which LaMa redrew cleanly on Give My Regards to Black
//! Jack where tiles of the page, as Leptonica paints them, had copied art
//! over Blue Archive's colour pages.
//!
//! Most lettering has one colour all round, told without its balloon. The rest
//! on plain paper is read again around each separate piece of its mask, since
//! detection can mask a stray piece on other paper or on a balloon's line, and
//! a glyph can lie against art. What its paper doesn't reach goes to the model
//! alone.

use std::collections::HashSet;

use image::{GrayImage, Luma, Rgb, RgbImage, imageops};
use imageproc::{
    distance_transform::Norm,
    morphology::dilate,
    region_labelling::{Connectivity, connected_components},
};
use koharu_scene::Rect;
use kurbo::BezPath;

use super::{inside, masked, telea};
use crate::stages::held;

/// How far past the grown mask its surroundings are read.
const RING: u32 = 4;
/// How far inside a balloon's outline its paper is read, past the line.
const EDGE: u32 = 3;
/// The fewest ring pixels a fill is judged from.
const RING_PIXELS: usize = 16;
/// The most any pixel of the whole ring may stray from its median, in any
/// channel, for the lettering to have one colour all round.
const PLAIN: u8 = 16;
/// The share of a lettering's mask that must lie inside its balloon.
const INSIDE: f64 = 0.9;
/// The most the ring may stray from its fit, on average, in any channel, to
/// be one colour or a gradient.
const FLAT: f64 = 10.0;
/// A ring pixel this far from its fit, in any channel, is a line or art.
const LINE_CONTRAST: f64 = 64.0;
/// How far Telea reads known pixels around each one it fills.
const RADIUS: u32 = 5;

/// Pixels of the page.
type Pixels = Vec<(u32, u32)>;

/// Where a lettering sits: its balloon's outline, and its panel's bounds on
/// the page.
#[derive(Clone, Default)]
pub(super) struct Place {
    pub(super) balloon: Option<BezPath>,
    pub(super) panel: Option<Rect>,
}

/// Where a lettering's paper lies: in its balloon, whose pixels of `rect`
/// around the lettering are found once for all its pieces, or in its panel.
struct Reach {
    rect: Rect,
    balloon: Option<GrayImage>,
    panel: Option<Rect>,
}

impl Reach {
    fn new(place: &Place, rect: Rect) -> Self {
        Self {
            rect,
            balloon: place.balloon.as_ref().map(|outline| held(outline, rect)),
            panel: place.panel,
        }
    }

    /// Whether the page pixel at `x`, `y` is in the balloon, if there's one.
    fn holds(&self, x: i64, y: i64) -> bool {
        self.balloon.as_ref().is_none_or(|pixels| {
            let (x, y) = (x - i64::from(self.rect.x), y - i64::from(self.rect.y));
            match (u32::try_from(x), u32::try_from(y)) {
                (Ok(x), Ok(y)) => pixels
                    .get_pixel_checked(x, y)
                    .is_some_and(|pixel| pixel.0[0] != 0),
                _ => false,
            }
        })
    }

    /// Whether paper is read at the page pixel `x`, `y`: in the balloon, away
    /// from its outline, or in the panel.
    fn reads(&self, x: u32, y: u32) -> bool {
        if let (None, Some(panel)) = (&self.balloon, self.panel) {
            return inside(panel, x, y);
        }
        let edge = i64::from(EDGE);
        [(0, 0), (edge, 0), (-edge, 0), (0, edge), (0, -edge)]
            .into_iter()
            .all(|(dx, dy)| self.holds(i64::from(x) + dx, i64::from(y) + dy))
    }
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
    /// What the model redraws, over the lettering's rectangle: its mask, or
    /// the part of it no tier could rebuild.
    Model(GrayImage),
}

/// What a mask lies on, as the pixels around it tell.
enum Paper {
    Color([u8; 3]),
    Smooth,
    /// `line` when a line or art runs against the mask.
    Art {
        line: bool,
    },
    /// Too few pixels to tell.
    Unread,
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

    /// Fills the lettering whose grown mask is `mask` over `rect`, as far as
    /// the tiers can rebuild what's around it.
    pub(super) fn fill(&mut self, rect: Rect, mask: &GrayImage, place: &Place) -> Verdict {
        let ring = self.around(rect, mask, RING);
        if let Some(color) = self.plain(&ring) {
            self.paint(masked(rect, mask), color);
            return Verdict::Filled(Fill::Color);
        }
        let (width, height) = self.image.dimensions();
        let reach = Reach::new(place, grown(rect, RING + EDGE, width, height));
        match self.paper(rect, mask, ring, &reach) {
            Paper::Color(color) => self.piecewise(rect, mask, color, &reach),
            Paper::Smooth => {
                self.smooth(rect, mask);
                Verdict::Filled(Fill::Smooth)
            }
            Paper::Art { line: true } => Verdict::Model(bordered(mask)),
            Paper::Art { line: false } | Paper::Unread => Verdict::Model(mask.clone()),
        }
    }

    /// Fills each piece of a lettering on `color` paper from its own paper,
    /// but for what that doesn't reach.
    fn piecewise(
        &mut self,
        rect: Rect,
        mask: &GrayImage,
        color: [u8; 3],
        reach: &Reach,
    ) -> Verdict {
        let mut left = GrayImage::new(mask.width(), mask.height());
        for (own, piece) in pieces(rect, mask) {
            let ring = self.around(own, &piece, RING);
            let (color, (reached, unreached)) = match self.paper(own, &piece, ring, reach) {
                Paper::Color(color) => (color, self.split(own, &piece, color, reach)),
                // Hemmed in by the rest of its lettering, whose paper it's on.
                Paper::Unread => (color, self.split(own, &piece, color, reach)),
                Paper::Art { .. } => (color, (Vec::new(), masked(own, &piece).collect())),
                Paper::Smooth => {
                    self.smooth(own, &piece);
                    continue;
                }
            };
            self.paint(reached, color);
            for (x, y) in unreached {
                left.put_pixel(x - rect.x as u32, y - rect.y as u32, Luma([255]));
            }
        }
        match left.pixels().any(|pixel| pixel.0[0] != 0) {
            true => Verdict::Model(bordered(&left)),
            false => Verdict::Filled(Fill::Color),
        }
    }

    fn paint(&mut self, pixels: impl IntoIterator<Item = (u32, u32)>, color: [u8; 3]) {
        for (x, y) in pixels {
            self.image.put_pixel(x, y, Rgb(color));
        }
    }

    /// The one colour all of `ring` is, if it is.
    fn plain(&self, ring: &[(u32, u32)]) -> Option<[u8; 3]> {
        if ring.len() < RING_PIXELS {
            return None;
        }
        let colors = self.colors(ring);
        let median = median(&colors);
        colors
            .iter()
            .all(|color| (0..3).all(|at| color[at].abs_diff(median[at]) <= PLAIN))
            .then_some(median)
    }

    /// What `mask` over `rect` lies on, as the part of its `ring` that
    /// `reach` reads tells.
    fn paper(&self, rect: Rect, mask: &GrayImage, mut ring: Pixels, reach: &Reach) -> Paper {
        let (mut inside, mut all) = (0, 0);
        for (x, y) in masked(rect, mask) {
            inside += usize::from(reach.holds(i64::from(x), i64::from(y)));
            all += 1;
        }
        if (inside as f64) < INSIDE * all as f64 {
            return Paper::Art { line: false };
        }
        ring.retain(|&(x, y)| reach.reads(x, y));
        if ring.len() < RING_PIXELS {
            return Paper::Unread;
        }
        let colors = self.colors(&ring);
        let median = median(&colors);
        let flat = Stray::of(&ring, &colors, |_, _| median.map(f64::from));
        if flat.mean < FLAT && flat.most <= LINE_CONTRAST {
            return Paper::Color(median);
        }
        let plane = Plane::fit(&ring, &colors);
        let smooth = Stray::of(&ring, &colors, |x, y| plane.at(x, y));
        if smooth.mean < FLAT && smooth.most <= LINE_CONTRAST {
            return Paper::Smooth;
        }
        Paper::Art {
            line: smooth.most > LINE_CONTRAST,
        }
    }

    /// The pixels of `mask` over `rect` that its `color` paper reaches, and
    /// those it doesn't: the ones outside the balloon, and the ones a line or
    /// art goes on under from outside the mask, a balloon's line too, which
    /// the ring leaves out.
    fn split(
        &self,
        rect: Rect,
        mask: &GrayImage,
        color: [u8; 3],
        reach: &Reach,
    ) -> (Pixels, Pixels) {
        let unlike = |x: u32, y: u32, by: f64| {
            let found = self.image.get_pixel(x, y).0;
            (0..3).any(|at| f64::from(found[at].abs_diff(color[at])) > by)
        };
        let mut met = self.around(rect, mask, 1);
        met.retain(|&(x, y)| unlike(x, y, LINE_CONTRAST));
        // Under the mask, a line's soft edges too.
        let under = masked(rect, mask).filter(|&(x, y)| unlike(x, y, LINE_CONTRAST / 2.0));
        let (width, height) = self.image.dimensions();
        let outer = grown(rect, 1, width, height);
        let at = |x: u32, y: u32| (x - outer.x as u32, y - outer.y as u32);
        let mut ink = GrayImage::new(outer.width, outer.height);
        for (x, y) in under.chain(met.iter().copied()) {
            let (x, y) = at(x, y);
            ink.put_pixel(x, y, Luma([255]));
        }
        let labels = connected_components(&ink, Connectivity::Eight, Luma([0]));
        let label = |x: u32, y: u32| {
            let (x, y) = at(x, y);
            labels.get_pixel(x, y).0[0]
        };
        let met: HashSet<u32> = met.into_iter().map(|(x, y)| label(x, y)).collect();
        masked(rect, mask).partition(|&(x, y)| {
            reach.holds(i64::from(x), i64::from(y)) && !met.contains(&label(x, y))
        })
    }

    fn colors(&self, pixels: &[(u32, u32)]) -> Vec<[u8; 3]> {
        pixels
            .iter()
            .map(|&(x, y)| self.image.get_pixel(x, y).0)
            .collect()
    }

    /// The pixels within `by` of the mask that no mask covers.
    fn around(&self, rect: Rect, mask: &GrayImage, by: u32) -> Pixels {
        let (width, height) = self.image.dimensions();
        let outer = grown(rect, by, width, height);
        let mut near = GrayImage::new(outer.width, outer.height);
        imageops::replace(
            &mut near,
            mask,
            i64::from(rect.x - outer.x),
            i64::from(rect.y - outer.y),
        );
        let near = dilate(&near, Norm::LInf, by as u8);
        masked(outer, &near)
            .filter(|&(x, y)| self.covered.get_pixel(x, y).0[0] == 0)
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

/// `mask` with a pixel around it, where a line running against it goes on.
fn bordered(mask: &GrayImage) -> GrayImage {
    dilate(mask, Norm::LInf, 1)
}

/// The separate pieces of `mask` over `rect`, each over its own rectangle.
fn pieces(rect: Rect, mask: &GrayImage) -> Vec<(Rect, GrayImage)> {
    let labels = connected_components(mask, Connectivity::Eight, Luma([0]));
    // Each piece's edges in `mask`, the right and bottom ones inclusive.
    let mut edges: Vec<[u32; 4]> = Vec::new();
    for (x, y, label) in labels.enumerate_pixels() {
        // Labels number the pieces from 1.
        let Some(at) = (label.0[0] as usize).checked_sub(1) else {
            continue;
        };
        if edges.len() <= at {
            edges.resize(at + 1, [u32::MAX, u32::MAX, 0, 0]);
        }
        let [left, top, right, bottom] = edges[at];
        edges[at] = [left.min(x), top.min(y), right.max(x), bottom.max(y)];
    }
    edges
        .into_iter()
        .zip(1..)
        .map(|([left, top, right, bottom], label)| {
            let own = Rect {
                x: rect.x + left as i32,
                y: rect.y + top as i32,
                width: right - left + 1,
                height: bottom - top + 1,
            };
            let piece = GrayImage::from_fn(own.width, own.height, |x, y| {
                Luma([match labels.get_pixel(left + x, top + y).0[0] == label {
                    true => 255,
                    false => 0,
                }])
            });
            (own, piece)
        })
        .collect()
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
    use kurbo::Shape as _;

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

    /// A white balloon shaded at its right, its line from x 97 to 100: a
    /// lettering on the white, with a piece of its mask on the shade, one over
    /// the line, and one up against it.
    #[test]
    fn a_stray_piece_takes_its_own_paper_and_one_meeting_a_line_is_the_models() {
        let pieces = [
            (15, 15, 30, 30),
            (80, 20, 4, 4),
            (97, 40, 6, 4),
            (93, 6, 4, 4),
        ];
        let mask = GrayImage::from_fn(120, 60, |x, y| {
            let held = pieces.iter().any(|&(left, top, width, height)| {
                (left..left + width).contains(&x) && (top..top + height).contains(&y)
            });
            Luma([if held { 255 } else { 0 }])
        });
        let mut image = RgbImage::from_fn(120, 60, |x, y| {
            Rgb([match x {
                _ if mask.get_pixel(x, y).0[0] != 0 => 0,
                0..70 => 255,
                70..97 => 230,
                97..100 => 0,
                _ => 100,
            }; 3])
        });
        let rect = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 60,
        };
        let place = Place {
            balloon: Some(kurbo::Rect::new(0.0, 0.0, 100.0, 60.0).to_path(0.1)),
            panel: None,
        };
        let verdict = Page::new(&mut image, &mask).fill(rect, &mask, &place);
        let Verdict::Model(left) = verdict else {
            panic!("the pieces meeting the line are the model's");
        };
        for (x, y, filled) in [
            (30, 30, Some(255)),
            (81, 21, Some(230)),
            (98, 41, None),
            (94, 7, None),
        ] {
            assert_eq!(image.get_pixel(x, y).0, [filled.unwrap_or(0); 3]);
            assert_eq!(left.get_pixel(x, y).0[0] != 0, filled.is_none());
        }
    }

    #[test]
    fn a_line_against_the_lettering_goes_to_the_model() {
        let (verdict, _) = verdict(|x, _| if x == 31 { [10; 3] } else { [240; 3] });
        assert!(matches!(verdict, Verdict::Model(_)));
    }
}
