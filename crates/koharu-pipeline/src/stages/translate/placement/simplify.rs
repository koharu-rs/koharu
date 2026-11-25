//! A room's outline as a text's frame takes it. A balloon's outline is traced
//! from its mask, with an anchor every few pixels and around every spike, and
//! a frame is reshaped by its anchors. So a frame has the few that keep the
//! shape of its room's body: the four of a rectangle or an ellipse, or those
//! of a smooth curve. Of 183 balloons, 120 of Manga109's test pages and 63 of
//! Blue Archive's comics, whose traced outlines had a median of 76 anchors and
//! at most 155, the frames have a median of 5 and at most 11. The median frame
//! holds 96% of its balloon with 0.7% of itself outside, and none has 2.5%
//! outside.

use image::GrayImage;
use imageproc::{
    contours::{BorderType, find_contours},
    distance_transform::Norm,
    geometry::contour_area,
    morphology::open,
};
use koharu_scene::Rect;
use kurbo::{
    Affine, BezPath, Circle, CubicBez, ParamCurve as _, ParamCurveMoments as _,
    ParamCurveNearest as _, Point, Shape as _, Vec2,
};

use crate::stages::held;

/// A spike or a tail narrower than twice this share of the room's shorter
/// side isn't part of its body.
const SPIKE: f64 = 0.06;
/// The least share of a room its body is; one losing more to `SPIKE` is thin
/// all over, and is its own body. Of 3,004 rooms in Manga109's test balloons
/// holding several texts, the spikiest three were 71 to 77% body.
const BODY: f64 = 0.5;
/// How far along the outline its points are averaged, as a share of the
/// room's shorter side, which takes out the steps of the pixels.
const SMOOTHING: f64 = 0.04;
/// The furthest a frame strays from its room's body, as a share of the room's
/// shorter side.
const STRAY: f64 = 0.04;
/// The points along an outline that a curve is fitted to.
const POINTS: usize = 128;

/// `room` simplified; itself when that takes no fewer anchors.
pub(super) fn simplified(room: BezPath) -> BezPath {
    let anchors = room.segments().count();
    // No frame has fewer.
    if anchors <= 4 {
        return room;
    }
    let bounds = room.bounding_box();
    let side = bounds.width().min(bounds.height());
    body(&room, bounds, side)
        .map(|body| primitive(&body, side * STRAY).unwrap_or_else(|| curve(&body, side)))
        .filter(|simple| simple.segments().count() < anchors)
        .unwrap_or(room)
}

/// The outline of `room`'s body, a point for each of its pixels.
fn body(room: &BezPath, bounds: kurbo::Rect, side: f64) -> Option<Vec<Point>> {
    // With a pixel outside the room all around.
    let rect = Rect {
        x: bounds.x0.floor() as i32 - 1,
        y: bounds.y0.floor() as i32 - 1,
        width: bounds.width().ceil() as u32 + 3,
        height: bounds.height().ceil() as u32 + 3,
    };
    let outlines = |pixels: &GrayImage| -> Vec<Vec<imageproc::point::Point<i32>>> {
        find_contours(pixels)
            .into_iter()
            .filter(|contour| contour.border_type == BorderType::Outer)
            .map(|contour| contour.points)
            .filter(|points| contour_area(points) > 0.0)
            .collect()
    };
    let pixels = held(room, rect);
    let whole = outlines(&pixels)
        .into_iter()
        .max_by(|left, right| contour_area(left).total_cmp(&contour_area(right)))?;
    let radius = (side * SPIKE).round().clamp(1.0, 255.0) as u8;
    // Two outlines are lobes either side of a neck, which a room keeps.
    let body = match outlines(&open(&pixels, Norm::L2, radius)).as_slice() {
        [only] if contour_area(only) >= BODY * contour_area(&whole) => only.clone(),
        _ => whole,
    };
    let centre = Vec2::new(f64::from(rect.x) + 0.5, f64::from(rect.y) + 0.5);
    Some(
        body.iter()
            .map(|point| Point::new(f64::from(point.x), f64::from(point.y)) + centre)
            .collect(),
    )
}

/// The rectangle or ellipse, upright or else tilted, that `body` is, if none
/// of it strays further than `stray`: the one with `body`'s area moments, as
/// scikit-image's `regionprops` gives a region's ellipse.
fn primitive(body: &[Point], stray: f64) -> Option<BezPath> {
    let mut outline = BezPath::new();
    outline.move_to(*body.first()?);
    for &point in &body[1..] {
        outline.line_to(point);
    }
    outline.close_path();
    let (area, moments) = (outline.area(), outline.moments());
    let centre = Vec2::new(moments.moment_x, moments.moment_y) / area;
    let xx = moments.moment_xx / area - centre.x * centre.x;
    let xy = moments.moment_xy / area - centre.x * centre.y;
    let yy = moments.moment_yy / area - centre.y * centre.y;
    // Along and across the tilt its variance is greatest at.
    let tilt = (2.0 * xy).atan2(xx - yy) / 2.0;
    let (middle, spread) = ((xx + yy) / 2.0, ((xx - yy) / 2.0).hypot(xy));
    [(0.0, xx, yy), (tilt, middle + spread, middle - spread)]
        .into_iter()
        .flat_map(|(tilt, along, across)| {
            let placed = Affine::translate(centre)
                * Affine::rotate(tilt)
                * Affine::scale_non_uniform(along.max(0.0).sqrt(), across.max(0.0).sqrt());
            // A square and a circle whose variance is 1 each way.
            let half = 3f64.sqrt();
            [
                placed * kurbo::Rect::new(-half, -half, half, half).to_path(0.1),
                placed * Circle::new(Point::ORIGIN, 2.0).to_path(0.1),
            ]
        })
        .find(|shape| {
            body.iter().all(|&point| {
                shape
                    .segments()
                    .any(|segment| segment.nearest(point, 0.1).distance_sq <= stray * stray)
            })
        })
}

/// `body` smoothed into the fewest cubics straying no further than `STRAY`,
/// starting at its top. Each is a quarter of the outline at most, so a
/// balloon's four anchors lie about its top, bottom, and sides.
fn curve(body: &[Point], side: f64) -> BezPath {
    let points = smoothed(body, (side * SMOOTHING).max(1.0));
    let stray = side * STRAY;
    // The cheapest chain of cubics to each point, from the top: what it costs,
    // a cubic each and their strays, and the point its last one starts at.
    let mut chains = vec![(0.0, 0)];
    for to in 1..=POINTS {
        let cheapest = (to.saturating_sub(POINTS / 4)..to)
            .filter_map(|from| {
                let (_, strayed) = cubic(&points, from, to);
                (strayed <= stray).then(|| (chains[from].0 + 1.0 + strayed / stray, from))
            })
            .min_by(|left: &(f64, usize), right| left.0.total_cmp(&right.0))
            .expect("no point lies between one and the next to stray");
        chains.push(cheapest);
    }
    let mut to = POINTS;
    let mut cubics = Vec::new();
    while to > 0 {
        let from = chains[to].1;
        cubics.push(cubic(&points, from, to).0);
        to = from;
    }
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for cubic in cubics.iter().rev() {
        path.curve_to(cubic.p1, cubic.p2, cubic.p3);
    }
    path.close_path();
    path
}

/// `POINTS` points evenly along `body` from its top, each the Gaussian mean
/// of those around it along the outline.
fn smoothed(body: &[Point], deviation: f64) -> Vec<Point> {
    let mut length = 0.0;
    let placed: Vec<(f64, Point)> = body
        .iter()
        .zip(body.iter().cycle().skip(1))
        .map(|(&point, &next)| {
            let at = length;
            length += point.distance(next);
            (at, point)
        })
        .collect();
    let mut points: Vec<Point> = (0..POINTS)
        .map(|index| {
            let here = length * index as f64 / POINTS as f64;
            let (sum, total) =
                placed
                    .iter()
                    .fold((Vec2::ZERO, 0.0), |(sum, total), &(at, point)| {
                        let apart = (at - here).abs();
                        let apart = apart.min(length - apart) / deviation;
                        let weight = (-0.5 * apart * apart).exp();
                        (sum + point.to_vec2() * weight, total + weight)
                    });
            (sum / total).to_point()
        })
        .collect();
    let top = (0..POINTS)
        .min_by(|&left, &right| points[left].y.total_cmp(&points[right].y))
        .unwrap_or_default();
    points.rotate_left(top);
    points
}

/// The cubic along the closed `points` from one to another, and the furthest
/// a point between strays from it. Its handles lie along the outline, so the
/// next cubic goes on smoothly, as long as bring it nearest the points, as
/// Schneider fits digitized curves in Graphics Gems.
fn cubic(points: &[Point], from: usize, to: usize) -> (CubicBez, f64) {
    let at = |index: usize| points[index % points.len()];
    let along = |index: usize| {
        let along = at(index + 1) - at(index + points.len() - 1);
        along / along.hypot().max(f64::EPSILON)
    };
    let share = |index: usize| (index - from) as f64 / (to - from) as f64;
    let (start, end) = (at(from), at(to));
    let (out, back) = (along(from), -along(to));
    let [mut oo, mut ob, mut bb, mut o, mut b] = [0.0; 5];
    for index in from + 1..to {
        let t = share(index);
        let (first, second) = (3.0 * t * (1.0 - t).powi(2), 3.0 * t.powi(2) * (1.0 - t));
        let rest = at(index) - CubicBez::new(start, start, end, end).eval(t);
        oo += first * first;
        ob += first * second * out.dot(back);
        bb += second * second;
        o += first * out.dot(rest);
        b += second * back.dot(rest);
    }
    let length: f64 = (from..to)
        .map(|index| at(index).distance(at(index + 1)))
        .sum();
    let solved = oo * bb - ob * ob;
    let [out_by, back_by] = match solved > 1e-9 {
        true => [(o * bb - b * ob) / solved, (b * oo - o * ob) / solved],
        // Too few points tell.
        false => [length / 3.0; 2],
    }
    // Neither folding back nor reaching past the other's end.
    .map(|by| by.clamp(0.15 * length, 0.6 * length));
    let cubic = CubicBez::new(start, start + out * out_by, end + back * back_by, end);
    let strayed = (from + 1..to)
        .map(|index| cubic.eval(share(index)).distance(at(index)))
        .fold(0.0, f64::max);
    (cubic, strayed)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use kurbo::{BezPath, ParamCurve as _, ParamCurveNearest as _, Point, Shape as _};

    use super::simplified;

    /// A closed outline through a point at each of `count` angles, `reach` from
    /// `(300, 300)` as a share of 120 by 200.
    fn traced(count: u32, reach: impl Fn(u32) -> f64) -> BezPath {
        let mut path = BezPath::new();
        for index in 0..count {
            let (sin, cos) = (TAU * f64::from(index) / f64::from(count)).sin_cos();
            let reach = reach(index);
            let point = Point::new(300.0 + 120.0 * reach * cos, 300.0 + 200.0 * reach * sin);
            match index {
                0 => path.move_to(point),
                _ => path.line_to(point),
            }
        }
        path.close_path();
        path
    }

    #[test]
    fn a_spiky_balloon_becomes_the_ellipse_its_spikes_stand_on() {
        // Every third of 120 points is a spike's tip.
        let simple = simplified(traced(120, |index| if index % 3 == 0 { 1.2 } else { 1.0 }));

        assert_eq!(simple.segments().count(), 4, "{}", simple.to_svg());
        let bounds = simple.bounding_box();
        assert!((bounds.width() - 240.0).abs() < 24.0, "{bounds:?}");
        assert!((bounds.height() - 400.0).abs() < 24.0, "{bounds:?}");
    }

    #[test]
    fn a_balloon_like_neither_keeps_its_shape_in_a_few_smooth_anchors() {
        // Two lobes about a waist, too wide a neck to part them.
        let outline = traced(160, |index| {
            1.0 - 0.3 * (TAU * f64::from(index) / 160.0).cos().powi(2)
        });

        let simple = simplified(outline.clone());

        assert!(simple.segments().count() <= 10, "{}", simple.to_svg());
        // No point of the outline strays past the tolerance and the smoothing.
        for segment in outline.segments() {
            let strayed = simple
                .segments()
                .map(|curve| curve.nearest(segment.start(), 0.1).distance_sq)
                .fold(f64::INFINITY, f64::min)
                .sqrt();
            assert!(strayed <= 0.08 * 168.0, "{strayed}");
        }
    }
}
