//! Bubble-aware layout bounds derived from explicit scene relations.

use koharu_scene::Geometry;

const MAX_CONTOUR_POINTS: usize = 1_024;

type Point = (f32, f32);
type Polygon = Vec<Point>;
type IndexedPolygon = (usize, Polygon);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeometryFrame {
    pub bounds: LayoutBox,
    pub angle_degrees: f32,
}

pub(crate) fn contour(geometry: &Geometry, frame: GeometryFrame) -> Vec<(f32, f32)> {
    if geometry.points.len() > MAX_CONTOUR_POINTS {
        return Vec::new();
    }
    let bounds = frame.bounds;
    let center_x = bounds.x + bounds.width * 0.5;
    let center_y = bounds.y + bounds.height * 0.5;
    let (sin, cos) = (-frame.angle_degrees.to_radians()).sin_cos();
    geometry
        .points
        .iter()
        .map(|point| {
            let x = point.x as f32 - center_x;
            let y = point.y as f32 - center_y;
            (
                x * cos - y * sin + center_x - bounds.x,
                x * sin + y * cos + center_y - bounds.y,
            )
        })
        .collect()
}

pub(crate) fn point_in_frame(frame: GeometryFrame, x: f32, y: f32) -> (f32, f32) {
    let bounds = frame.bounds;
    let center_x = bounds.x + bounds.width * 0.5;
    let center_y = bounds.y + bounds.height * 0.5;
    let (sin, cos) = (-frame.angle_degrees.to_radians()).sin_cos();
    let x = x - center_x;
    let y = y - center_y;
    (
        x * cos - y * sin + center_x - bounds.x,
        x * sin + y * cos + center_y - bounds.y,
    )
}

/// Divides a joined balloon into one non-overlapping polygon per text flow.
///
/// Reflex-vertex diagonals recover the physical necks between lobes first. Source
/// anchors assign those lobes to flows and only become the partition itself when
/// the contour has no complete, non-crossing neck decomposition.
pub(crate) fn flow_cells(
    frame: GeometryFrame,
    contour: &[(f32, f32)],
    anchors: &[(f32, f32)],
) -> Vec<Vec<(f32, f32)>> {
    if anchors.is_empty() {
        return Vec::new();
    }
    let width = frame.bounds.width;
    let height = frame.bounds.height;
    let anchors = anchors
        .iter()
        .map(|&(x, y)| point_in_frame(frame, x, y))
        .map(|(x, y)| (x.clamp(0.0, width), y.clamp(0.0, height)))
        .collect::<Vec<_>>();
    topological_flow_cells(contour, &anchors)
        .unwrap_or_else(|| anchor_flow_cells(width, height, contour, &anchors))
}

fn topological_flow_cells(
    contour: &[(f32, f32)],
    anchors: &[(f32, f32)],
) -> Option<Vec<Vec<(f32, f32)>>> {
    if contour.len() < 4
        || contour.len() > MAX_CONTOUR_POINTS
        || contour
            .iter()
            .chain(anchors)
            .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return None;
    }
    let (min_x, max_x, min_y, max_y) = polygon_bounds(contour)?;
    let scale = (max_x - min_x).min(max_y - min_y).max(1.0);
    let tolerance = scale * 0.0075;
    let indexed_anchors = anchors.iter().copied().enumerate().collect::<Vec<_>>();
    let mut cells = decompose_lobes(contour.to_vec(), indexed_anchors, tolerance)?;
    cells.sort_by_key(|(index, _)| *index);
    (cells.len() == anchors.len()).then(|| cells.into_iter().map(|(_, cell)| cell).collect())
}

#[derive(Clone)]
struct LobeSplit {
    structural_support: f32,
    length_squared: f32,
    first: Vec<(f32, f32)>,
    second: Vec<(f32, f32)>,
    first_anchors: Vec<(usize, (f32, f32))>,
    second_anchors: Vec<(usize, (f32, f32))>,
}

fn decompose_lobes(
    polygon: Vec<(f32, f32)>,
    anchors: Vec<(usize, (f32, f32))>,
    tolerance: f32,
) -> Option<Vec<IndexedPolygon>> {
    if anchors.len() == 1 {
        return Some(vec![(anchors[0].0, polygon)]);
    }
    let simplified = simplify_closed_indices(&polygon, tolerance);
    if simplified.len() < 4 {
        return None;
    }
    let orientation = polygon_area_from_indices(&polygon, &simplified).signum();
    if orientation == 0.0 {
        return None;
    }
    let hull = convex_hull(
        &simplified
            .iter()
            .map(|&index| polygon[index])
            .collect::<Vec<_>>(),
    );
    let minimum_reflex_cross = tolerance * tolerance * 0.05;
    let reflex = (0..simplified.len())
        .filter_map(|index| {
            let previous = polygon[simplified[(index + simplified.len() - 1) % simplified.len()]];
            let current = polygon[simplified[index]];
            let next = polygon[simplified[(index + 1) % simplified.len()]];
            let cross = turn(previous, current, next) * orientation;
            if cross >= -minimum_reflex_cross {
                return None;
            }
            let edge_product =
                (distance_squared(previous, current) * distance_squared(current, next)).sqrt();
            (edge_product > f32::EPSILON).then(|| {
                let corner_strength = -cross / edge_product;
                let recession = distance_to_polygon_boundary(current, &hull);
                (simplified[index], corner_strength * recession)
            })
        })
        .collect::<Vec<_>>();

    let mut candidates = Vec::new();
    for first_index in 0..reflex.len() {
        for second_index in first_index + 1..reflex.len() {
            let (first_vertex, first_support) = reflex[first_index];
            let (second_vertex, second_support) = reflex[second_index];
            if !valid_diagonal(&polygon, first_vertex, second_vertex, tolerance) {
                continue;
            }
            let (first, second) = split_polygon(&polygon, first_vertex, second_vertex);
            if first.len() < 3
                || second.len() < 3
                || polygon_area(&first).abs() <= tolerance * tolerance
                || polygon_area(&second).abs() <= tolerance * tolerance
            {
                continue;
            }
            let mut first_anchors = Vec::new();
            let mut second_anchors = Vec::new();
            let mut assigns_cleanly = true;
            let cross_epsilon = (tolerance * tolerance * 0.001).max(f32::EPSILON);
            for &(index, anchor) in &anchors {
                match (
                    point_in_polygon(&first, anchor, cross_epsilon),
                    point_in_polygon(&second, anchor, cross_epsilon),
                ) {
                    (true, false) => first_anchors.push((index, anchor)),
                    (false, true) => second_anchors.push((index, anchor)),
                    _ => {
                        assigns_cleanly = false;
                        break;
                    }
                }
            }
            if !assigns_cleanly || first_anchors.is_empty() || second_anchors.is_empty() {
                continue;
            }
            let length_squared = distance_squared(polygon[first_vertex], polygon[second_vertex]);
            if length_squared <= f32::EPSILON {
                continue;
            }
            candidates.push(LobeSplit {
                structural_support: first_support.min(second_support) / length_squared.sqrt(),
                length_squared,
                first,
                second,
                first_anchors,
                second_anchors,
            });
        }
    }
    // A structural neck connects sharp corners that are recessed from the bubble's
    // convex envelope. Recession makes outer-wall raster kinks contribute almost no
    // support without imposing a pixel- or scale-specific rejection threshold.
    candidates.sort_by(|left, right| {
        right
            .structural_support
            .total_cmp(&left.structural_support)
            .then_with(|| left.length_squared.total_cmp(&right.length_squared))
    });
    for candidate in candidates {
        let Some(mut first) = decompose_lobes(candidate.first, candidate.first_anchors, tolerance)
        else {
            continue;
        };
        let Some(second) = decompose_lobes(candidate.second, candidate.second_anchors, tolerance)
        else {
            continue;
        };
        first.extend(second);
        return Some(first);
    }
    None
}

fn convex_hull(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut points = points.to_vec();
    points.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
    });
    points.dedup();
    if points.len() <= 2 {
        return points;
    }

    let mut lower = Vec::new();
    for &point in &points {
        while lower.len() >= 2 && turn(lower[lower.len() - 2], lower[lower.len() - 1], point) <= 0.0
        {
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper = Vec::new();
    for &point in points.iter().rev() {
        while upper.len() >= 2 && turn(upper[upper.len() - 2], upper[upper.len() - 1], point) <= 0.0
        {
            upper.pop();
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn distance_to_polygon_boundary(point: (f32, f32), polygon: &[(f32, f32)]) -> f32 {
    if polygon.len() < 2 {
        return 0.0;
    }
    (0..polygon.len())
        .map(|index| {
            point_segment_distance(point, polygon[index], polygon[(index + 1) % polygon.len()])
        })
        .fold(f32::INFINITY, f32::min)
}

fn simplify_closed_indices(polygon: &[(f32, f32)], tolerance: f32) -> Vec<usize> {
    let split = (1..polygon.len())
        .max_by(|&left, &right| {
            distance_squared(polygon[0], polygon[left])
                .total_cmp(&distance_squared(polygon[0], polygon[right]))
        })
        .unwrap_or(0);
    if split == 0 {
        return (0..polygon.len()).collect();
    }
    let first_chain = (0..=split).collect::<Vec<_>>();
    let second_chain = (split..polygon.len())
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut first = simplify_chain_indices(polygon, &first_chain, tolerance);
    let mut second = simplify_chain_indices(polygon, &second_chain, tolerance);
    first.pop();
    second.pop();
    first.extend(second);
    if first.len() >= 3 {
        first
    } else {
        (0..polygon.len()).collect()
    }
}

fn simplify_chain_indices(polygon: &[(f32, f32)], chain: &[usize], tolerance: f32) -> Vec<usize> {
    if chain.len() <= 2 {
        return chain.to_vec();
    }
    let start = polygon[chain[0]];
    let end = polygon[*chain.last().unwrap()];
    let mut farthest = None;
    for (position, &index) in chain.iter().enumerate().skip(1).take(chain.len() - 2) {
        let distance = point_segment_distance(polygon[index], start, end);
        if farthest.is_none_or(|(_, best)| distance > best) {
            farthest = Some((position, distance));
        }
    }
    let Some((position, distance)) = farthest else {
        return vec![chain[0], *chain.last().unwrap()];
    };
    if distance <= tolerance {
        return vec![chain[0], *chain.last().unwrap()];
    }
    let mut first = simplify_chain_indices(polygon, &chain[..=position], tolerance);
    let second = simplify_chain_indices(polygon, &chain[position..], tolerance);
    first.pop();
    first.extend(second);
    first
}

fn valid_diagonal(polygon: &[(f32, f32)], first: usize, second: usize, tolerance: f32) -> bool {
    let len = polygon.len();
    if first == second || (first + 1) % len == second || (second + 1) % len == first {
        return false;
    }
    let start = polygon[first];
    let end = polygon[second];
    let epsilon = (tolerance * tolerance * 0.001).max(f32::EPSILON);
    for edge in 0..len {
        let next = (edge + 1) % len;
        if edge == first || edge == second || next == first || next == second {
            continue;
        }
        if segments_intersect(start, end, polygon[edge], polygon[next], epsilon) {
            return false;
        }
    }
    [0.2, 0.5, 0.8].into_iter().all(|fraction| {
        point_in_polygon(
            polygon,
            (
                start.0 + (end.0 - start.0) * fraction,
                start.1 + (end.1 - start.1) * fraction,
            ),
            epsilon,
        )
    })
}

fn split_polygon(polygon: &[(f32, f32)], first: usize, second: usize) -> (Polygon, Polygon) {
    let (first, second) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    let first_part = polygon[first..=second].to_vec();
    let second_part = polygon[second..]
        .iter()
        .chain(&polygon[..=first])
        .copied()
        .collect();
    (first_part, second_part)
}

fn anchor_flow_cells(
    width: f32,
    height: f32,
    contour: &[(f32, f32)],
    anchors: &[(f32, f32)],
) -> Vec<Vec<(f32, f32)>> {
    let scale = width.min(height).max(1.0);
    let coincidence_distance_squared = (scale * 0.0025).powi(2);
    let mut clusters = Vec::<((f32, f32), Vec<usize>)>::new();
    for (index, &anchor) in anchors.iter().enumerate() {
        if let Some((site, indices)) = clusters.iter_mut().find(|(site, _)| {
            let dx = anchor.0 - site.0;
            let dy = anchor.1 - site.1;
            dx * dx + dy * dy <= coincidence_distance_squared
        }) {
            let count = indices.len() as f32;
            site.0 = (site.0 * count + anchor.0) / (count + 1.0);
            site.1 = (site.1 * count + anchor.1) / (count + 1.0);
            indices.push(index);
        } else {
            clusters.push((anchor, vec![index]));
        }
    }

    let mut cells = vec![Vec::new(); anchors.len()];
    for (cluster_index, &((x, y), ref indices)) in clusters.iter().enumerate() {
        // The cell is the editable balloon shape, so fallback bisectors must
        // clip the physical contour rather than just its bounding rectangle.
        let mut cell = if contour.len() >= 3 {
            contour.to_vec()
        } else {
            vec![(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
        };
        for (other_index, &((other_x, other_y), _)) in clusters.iter().enumerate() {
            if cluster_index == other_index {
                continue;
            }
            let normal = (other_x - x, other_y - y);
            let offset = (other_x * other_x + other_y * other_y - x * x - y * y) * 0.5;
            cell = clip_half_plane(&cell, normal, offset);
            if cell.len() < 3 {
                break;
            }
        }
        if indices.len() == 1 {
            cells[indices[0]] = cell;
            continue;
        }
        let (min_x, max_x, min_y, max_y) = cell.iter().fold(
            (
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ),
            |(min_x, max_x, min_y, max_y), &(x, y)| {
                (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
            },
        );
        let horizontal = max_x - min_x >= max_y - min_y;
        let (minimum, maximum) = if horizontal {
            (min_x, max_x)
        } else {
            (min_y, max_y)
        };
        let step = (maximum - minimum) / indices.len() as f32;
        for (order, &index) in indices.iter().enumerate() {
            let lower = minimum + step * order as f32;
            let upper = if order + 1 == indices.len() {
                maximum
            } else {
                lower + step
            };
            let (lower_normal, upper_normal) = if horizontal {
                ((-1.0, 0.0), (1.0, 0.0))
            } else {
                ((0.0, -1.0), (0.0, 1.0))
            };
            let strip = clip_half_plane(&cell, lower_normal, -lower);
            cells[index] = clip_half_plane(&strip, upper_normal, upper);
        }
    }
    cells
}

fn polygon_bounds(polygon: &[(f32, f32)]) -> Option<(f32, f32, f32, f32)> {
    let &(first_x, first_y) = polygon.first()?;
    Some(polygon[1..].iter().fold(
        (first_x, first_x, first_y, first_y),
        |(min_x, max_x, min_y, max_y), &(x, y)| {
            (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
        },
    ))
}

fn polygon_area(polygon: &[(f32, f32)]) -> f32 {
    if polygon.len() < 3 {
        return 0.0;
    }
    (0..polygon.len())
        .map(|index| {
            let first = polygon[index];
            let second = polygon[(index + 1) % polygon.len()];
            first.0 * second.1 - second.0 * first.1
        })
        .sum::<f32>()
        * 0.5
}

fn polygon_area_from_indices(polygon: &[(f32, f32)], indices: &[usize]) -> f32 {
    (0..indices.len())
        .map(|index| {
            let first = polygon[indices[index]];
            let second = polygon[indices[(index + 1) % indices.len()]];
            first.0 * second.1 - second.0 * first.1
        })
        .sum::<f32>()
        * 0.5
}

fn point_in_polygon(polygon: &[(f32, f32)], point: (f32, f32), epsilon: f32) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let first = polygon[index];
        let second = polygon[(index + 1) % polygon.len()];
        if point_on_segment(first, second, point, epsilon) {
            return true;
        }
        if (first.1 > point.1) != (second.1 > point.1) {
            let crossing =
                (second.0 - first.0) * (point.1 - first.1) / (second.1 - first.1) + first.0;
            if point.0 < crossing {
                inside = !inside;
            }
        }
    }
    inside
}

fn segments_intersect(
    first_start: (f32, f32),
    first_end: (f32, f32),
    second_start: (f32, f32),
    second_end: (f32, f32),
    epsilon: f32,
) -> bool {
    let first_side_start = orientation(first_start, first_end, second_start);
    let first_side_end = orientation(first_start, first_end, second_end);
    let second_side_start = orientation(second_start, second_end, first_start);
    let second_side_end = orientation(second_start, second_end, first_end);
    let crosses = ((first_side_start > epsilon && first_side_end < -epsilon)
        || (first_side_start < -epsilon && first_side_end > epsilon))
        && ((second_side_start > epsilon && second_side_end < -epsilon)
            || (second_side_start < -epsilon && second_side_end > epsilon));
    crosses
        || point_on_segment(first_start, first_end, second_start, epsilon)
        || point_on_segment(first_start, first_end, second_end, epsilon)
        || point_on_segment(second_start, second_end, first_start, epsilon)
        || point_on_segment(second_start, second_end, first_end, epsilon)
}

fn point_on_segment(start: (f32, f32), end: (f32, f32), point: (f32, f32), epsilon: f32) -> bool {
    orientation(start, end, point).abs() <= epsilon
        && point.0 >= start.0.min(end.0) - epsilon
        && point.0 <= start.0.max(end.0) + epsilon
        && point.1 >= start.1.min(end.1) - epsilon
        && point.1 <= start.1.max(end.1) + epsilon
}

fn orientation(first: (f32, f32), second: (f32, f32), third: (f32, f32)) -> f32 {
    (second.0 - first.0) * (third.1 - first.1) - (second.1 - first.1) * (third.0 - first.0)
}

fn turn(previous: (f32, f32), current: (f32, f32), next: (f32, f32)) -> f32 {
    (current.0 - previous.0) * (next.1 - current.1)
        - (current.1 - previous.1) * (next.0 - current.0)
}

fn point_segment_distance(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let segment = (end.0 - start.0, end.1 - start.1);
    let length_squared = segment.0 * segment.0 + segment.1 * segment.1;
    if length_squared <= f32::EPSILON {
        return distance_squared(point, start).sqrt();
    }
    let fraction = (((point.0 - start.0) * segment.0 + (point.1 - start.1) * segment.1)
        / length_squared)
        .clamp(0.0, 1.0);
    distance_squared(
        point,
        (
            start.0 + segment.0 * fraction,
            start.1 + segment.1 * fraction,
        ),
    )
    .sqrt()
}

fn distance_squared(first: (f32, f32), second: (f32, f32)) -> f32 {
    (first.0 - second.0).powi(2) + (first.1 - second.1).powi(2)
}

fn clip_half_plane(polygon: &[(f32, f32)], normal: (f32, f32), offset: f32) -> Vec<(f32, f32)> {
    let Some(&last) = polygon.last() else {
        return Vec::new();
    };
    let signed = |point: (f32, f32)| point.0 * normal.0 + point.1 * normal.1 - offset;
    let mut output = Vec::new();
    let mut previous = last;
    let mut previous_distance = signed(previous);
    for &current in polygon {
        let current_distance = signed(current);
        let previous_inside = previous_distance <= f32::EPSILON;
        let current_inside = current_distance <= f32::EPSILON;
        if previous_inside != current_inside {
            let denominator = previous_distance - current_distance;
            if denominator.abs() > f32::EPSILON {
                let fraction = previous_distance / denominator;
                output.push((
                    previous.0 + (current.0 - previous.0) * fraction,
                    previous.1 + (current.1 - previous.1) * fraction,
                ));
            }
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_distance = current_distance;
    }
    output
}

pub(crate) fn geometry_bounds(geometry: &Geometry) -> Option<LayoutBox> {
    if geometry
        .points
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return None;
    }
    let first = geometry.points.first()?;
    let (mut min_x, mut min_y) = (first.x, first.y);
    let (mut max_x, mut max_y) = (first.x, first.y);
    for point in &geometry.points[1..] {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let x = min_x as f32;
    let y = min_y as f32;
    let width = (max_x - min_x) as f32;
    let height = (max_y - min_y) as f32;
    if !x.is_finite()
        || !y.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    Some(LayoutBox {
        x,
        y,
        width,
        height,
    })
}

pub(crate) fn geometry_frame(
    geometry: &Geometry,
    angle_degrees: Option<f32>,
) -> Option<GeometryFrame> {
    if let Some(angle_degrees) = angle_degrees {
        if !angle_degrees.is_finite() || geometry.points.is_empty() {
            return None;
        }
        // Measure the contour in the authored text axes. A polygon alone cannot
        // recover those axes after a rotation, even when its bounds are square.
        let (sin, cos) = f64::from(angle_degrees).to_radians().sin_cos();
        let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for point in &geometry.points {
            if !point.x.is_finite() || !point.y.is_finite() {
                return None;
            }
            let x = point.x * cos + point.y * sin;
            let y = -point.x * sin + point.y * cos;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let width = max_x - min_x;
        let height = max_y - min_y;
        let center_x = (min_x + max_x) * 0.5;
        let center_y = (min_y + max_y) * 0.5;
        let bounds = LayoutBox {
            x: (center_x * cos - center_y * sin - width * 0.5) as f32,
            y: (center_x * sin + center_y * cos - height * 0.5) as f32,
            width: width as f32,
            height: height as f32,
        };
        return ([bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(f32::is_finite)
            && bounds.width > 0.0
            && bounds.height > 0.0)
            .then_some(GeometryFrame {
                bounds,
                angle_degrees,
            });
    }
    let [top_left, top_right, bottom_right, bottom_left] = geometry.points.as_slice() else {
        return geometry_bounds(geometry).map(|bounds| GeometryFrame {
            bounds,
            angle_degrees: 0.0,
        });
    };
    let top = (top_right.x - top_left.x, top_right.y - top_left.y);
    let right = (bottom_right.x - top_right.x, bottom_right.y - top_right.y);
    let bottom = (
        bottom_left.x - bottom_right.x,
        bottom_left.y - bottom_right.y,
    );
    let left = (top_left.x - bottom_left.x, top_left.y - bottom_left.y);
    let width = top.0.hypot(top.1);
    let height = right.0.hypot(right.1);
    if !width.is_finite() || !height.is_finite() || width <= f64::EPSILON || height <= f64::EPSILON
    {
        return None;
    }

    let scale = width.max(height).max(1.0);
    let length_tolerance = scale * 1e-6;
    let opposite_lengths_match = (bottom.0.hypot(bottom.1) - width).abs() <= length_tolerance
        && (left.0.hypot(left.1) - height).abs() <= length_tolerance;
    let perpendicular = (top.0 * right.0 + top.1 * right.1).abs() <= width * height * 1e-6;
    let diagonals_bisect = ((top_left.x + bottom_right.x) - (top_right.x + bottom_left.x)).abs()
        <= length_tolerance
        && ((top_left.y + bottom_right.y) - (top_right.y + bottom_left.y)).abs()
            <= length_tolerance;
    if !opposite_lengths_match || !perpendicular || !diagonals_bisect {
        return geometry_bounds(geometry).map(|bounds| GeometryFrame {
            bounds,
            angle_degrees: 0.0,
        });
    }

    let center_x = (top_left.x + top_right.x + bottom_right.x + bottom_left.x) * 0.25;
    let center_y = (top_left.y + top_right.y + bottom_right.y + bottom_left.y) * 0.25;
    Some(GeometryFrame {
        bounds: LayoutBox {
            x: (center_x - width * 0.5) as f32,
            y: (center_y - height * 0.5) as f32,
            width: width as f32,
            height: height as f32,
        },
        angle_degrees: top.1.atan2(top.0).to_degrees() as f32,
    })
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Geometry, Origin, Point};

    use super::*;

    #[test]
    fn polygon_bounds_use_all_points() {
        let geometry = Geometry {
            origin: Origin::User,
            points: vec![
                Point { x: 20.0, y: 30.0 },
                Point { x: 80.0, y: 30.0 },
                Point { x: 70.0, y: 90.0 },
                Point { x: 20.0, y: 80.0 },
            ],
        };

        assert_eq!(
            geometry_bounds(&geometry),
            Some(LayoutBox {
                x: 20.0,
                y: 30.0,
                width: 60.0,
                height: 60.0,
            })
        );
    }

    #[test]
    fn rotated_rectangle_preserves_layout_dimensions_and_angle() {
        let (sin, cos) = 27.0_f64.to_radians().sin_cos();
        let geometry = Geometry {
            origin: Origin::User,
            points: [(-40.0, -15.0), (40.0, -15.0), (40.0, 15.0), (-40.0, 15.0)]
                .map(|(x, y)| Point {
                    x: 100.0 + x * cos - y * sin,
                    y: 80.0 + x * sin + y * cos,
                })
                .into(),
        };

        let frame = geometry_frame(&geometry, None).unwrap();
        assert!((frame.bounds.x - 60.0).abs() < 1e-4);
        assert!((frame.bounds.y - 65.0).abs() < 1e-4);
        assert!((frame.bounds.width - 80.0).abs() < 1e-4);
        assert!((frame.bounds.height - 30.0).abs() < 1e-4);
        assert!((frame.angle_degrees - 27.0).abs() < 1e-4);

        let mut contour = geometry;
        contour.points.insert(1, Point { x: 100.0, y: 80.0 });
        assert_eq!(geometry_frame(&contour, None).unwrap().angle_degrees, 0.0);
        let authored = geometry_frame(&contour, Some(27.0)).unwrap();
        assert!((authored.bounds.x - frame.bounds.x).abs() < 1e-4);
        assert!((authored.bounds.y - frame.bounds.y).abs() < 1e-4);
        assert!((authored.bounds.width - frame.bounds.width).abs() < 1e-4);
        assert!((authored.bounds.height - frame.bounds.height).abs() < 1e-4);
        assert_eq!(authored.angle_degrees, 27.0);
    }

    #[test]
    fn flow_cells_split_the_frame_between_source_anchors() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (0.0, 80.0)];
        let cells = flow_cells(frame, &contour, &[(25.0, 40.0), (75.0, 40.0)]);
        assert_eq!(cells.len(), 2);
        let first_right = cells[0]
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::NEG_INFINITY, f32::max);
        let second_left = cells[1]
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::INFINITY, f32::min);
        assert!((first_right - 50.0).abs() < 2.0);
        assert!((first_right - second_left).abs() < 1e-4);
    }

    #[test]
    fn flow_cells_fallback_preserves_the_physical_contour() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            angle_degrees: 0.0,
        };
        let contour = [(0.0, 0.0), (100.0, 0.0), (50.0, 100.0)];
        let cells = flow_cells(frame, &contour, &[(25.0, 25.0), (75.0, 25.0)]);
        assert_eq!(cells.len(), 2);
        for cell in &cells {
            assert!(
                cell.iter()
                    .all(|&point| point_in_polygon(&contour, point, 1e-4))
            );
            assert!((polygon_area(cell).abs() - 2_500.0).abs() < 1e-4);
        }
        assert!(cells[0].iter().all(|&(x, _)| x <= 50.0));
        assert!(cells[1].iter().all(|&(x, _)| x >= 50.0));
    }

    #[test]
    fn flow_cells_cut_the_contour_neck_before_using_anchor_bisectors() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![
            (0.0, 20.0),
            (40.0, 20.0),
            (50.0, 30.0),
            (60.0, 20.0),
            (100.0, 20.0),
            (100.0, 80.0),
            (60.0, 80.0),
            (50.0, 70.0),
            (40.0, 80.0),
            (0.0, 80.0),
        ];

        let cells = flow_cells(frame, &contour, &[(15.0, 50.0), (65.0, 50.0)]);

        assert!(contains_edge(&cells[0], (50.0, 30.0), (50.0, 70.0)));
        assert!(contains_edge(&cells[1], (50.0, 30.0), (50.0, 70.0)));
        assert!(point_in_polygon(&cells[0], (15.0, 50.0), 0.001));
        assert!(point_in_polygon(&cells[1], (65.0, 50.0), 0.001));
        // The anchor bisector is x=40; x=50 proves that the physical neck won.
        assert!(!contains_edge(&cells[0], (40.0, 0.0), (40.0, 100.0)));
    }

    #[test]
    fn flow_cells_ignore_edge_noise_when_splitting_overlapping_boxes() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 167.0,
                height: 311.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![
            (163.0, 0.0),
            (162.0, 1.0),
            (71.0, 1.0),
            (70.0, 2.0),
            (62.0, 1.0),
            (60.0, 2.0),
            (59.0, 3.0),
            (59.0, 56.0),
            (60.0, 57.0),
            (59.0, 100.0),
            (56.0, 102.0),
            (17.0, 101.0),
            (9.0, 103.0),
            (3.0, 102.0),
            (1.0, 104.0),
            (0.0, 107.0),
            (1.0, 110.0),
            (0.0, 182.0),
            (1.0, 183.0),
            (2.0, 234.0),
            (1.0, 235.0),
            (0.0, 282.0),
            (2.0, 288.0),
            (2.0, 309.0),
            (7.0, 311.0),
            (46.0, 311.0),
            (47.0, 310.0),
            (89.0, 311.0),
            (93.0, 308.0),
            (92.0, 300.0),
            (93.0, 294.0),
            (92.0, 239.0),
            (93.0, 237.0),
            (95.0, 235.0),
            (164.0, 235.0),
            (166.0, 233.0),
            (165.0, 136.0),
            (167.0, 117.0),
            (167.0, 72.0),
            (166.0, 71.0),
            (167.0, 23.0),
            (166.0, 1.0),
        ];

        let cells = flow_cells(frame, &contour, &[(114.0, 115.0), (47.0, 207.0)]);

        assert!(contains_edge(&cells[0], (59.0, 100.0), (95.0, 235.0)));
        assert!(contains_edge(&cells[1], (59.0, 100.0), (95.0, 235.0)));
        assert!(!contains_edge(&cells[0], (59.0, 100.0), (165.0, 136.0)));
    }

    #[test]
    fn flow_cells_keep_the_short_neck_between_rounded_lobes() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 91.0,
                height: 181.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![
            (62.0, 0.0),
            (61.0, 1.0),
            (50.0, 1.0),
            (49.0, 2.0),
            (47.0, 2.0),
            (41.0, 5.0),
            (37.0, 10.0),
            (37.0, 18.0),
            (36.0, 19.0),
            (36.0, 38.0),
            (35.0, 39.0),
            (35.0, 51.0),
            (32.0, 54.0),
            (30.0, 54.0),
            (29.0, 55.0),
            (25.0, 55.0),
            (24.0, 56.0),
            (17.0, 56.0),
            (16.0, 57.0),
            (13.0, 57.0),
            (10.0, 59.0),
            (6.0, 60.0),
            (4.0, 62.0),
            (2.0, 66.0),
            (2.0, 68.0),
            (1.0, 69.0),
            (1.0, 80.0),
            (0.0, 81.0),
            (0.0, 105.0),
            (1.0, 106.0),
            (1.0, 134.0),
            (2.0, 135.0),
            (2.0, 144.0),
            (3.0, 145.0),
            (3.0, 153.0),
            (4.0, 154.0),
            (4.0, 161.0),
            (5.0, 162.0),
            (5.0, 166.0),
            (6.0, 167.0),
            (6.0, 169.0),
            (7.0, 171.0),
            (12.0, 176.0),
            (14.0, 177.0),
            (20.0, 178.0),
            (21.0, 179.0),
            (24.0, 179.0),
            (25.0, 180.0),
            (39.0, 180.0),
            (40.0, 181.0),
            (47.0, 181.0),
            (48.0, 180.0),
            (50.0, 180.0),
            (54.0, 178.0),
            (59.0, 173.0),
            (60.0, 170.0),
            (63.0, 166.0),
            (64.0, 160.0),
            (65.0, 159.0),
            (65.0, 156.0),
            (66.0, 155.0),
            (66.0, 143.0),
            (68.0, 141.0),
            (71.0, 141.0),
            (72.0, 140.0),
            (78.0, 140.0),
            (81.0, 138.0),
            (85.0, 137.0),
            (87.0, 135.0),
            (89.0, 131.0),
            (89.0, 126.0),
            (90.0, 125.0),
            (90.0, 115.0),
            (91.0, 114.0),
            (91.0, 51.0),
            (90.0, 50.0),
            (90.0, 18.0),
            (89.0, 17.0),
            (89.0, 13.0),
            (88.0, 12.0),
            (88.0, 9.0),
            (87.0, 7.0),
            (83.0, 3.0),
            (81.0, 2.0),
            (77.0, 2.0),
            (76.0, 1.0),
        ];

        let cells = flow_cells(frame, &contour, &[(65.0, 71.0), (29.0, 118.0)]);

        assert!(contains_edge(&cells[0], (32.0, 54.0), (66.0, 143.0)));
        assert!(contains_edge(&cells[1], (32.0, 54.0), (66.0, 143.0)));
    }

    #[test]
    fn flow_cells_use_the_overlap_junctions_of_uneven_rounded_lobes() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 186.0,
                height: 313.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![
            (123.0, 0.0),
            (111.0, 6.0),
            (101.0, 15.0),
            (97.0, 17.0),
            (83.0, 31.0),
            (75.0, 42.0),
            (72.0, 47.0),
            (67.0, 65.0),
            (65.0, 90.0),
            (63.0, 95.0),
            (59.0, 98.0),
            (45.0, 101.0),
            (37.0, 105.0),
            (23.0, 115.0),
            (15.0, 123.0),
            (9.0, 132.0),
            (3.0, 146.0),
            (1.0, 157.0),
            (1.0, 170.0),
            (0.0, 171.0),
            (1.0, 203.0),
            (9.0, 254.0),
            (17.0, 277.0),
            (28.0, 296.0),
            (36.0, 303.0),
            (43.0, 306.0),
            (50.0, 313.0),
            (59.0, 307.0),
            (72.0, 304.0),
            (79.0, 299.0),
            (89.0, 284.0),
            (92.0, 275.0),
            (92.0, 271.0),
            (94.0, 265.0),
            (96.0, 263.0),
            (102.0, 262.0),
            (111.0, 266.0),
            (124.0, 269.0),
            (136.0, 269.0),
            (149.0, 263.0),
            (154.0, 258.0),
            (161.0, 247.0),
            (168.0, 229.0),
            (169.0, 219.0),
            (178.0, 179.0),
            (181.0, 157.0),
            (181.0, 148.0),
            (182.0, 147.0),
            (182.0, 138.0),
            (183.0, 137.0),
            (183.0, 128.0),
            (184.0, 127.0),
            (184.0, 118.0),
            (185.0, 117.0),
            (186.0, 83.0),
            (185.0, 82.0),
            (185.0, 71.0),
            (183.0, 58.0),
            (180.0, 46.0),
            (172.0, 28.0),
            (161.0, 16.0),
            (150.0, 7.0),
            (139.0, 1.0),
        ];

        let cells = flow_cells(frame, &contour, &[(126.25, 135.4353), (50.0, 202.05078)]);

        assert!(contains_edge(&cells[0], (63.0, 95.0), (96.0, 263.0)));
        assert!(contains_edge(&cells[1], (63.0, 95.0), (96.0, 263.0)));
    }

    #[test]
    fn flow_cells_recursively_decompose_a_three_lobe_contour() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 140.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![
            (0.0, 0.0),
            (40.0, 0.0),
            (40.0, 20.0),
            (60.0, 20.0),
            (60.0, 0.0),
            (100.0, 0.0),
            (100.0, 60.0),
            (75.0, 60.0),
            (75.0, 80.0),
            (90.0, 80.0),
            (90.0, 140.0),
            (50.0, 140.0),
            (50.0, 80.0),
            (65.0, 80.0),
            (65.0, 60.0),
            (60.0, 60.0),
            (60.0, 40.0),
            (40.0, 40.0),
            (40.0, 60.0),
            (0.0, 60.0),
        ];
        let anchors = [(20.0, 30.0), (80.0, 30.0), (70.0, 110.0)];

        let cells = flow_cells(frame, &contour, &anchors);

        assert_eq!(cells.len(), 3);
        for (index, &anchor) in anchors.iter().enumerate() {
            assert!(point_in_polygon(&cells[index], anchor, 0.001));
            assert!(
                anchors
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .all(|(_, &other)| !point_in_polygon(&cells[index], other, 0.001))
            );
        }
        assert!(
            (cells
                .iter()
                .map(|cell| polygon_area(cell).abs())
                .sum::<f32>()
                - polygon_area(&contour).abs())
            .abs()
                < 0.01
        );
    }

    #[test]
    fn flow_cells_disambiguate_coincident_source_anchors() {
        let frame = GeometryFrame {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            angle_degrees: 0.0,
        };
        let contour = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (0.0, 80.0)];
        let cells = flow_cells(frame, &contour, &[(50.0, 40.0), (50.0, 40.0), (50.0, 40.0)]);
        assert_eq!(cells.len(), 3);
        assert!(cells.iter().all(|cell| cell.len() >= 3));
        let bounds = cells
            .iter()
            .map(|cell| {
                cell.iter().fold(
                    (f32::INFINITY, f32::NEG_INFINITY),
                    |(minimum, maximum), &(x, _)| (minimum.min(x), maximum.max(x)),
                )
            })
            .collect::<Vec<_>>();
        assert!(bounds[0].1 <= bounds[1].0 + 1e-4);
        assert!(bounds[1].1 <= bounds[2].0 + 1e-4);
    }

    fn contains_edge(polygon: &[(f32, f32)], first: (f32, f32), second: (f32, f32)) -> bool {
        (0..polygon.len()).any(|index| {
            let start = polygon[index];
            let end = polygon[(index + 1) % polygon.len()];
            (start == first && end == second) || (start == second && end == first)
        })
    }
}
