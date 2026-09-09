//! MTSv3 test preprocessing and segmentation-map postprocessing.
//!
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/MTSv3/maskrcnn_benchmark/data/transforms/transforms.py#L30-L69
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/MTSv3/maskrcnn_benchmark/modeling/segmentation/inference.py#L100-L285

use anyhow::{Context, Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GrayImage, Luma, RgbImage};
use imageproc::contours::find_contours_with_threshold;
use koharu_torch::{Device, Tensor};
use serde::{Deserialize, Serialize};

use super::config::Config;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub polygon: Vec<[f32; 2]>,
    pub rotated_box: [[f32; 2]; 4],
    pub bounding_box: [f32; 4],
    pub score: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ImageSize {
    original_width: u32,
    original_height: u32,
    resized_width: u32,
    resized_height: u32,
}

#[derive(Debug)]
pub(super) struct Processor {
    config: Config,
}

impl Processor {
    pub(super) fn new(config: Config) -> Self {
        Self { config }
    }

    pub(super) fn preprocess(
        &self,
        image: &DynamicImage,
        device: Device,
    ) -> Result<(Tensor, ImageSize)> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "MTSv3 input image is empty"
        );
        let (resized_width, resized_height) = resize_dimensions(
            image.width(),
            image.height(),
            self.config.min_size,
            self.config.max_size,
        );
        self.preprocess_at_size(image, device, resized_width, resized_height)
    }

    fn preprocess_at_size(
        &self,
        image: &DynamicImage,
        device: Device,
        resized_width: u32,
        resized_height: u32,
    ) -> Result<(Tensor, ImageSize)> {
        let source = image.to_rgb8();
        let mut resized = RgbImage::new(resized_width, resized_height);
        Resizer::new()
            .resize(
                &source,
                &mut resized,
                &ResizeOptions::new()
                    .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                    .use_alpha(false),
            )
            .context("failed to resize MTSv3 input")?;

        let padded_width = resized_width.next_multiple_of(self.config.size_divisibility);
        let padded_height = resized_height.next_multiple_of(self.config.size_divisibility);
        let plane = (padded_width * padded_height) as usize;
        let mut values = vec![0.0_f32; plane * 3];
        for (x, y, pixel) in resized.enumerate_pixels() {
            let index = y as usize * padded_width as usize + x as usize;
            // torchvision converts RGB to BGR255 before subtracting the Caffe means.
            values[index] = f32::from(pixel[2]) - self.config.pixel_mean[0];
            values[plane + index] = f32::from(pixel[1]) - self.config.pixel_mean[1];
            values[plane * 2 + index] = f32::from(pixel[0]) - self.config.pixel_mean[2];
        }
        let tensor = Tensor::from_slice(&values)
            .view([1, 3, padded_height as i64, padded_width as i64])
            .to_device(device);
        Ok((
            tensor,
            ImageSize {
                original_width: image.width(),
                original_height: image.height(),
                resized_width,
                resized_height,
            },
        ))
    }

    pub(super) fn postprocess(
        &self,
        prediction: &Tensor,
        size: ImageSize,
    ) -> Result<Vec<Detection>> {
        ensure!(
            prediction.size().len() == 4 && prediction.size()[0] == 1 && prediction.size()[1] == 1,
            "MTSv3 expects a [1, 1, H, W] segmentation map, got {:?}",
            prediction.size()
        );
        let map = prediction
            .squeeze_dim(0)
            .squeeze_dim(0)
            .to_device(Device::Cpu);
        let rows: Vec<Vec<f32>> = Vec::<Vec<f32>>::try_from(&map)
            .context("failed to copy MTSv3 segmentation map to CPU")?;
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, Vec::len) as u32;
        let values = rows.into_iter().flatten().collect::<Vec<_>>();
        Ok(detections_from_bitmap(
            &values,
            width,
            height,
            size,
            &self.config,
        ))
    }
}

fn resize_dimensions(width: u32, height: u32, min_size: u32, max_size: u32) -> (u32, u32) {
    let minimum = width.min(height) as f64;
    let maximum = width.max(height) as f64;
    let mut target = min_size as f64;
    if maximum / minimum * target > max_size as f64 {
        target = (max_size as f64 * minimum / maximum).round();
    }
    if width < height {
        (
            target as u32,
            (target * height as f64 / width as f64) as u32,
        )
    } else {
        (
            (target * width as f64 / height as f64) as u32,
            target as u32,
        )
    }
}

fn detections_from_bitmap(
    prediction: &[f32],
    width: u32,
    height: u32,
    size: ImageSize,
    config: &Config,
) -> Vec<Detection> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    // The one-pixel background border reproduces OpenCV RETR_LIST edge behavior.
    let bitmap = GrayImage::from_fn(width + 2, height + 2, |x, y| {
        if x == 0 || y == 0 || x > width || y > height {
            return Luma([0]);
        }
        let value = prediction[(y as usize - 1) * width as usize + x as usize - 1];
        Luma([if value > config.binary_threshold {
            255
        } else {
            0
        }])
    });
    let x_scale = size.original_width as f32 / size.resized_width as f32;
    let y_scale = size.original_height as f32 / size.resized_height as f32;

    find_contours_with_threshold::<i32>(&bitmap, 0)
        .into_iter()
        .rev()
        .filter_map(|contour| {
            let contour = contour
                .points
                .into_iter()
                .map(|point| [(point.x - 1) as f32, (point.y - 1) as f32])
                .collect::<Vec<_>>();
            let (box_points, short_side) = mini_box(&contour)?;
            if short_side < config.minimum_size {
                return None;
            }
            let score = polygon_score(prediction, width, height, &box_points);
            if score < config.box_threshold {
                return None;
            }

            let approximated = approximate_closed_polygon(&contour, 0.01 * perimeter(&contour));
            if approximated.len() <= 2 {
                return None;
            }
            let mut polygon = unclip(&approximated, config.polygon_expand_ratio)?;
            let expanded_box = unclip(&box_points, config.box_expand_ratio)?;
            let (mut rotated_box, _) = mini_box(&expanded_box)?;

            for point in &mut polygon {
                point[0] = point[0].clamp(0.0, size.resized_width as f32) * x_scale;
                point[1] = point[1].clamp(0.0, size.resized_height as f32) * y_scale;
            }
            for point in &mut rotated_box {
                point[0] = point[0].round().clamp(0.0, size.resized_width as f32) * x_scale;
                point[1] = point[1].round().clamp(0.0, size.resized_height as f32) * y_scale;
            }
            let bounding_box = rotated_box.iter().fold(
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                |mut bounds, point| {
                    bounds[0] = bounds[0].min(point[0]);
                    bounds[1] = bounds[1].min(point[1]);
                    bounds[2] = bounds[2].max(point[0]);
                    bounds[3] = bounds[3].max(point[1]);
                    bounds
                },
            );
            Some(Detection {
                polygon,
                rotated_box,
                bounding_box,
                score,
            })
        })
        .take(config.top_n)
        .collect()
}

fn perimeter(points: &[[f32; 2]]) -> f32 {
    (0..points.len())
        .map(|index| {
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            (next[0] - current[0]).hypot(next[1] - current[1])
        })
        .sum()
}

fn approximate_closed_polygon(points: &[[f32; 2]], epsilon: f32) -> Vec<[f32; 2]> {
    if points.len() <= 3 || epsilon <= 0.0 {
        return points.to_vec();
    }
    let mut farthest = 1;
    let mut distance = 0.0;
    for index in 1..points.len() {
        let dx = points[index][0] - points[0][0];
        let dy = points[index][1] - points[0][1];
        let candidate = dx * dx + dy * dy;
        if candidate > distance {
            distance = candidate;
            farthest = index;
        }
    }
    let first = approximate_open_polyline(&points[..=farthest], epsilon);
    let mut second = points[farthest..].to_vec();
    second.push(points[0]);
    let second = approximate_open_polyline(&second, epsilon);
    first
        .into_iter()
        .chain(
            second
                .into_iter()
                .skip(1)
                .take_while(|point| *point != points[0]),
        )
        .collect()
}

fn approximate_open_polyline(points: &[[f32; 2]], epsilon: f32) -> Vec<[f32; 2]> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let start = points[0];
    let end = points[points.len() - 1];
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = dx.hypot(dy);
    let mut farthest = 0;
    let mut maximum = 0.0;
    for (index, point) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let distance = if length <= f32::EPSILON {
            (point[0] - start[0]).hypot(point[1] - start[1])
        } else {
            (dy * point[0] - dx * point[1] + end[0] * start[1] - end[1] * start[0]).abs() / length
        };
        if distance > maximum {
            maximum = distance;
            farthest = index;
        }
    }
    if maximum <= epsilon {
        return vec![start, end];
    }
    let mut left = approximate_open_polyline(&points[..=farthest], epsilon);
    left.pop();
    left.extend(approximate_open_polyline(&points[farthest..], epsilon));
    left
}

#[derive(Clone, Copy)]
struct RotatedRect {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    cos: f32,
    sin: f32,
}

impl RotatedRect {
    fn corners(self) -> [[f32; 2]; 4] {
        [
            [self.min_x, self.min_y],
            [self.max_x, self.min_y],
            [self.max_x, self.max_y],
            [self.min_x, self.max_y],
        ]
        .map(|[x, y]| [x * self.cos - y * self.sin, x * self.sin + y * self.cos])
    }
}

fn mini_box(points: &[[f32; 2]]) -> Option<([[f32; 2]; 4], f32)> {
    let rect = minimum_area_rect(points)?;
    let mut points = rect.corners().to_vec();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
    let (index_1, index_4) = if points[1][1] > points[0][1] {
        (0, 1)
    } else {
        (1, 0)
    };
    let (index_2, index_3) = if points[3][1] > points[2][1] {
        (2, 3)
    } else {
        (3, 2)
    };
    Some((
        [
            points[index_1],
            points[index_2],
            points[index_3],
            points[index_4],
        ],
        (rect.max_x - rect.min_x).min(rect.max_y - rect.min_y),
    ))
}

fn minimum_area_rect(points: &[[f32; 2]]) -> Option<RotatedRect> {
    let hull = convex_hull(points);
    if hull.len() < 3 {
        return None;
    }
    let mut best = None;
    let mut best_area = f32::INFINITY;
    for index in 0..hull.len() {
        let a = hull[index];
        let b = hull[(index + 1) % hull.len()];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            continue;
        }
        let cos = dx / length;
        let sin = dy / length;
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for point in &hull {
            let x = point[0] * cos + point[1] * sin;
            let y = -point[0] * sin + point[1] * cos;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let area = (max_x - min_x) * (max_y - min_y);
        if area < best_area {
            best_area = area;
            best = Some(RotatedRect {
                min_x,
                min_y,
                max_x,
                max_y,
                cos,
                sin,
            });
        }
    }
    best
}

fn convex_hull(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut points = points.to_vec();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() <= 2 {
        return points;
    }
    let mut lower = Vec::new();
    for &point in &points {
        while lower.len() >= 2
            && cross(lower[lower.len() - 2], lower[lower.len() - 1], point) <= 0.0
        {
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper = Vec::new();
    for &point in points.iter().rev() {
        while upper.len() >= 2
            && cross(upper[upper.len() - 2], upper[upper.len() - 1], point) <= 0.0
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

fn cross(origin: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - origin[0]) * (b[1] - origin[1]) - (a[1] - origin[1]) * (b[0] - origin[0])
}

fn polygon_score(map: &[f32], width: u32, height: u32, polygon: &[[f32; 2]]) -> f32 {
    let polygon = polygon
        .iter()
        .map(|point| [point[0] as i32, point[1] as i32])
        .collect::<Vec<_>>();
    let min_x = polygon
        .iter()
        .map(|point| point[0])
        .min()
        .unwrap_or_default()
        .clamp(0, width.saturating_sub(1) as i32);
    let max_x = polygon
        .iter()
        .map(|point| point[0])
        .max()
        .unwrap_or_default()
        .clamp(0, width.saturating_sub(1) as i32);
    let min_y = polygon
        .iter()
        .map(|point| point[1])
        .min()
        .unwrap_or_default()
        .clamp(0, height.saturating_sub(1) as i32);
    let max_y = polygon
        .iter()
        .map(|point| point[1])
        .max()
        .unwrap_or_default()
        .clamp(0, height.saturating_sub(1) as i32);
    if min_x > max_x || min_y > max_y {
        return 0.0;
    }
    let local = polygon
        .iter()
        .map(|point| [point[0] - min_x, point[1] - min_y])
        .collect::<Vec<_>>();
    let mask_width = (max_x - min_x + 1) as usize;
    let mask_height = (max_y - min_y + 1) as usize;
    let mask = fill_polygon(mask_width, mask_height, &local);
    let mut sum = 0.0_f64;
    let mut count = 0;
    for y in 0..mask_height {
        for x in 0..mask_width {
            if mask[y * mask_width + x] {
                sum += f64::from(map[(min_y as usize + y) * width as usize + min_x as usize + x]);
                count += 1;
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    }
}

#[derive(Clone, Copy)]
struct PolygonEdge {
    y0: i32,
    y1: i32,
    x: i64,
    dx: i64,
}

// OpenCV's integer fillPoly scan conversion is used by box_score_fast.
// https://github.com/opencv/opencv/blob/4.13.0/modules/imgproc/src/drawing.cpp#L1262-L1493
fn fill_polygon(width: usize, height: usize, polygon: &[[i32; 2]]) -> Vec<bool> {
    const XY_SHIFT: u32 = 16;
    const XY_ONE: i64 = 1 << XY_SHIFT;

    let mut mask = vec![false; width * height];
    if polygon.is_empty() {
        return mask;
    }

    let mut edges = Vec::with_capacity(polygon.len());
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        draw_line_8_connected(&mut mask, width, height, previous, current);
        if previous[1] != current[1] {
            let dx =
                ((current[0] - previous[0]) as i64 * XY_ONE) / (current[1] - previous[1]) as i64;
            let (y0, y1, x) = if previous[1] < current[1] {
                (previous[1], current[1], previous[0] as i64 * XY_ONE)
            } else {
                (current[1], previous[1], current[0] as i64 * XY_ONE)
            };
            edges.push(PolygonEdge { y0, y1, x, dx });
        }
        previous = current;
    }
    if edges.len() < 2 {
        return mask;
    }

    edges.sort_by_key(|edge| (edge.y0, edge.x, edge.dx));
    let y_min = edges.iter().map(|edge| edge.y0).min().unwrap_or_default();
    let y_max = edges.iter().map(|edge| edge.y1).max().unwrap_or_default();
    let mut active = Vec::<PolygonEdge>::new();
    for y in y_min..y_max.min(height as i32) {
        active.retain(|edge| edge.y1 != y);
        active.extend(edges.iter().filter(|edge| edge.y0 == y).copied());
        active.sort_by_key(|edge| edge.x);
        for pair in active.as_chunks::<2>().0 {
            let left = pair[0].x.min(pair[1].x);
            let right = pair[0].x.max(pair[1].x);
            let x1 = (left + XY_ONE - 1) >> XY_SHIFT;
            let x2 = right >> XY_SHIFT;
            if y >= 0 && x1 < width as i64 && x2 >= 0 {
                let x1 = x1.max(0) as usize;
                let x2 = x2.min(width as i64 - 1) as usize;
                for x in x1..=x2 {
                    mask[y as usize * width + x] = true;
                }
            }
        }
        for edge in &mut active {
            edge.x += edge.dx;
        }
    }
    mask
}

fn draw_line_8_connected(
    mask: &mut [bool],
    width: usize,
    height: usize,
    [mut x0, mut y0]: [i32; 2],
    [x1, y1]: [i32; 2],
) {
    let mut delta_x = 1;
    let mut delta_y = 1;
    let mut dx = x1 - x0;
    let mut dy = y1 - y0;
    if dx < 0 {
        dx = -dx;
        dy = -dy;
        x0 = x1;
        y0 = y1;
    }
    if dy < 0 {
        dy = -dy;
        delta_y = -1;
    }
    let vertical = dy > dx;
    if vertical {
        std::mem::swap(&mut dx, &mut dy);
        std::mem::swap(&mut delta_x, &mut delta_y);
    }
    let mut error = dx - 2 * dy;
    let plus_delta = 2 * dx;
    let minus_delta = -2 * dy;
    let mut minus_x = delta_x;
    let mut plus_x = 0;
    let mut minus_y = 0;
    let mut plus_y = delta_y;
    if vertical {
        std::mem::swap(&mut plus_y, &mut plus_x);
        std::mem::swap(&mut minus_y, &mut minus_x);
    }
    for _ in 0..=dx {
        if x0 >= 0 && y0 >= 0 && x0 < width as i32 && y0 < height as i32 {
            mask[y0 as usize * width + x0 as usize] = true;
        }
        let take_plus = error < 0;
        error += minus_delta + if take_plus { plus_delta } else { 0 };
        x0 += minus_x + if take_plus { plus_x } else { 0 };
        y0 += minus_y + if take_plus { plus_y } else { 0 };
    }
}

fn unclip(polygon: &[[f32; 2]], ratio: f32) -> Option<Vec<[f32; 2]>> {
    if polygon.len() < 3 {
        return None;
    }
    let mut twice_area = 0.0;
    let mut perimeter = 0.0;
    for index in 0..polygon.len() {
        let current = polygon[index];
        let next = polygon[(index + 1) % polygon.len()];
        twice_area += current[0] * next[1] - current[1] * next[0];
        perimeter += (next[0] - current[0]).hypot(next[1] - current[1]);
    }
    if perimeter <= f32::EPSILON {
        return None;
    }
    let distance = twice_area.abs() * 0.5 * ratio / perimeter;
    let counter_clockwise = twice_area > 0.0;
    let mut directions = Vec::with_capacity(polygon.len());
    let mut normals = Vec::with_capacity(polygon.len());
    let mut shifted = Vec::with_capacity(polygon.len());
    for index in 0..polygon.len() {
        let current = polygon[index];
        let next = polygon[(index + 1) % polygon.len()];
        let edge = [next[0] - current[0], next[1] - current[1]];
        let length = edge[0].hypot(edge[1]).max(1e-6);
        let direction = [edge[0] / length, edge[1] / length];
        let normal = if counter_clockwise {
            [direction[1], -direction[0]]
        } else {
            [-direction[1], direction[0]]
        };
        directions.push(direction);
        normals.push(normal);
        shifted.push([
            current[0] + distance * normal[0],
            current[1] + distance * normal[1],
        ]);
    }
    let mut result = Vec::with_capacity(polygon.len());
    for index in 0..polygon.len() {
        let previous = (index + polygon.len() - 1) % polygon.len();
        let previous_direction = directions[previous];
        let direction = directions[index];
        let denominator =
            previous_direction[0] * direction[1] - previous_direction[1] * direction[0];
        if denominator.abs() < 1e-6 {
            result.push([
                polygon[index][0] + 0.5 * distance * (normals[previous][0] + normals[index][0]),
                polygon[index][1] + 0.5 * distance * (normals[previous][1] + normals[index][1]),
            ]);
            continue;
        }
        let vector = [
            shifted[index][0] - shifted[previous][0],
            shifted[index][1] - shifted[previous][1],
        ];
        let parameter = (vector[0] * direction[1] - vector[1] * direction[0]) / denominator;
        result.push([
            shifted[previous][0] + previous_direction[0] * parameter,
            shifted[previous][1] + previous_direction[1] * parameter,
        ]);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resize_matches_upstream_integer_rules() {
        assert_eq!(resize_dimensions(280, 53, 1440, 4000), (3999, 757));
        assert_eq!(resize_dimensions(2000, 3000, 1440, 4000), (1440, 2160));
    }

    #[test]
    fn unclip_expands_rectangle() {
        let polygon = [[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]];
        let expanded = unclip(&polygon, 1.5).unwrap();
        let (box_points, _) = mini_box(&expanded).unwrap();
        assert!(box_points[2][0] - box_points[0][0] > 10.0);
    }
}
