//! A balloon's outline as a closed path, and the room each of its texts has
//! in it. Conjoined balloons, two balloons drawn overlapping, first part
//! along the chord between the notches where their outlines cross. Texts
//! still sharing a part split it by a seeded watershed on the distance from
//! the balloon's edge, so each border falls where it narrows. Alone, the
//! watershed floods a small lobe from its neighbour's text when the lobe has
//! no distance peak of its own. Of 24 random Manga109 test bubbles where
//! cutting at the waist first changes a room, it was better in 18, the same
//! in 3, and worse in 3, all spiky balloons whose spikes read as notches.

use std::{cmp::Reverse, collections::BinaryHeap};

use image::{GrayImage, ImageBuffer, Luma};
use imageproc::{
    contours::{BorderType, find_contours_with_threshold},
    distance_transform::euclidean_squared_distance_transform,
    drawing::draw_polygon_mut,
    geometry::{approximate_polygon_dp, arc_length, contour_area, convex_hull},
};
use kurbo::{BezPath, PathEl, Point, Rect, Shape};

use super::Mask;

/// A notch at least this deep, as a share of the balloon's square root of
/// area, is where two balloons' outlines cross. Among 40 hand-labelled
/// conjoined balloons in Manga109's test bubbles, the shallower notch of
/// each junction found was at least 0.045 deep.
const NOTCH: f64 = 0.04;

/// A chord parts texts only when it leaves each at least this share of its
/// box on one side. Of 24 random Manga109 test bubbles where a chord
/// changed a room, chords allowed down to 0.75 ran diagonally through a
/// text three times, and down to 0.9 once.
const SIDE: f64 = 0.9;

/// `mask`'s outline in page pixels: its largest outer contour, simplified.
pub(super) fn outline(mask: &Mask) -> Option<BezPath> {
    let image = GrayImage::from_raw(mask.width, mask.height, mask.pixels.clone())?;
    outline_of(&image, mask.x, mask.y)
}

/// The outline of `image`'s set pixels, whose corner is `(x, y)` on the page.
fn outline_of(image: &GrayImage, x: u32, y: u32) -> Option<BezPath> {
    let mut padded = GrayImage::new(image.width() + 2, image.height() + 2);
    image::imageops::replace(&mut padded, image, 1, 1);
    let contours = find_contours_with_threshold::<i32>(&padded, 0);
    let contour = contours
        .iter()
        .filter(|contour| contour.border_type == BorderType::Outer)
        .max_by(|left, right| contour_area(&left.points).total_cmp(&contour_area(&right.points)))?;
    if contour.points.len() < 3 {
        return None;
    }
    let epsilon = (arc_length(&contour.points, true) * 0.001).max(f64::EPSILON);
    let points = approximate_polygon_dp(&contour.points, epsilon, true);
    if points.len() < 3 {
        return None;
    }
    let points: Vec<Point> = points
        .iter()
        .map(|point| {
            Point::new(
                f64::from(point.x - 1) + f64::from(x),
                f64::from(point.y - 1) + f64::from(y),
            )
        })
        .collect();
    Some(closed(&points))
}

/// The closed polygon through `points`.
fn closed(points: &[Point]) -> BezPath {
    points
        .iter()
        .enumerate()
        .map(|(index, &point)| match index {
            0 => PathEl::MoveTo(point),
            _ => PathEl::LineTo(point),
        })
        .chain([PathEl::ClosePath])
        .collect()
}

/// One room in the balloon `outline` for each group of boxes, in page
/// pixels. Each group's boxes seed the balloon's pixels under them, so a
/// translation covering several texts gets one room, and stay whole in it.
/// A group whose room comes out empty gets the whole outline.
pub fn rooms(outline: &BezPath, groups: &[Vec<Rect>]) -> Vec<BezPath> {
    let whole = || vec![outline.clone(); groups.len()];
    if groups.len() < 2 {
        return whole();
    }
    let bounds = outline.bounding_box();
    let (x, y) = (
        bounds.x0.floor().max(0.0) as u32,
        bounds.y0.floor().max(0.0) as u32,
    );
    let (width, height) = (
        (bounds.x1.ceil().max(0.0) as u32).saturating_sub(x) + 2,
        (bounds.y1.ceil().max(0.0) as u32).saturating_sub(y) + 2,
    );
    // Local pixel (lx, ly) is page pixel (x + lx - 1, y + ly - 1), so a
    // one-pixel margin outside the balloon rings it.
    let page = |lx: u32, ly: u32| Point::new(f64::from(x + lx) - 0.5, f64::from(y + ly) - 0.5);
    let outside = GrayImage::from_fn(width, height, |lx, ly| {
        Luma([u8::from(!outline.contains(page(lx, ly))) * u8::MAX])
    });
    let distance = euclidean_squared_distance_transform(&outside);
    let seeds: Vec<Vec<(u32, u32)>> = groups
        .iter()
        .map(|boxes| {
            (0..height)
                .flat_map(|ly| (0..width).map(move |lx| (lx, ly)))
                .filter(|&(lx, ly)| {
                    outside.get_pixel(lx, ly)[0] == 0
                        && boxes.iter().any(|seed| seed.contains(page(lx, ly)))
                })
                .collect()
        })
        .collect();
    // Conjoined balloons part at their waists: `parts` numbers each part's
    // pixels, and `home` is the part holding each group. The outline goes to
    // local pixels, where pixel (lx, ly) sits at (lx, ly).
    let mut parts = ImageBuffer::<Luma<u32>, Vec<u32>>::new(width, height);
    let mut home = vec![0; groups.len()];
    let origin = page(0, 0).to_vec2();
    let mut polygon = Vec::new();
    kurbo::flatten(outline, 0.25, |element| {
        if let PathEl::MoveTo(point) | PathEl::LineTo(point) = element {
            polygon.push(point - origin);
        }
    });
    let mut pending = vec![(0, polygon, (0..groups.len()).collect::<Vec<_>>())];
    let mut count = 1;
    while let Some((part, polygon, members)) = pending.pop() {
        for &member in &members {
            home[member] = part;
        }
        if members.len() < 2 {
            continue;
        }
        let held: Vec<Vec<(u32, u32)>> = members
            .iter()
            .map(|&member| {
                seeds[member]
                    .iter()
                    .copied()
                    .filter(|&(lx, ly)| parts.get_pixel(lx, ly)[0] == part)
                    .collect()
            })
            .collect();
        let Some((start, end, between, side)) = waist(&polygon, &held, width, height) else {
            continue;
        };
        for (lx, ly, pixel) in parts.enumerate_pixels_mut() {
            if pixel[0] == part && side.get_pixel(lx, ly)[0] != 0 {
                *pixel = Luma([count]);
            }
        }
        let (inner, outer): (Vec<_>, Vec<_>) = members
            .into_iter()
            .zip(between)
            .partition(|&(_, between)| between);
        pending.push((
            count,
            polygon[start..=end].to_vec(),
            inner.into_iter().map(|(member, _)| member).collect(),
        ));
        pending.push((
            part,
            [&polygon[end..], &polygon[..=start]].concat(),
            outer.into_iter().map(|(member, _)| member).collect(),
        ));
        count += 1;
    }
    let mut labels = ImageBuffer::<Luma<u32>, Vec<u32>>::new(width, height);
    let mut queue = BinaryHeap::new();
    let mut order = 0u64;
    for (group, pixels) in seeds.iter().enumerate() {
        for &(lx, ly) in pixels {
            if parts.get_pixel(lx, ly)[0] == home[group] && labels.get_pixel(lx, ly)[0] == 0 {
                labels.put_pixel(lx, ly, Luma([group as u32 + 1]));
                queue.push((distance.get_pixel(lx, ly)[0] as u64, Reverse(order), lx, ly));
                order += 1;
            }
        }
    }
    // Lowest on -distance first: each part's middle floods before its necks.
    while let Some((_, _, lx, ly)) = queue.pop() {
        let label = labels.get_pixel(lx, ly)[0];
        for (nx, ny) in [
            (lx.wrapping_sub(1), ly),
            (lx + 1, ly),
            (lx, ly.wrapping_sub(1)),
            (lx, ly + 1),
        ] {
            if nx < width
                && ny < height
                && outside.get_pixel(nx, ny)[0] == 0
                && labels.get_pixel(nx, ny)[0] == 0
                && parts.get_pixel(nx, ny) == parts.get_pixel(lx, ly)
            {
                labels.put_pixel(nx, ny, Luma([label]));
                queue.push((distance.get_pixel(nx, ny)[0] as u64, Reverse(order), nx, ny));
                order += 1;
            }
        }
    }
    // A box crossing its waist keeps the corner across it.
    for (group, pixels) in seeds.iter().enumerate() {
        for &(lx, ly) in pixels {
            labels.put_pixel(lx, ly, Luma([group as u32 + 1]));
        }
    }
    (1..=groups.len() as u32)
        .map(|label| {
            let room = GrayImage::from_fn(width, height, |lx, ly| {
                Luma([u8::from(labels.get_pixel(lx, ly)[0] == label) * u8::MAX])
            });
            outline_of(&room, x.saturating_sub(1), y.saturating_sub(1))
                .unwrap_or_else(|| outline.clone())
        })
        .collect()
}

/// The chord inside `polygon`, in local pixels, between two of its notches
/// that parts the texts whose seed pixels `held` lists: the notches'
/// indices, whether each text lies between them, and the pixels between
/// them on a `width` × `height` mask. Of several, the shortest for the
/// smaller side's size: a waist is narrow for the lobes it parts.
fn waist(
    polygon: &[Point],
    held: &[Vec<(u32, u32)>],
    width: u32,
    height: u32,
) -> Option<(usize, usize, Vec<bool>, GrayImage)> {
    let whole = closed(polygon);
    let area = whole.area().abs();
    let notches = notches(polygon, NOTCH * area.sqrt());
    let mut chords: Vec<(f64, usize, usize)> = notches
        .iter()
        .enumerate()
        .flat_map(|(index, &start)| notches[index + 1..].iter().map(move |&end| (start, end)))
        .map(|(start, end)| {
            let side = closed(&polygon[start..=end]).area().abs();
            let length = polygon[start].distance(polygon[end]) / side.min(area - side).sqrt();
            (length, start, end)
        })
        .collect();
    chords.sort_by(|left, right| left.0.total_cmp(&right.0));
    chords.into_iter().find_map(|(_, start, end)| {
        let (from, to) = (polygon[start], polygon[end]);
        let steps = from.distance(to).ceil();
        if !(1..steps as usize).all(|step| whole.contains(from.lerp(to, step as f64 / steps))) {
            return None;
        }
        let mut side = GrayImage::new(width, height);
        let corners: Vec<_> = polygon[start..=end]
            .iter()
            .map(|point| {
                imageproc::point::Point::new(point.x.round() as i32, point.y.round() as i32)
            })
            .collect();
        draw_polygon_mut(&mut side, &corners, Luma([u8::MAX]));
        let between = held
            .iter()
            .map(|pixels| {
                let inside = pixels
                    .iter()
                    .filter(|&&(lx, ly)| side.get_pixel(lx, ly)[0] != 0)
                    .count();
                let share = inside as f64 / pixels.len().max(1) as f64;
                (share >= SIDE || share <= 1.0 - SIDE).then_some(share >= SIDE)
            })
            .collect::<Option<Vec<bool>>>()?;
        (between.contains(&true) && between.contains(&false)).then_some((start, end, between, side))
    })
}

/// The indices of `polygon`'s notches at least `depth` deep: in each
/// pocket between corners of its convex hull, the vertex farthest from the
/// hull, as OpenCV's `convexityDefects` finds them.
fn notches(polygon: &[Point], depth: f64) -> Vec<usize> {
    let hull = convex_hull(
        polygon
            .iter()
            .map(|point| imageproc::point::Point::new(point.x, point.y))
            .collect::<Vec<_>>(),
    );
    let mut corners: Vec<usize> = hull
        .iter()
        .filter_map(|corner| {
            polygon
                .iter()
                .position(|point| point.x == corner.x && point.y == corner.y)
        })
        .collect();
    corners.sort_unstable();
    let count = polygon.len();
    let mut found: Vec<usize> = corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .filter_map(|(&from, &to)| {
            let to = if to > from { to } else { to + count };
            let edge = polygon[to % count] - polygon[from];
            (from + 1..to)
                .map(|index| index % count)
                .map(|index| {
                    let offset = polygon[index] - polygon[from];
                    (index, edge.cross(offset).abs() / edge.hypot())
                })
                .max_by(|left, right| left.1.total_cmp(&right.1))
                .filter(|&(_, distance)| distance >= depth)
                .map(|(index, _)| index)
        })
        .collect();
    found.sort_unstable();
    found
}

#[cfg(test)]
mod tests {
    use kurbo::{BezPath, Point, Rect, Shape, Vec2};

    use super::{closed, rooms};

    /// The outline of overlapping ellipses, each a centre and radii, all
    /// holding `from`. A union of convex shapes holding a point is star
    /// shaped around it: each ray from it leaves where it leaves the farthest
    /// ellipse.
    fn ellipses(from: Point, ellipses: &[(Point, Vec2)]) -> BezPath {
        let points: Vec<Point> = (0..256)
            .map(|k| {
                let angle = std::f64::consts::TAU * f64::from(k) / 256.0;
                let direction = Vec2::new(angle.cos(), angle.sin());
                let reach = ellipses
                    .iter()
                    .map(|&(centre, radii)| {
                        // Scaled to a unit circle, the ray leaves it where
                        // |offset + t direction| = 1.
                        let scale =
                            |vector: Vec2| Vec2::new(vector.x / radii.x, vector.y / radii.y);
                        let (offset, direction) = (scale(from - centre), scale(direction));
                        let (a, b, c) = (
                            direction.hypot2(),
                            direction.dot(offset),
                            offset.hypot2() - 1.0,
                        );
                        (-b + (b * b - a * c).sqrt()) / a
                    })
                    .fold(0.0, f64::max);
                from + direction * reach
            })
            .collect();
        closed(&points)
    }

    #[test]
    fn conjoined_balloons_split_at_their_waist() {
        // Two circles of radius 60 at x = 50 and x = 150, overlapping into a
        // waist at x = 100.
        let outline = ellipses(
            Point::new(100.0, 100.0),
            &[
                (Point::new(50.0, 100.0), Vec2::new(60.0, 60.0)),
                (Point::new(150.0, 100.0), Vec2::new(60.0, 60.0)),
            ],
        );

        let found = rooms(
            &outline,
            &[
                vec![Rect::new(20.0, 80.0, 60.0, 120.0)],
                vec![Rect::new(140.0, 80.0, 180.0, 120.0)],
            ],
        );

        let [left_room, right_room] = [found[0].bounding_box(), found[1].bounding_box()];
        assert!((left_room.x1 - 100.0).abs() <= 3.0, "{left_room:?}");
        assert!((right_room.x0 - 100.0).abs() <= 3.0, "{right_room:?}");
    }

    #[test]
    fn a_small_lobe_keeps_its_text_lettered_by_the_waist() {
        // A tall small balloon overlapping a large one, its text at the top
        // by the waist and the large one's text beside it. The distance from
        // the edge keeps rising from the small lobe into the large one, so a
        // watershed alone floods the small lobe's lower half from the large
        // text.
        let outline = ellipses(
            Point::new(100.0, 80.0),
            &[
                (Point::new(82.0, 128.0), Vec2::new(45.0, 73.0)),
                (Point::new(118.0, 75.0), Vec2::new(28.0, 49.0)),
            ],
        );

        let found = rooms(
            &outline,
            &[
                vec![Rect::new(56.0, 56.0, 84.0, 176.0)],
                vec![Rect::new(100.0, 44.0, 124.0, 84.0)],
            ],
        );

        let lower = Point::new(134.0, 99.0);
        assert!(found[1].contains(lower) && !found[0].contains(lower));
    }
}
