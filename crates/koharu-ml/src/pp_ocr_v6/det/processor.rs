//! Transformers-compatible PP-OCRv6 detection preprocessing and DB decoding.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/pp_ocrv5_server_det/image_processing_pp_ocrv5_server_det.py

use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImageView, GrayImage, Luma};
use imageproc::contours::find_contours_with_threshold;
use koharu_torch::{Device, Kind, Tensor};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PPOCRV6MediumDetImageProcessor {
    pub do_convert_rgb: bool,
    pub do_normalize: bool,
    pub do_rescale: bool,
    pub do_resize: bool,
    pub image_mean: Vec<f32>,
    pub image_std: Vec<f32>,
    pub rescale_factor: f64,
    pub resample: i64,
    pub limit_side_len: i64,
    pub limit_type: String,
    pub max_side_limit: i64,
}

impl Default for PPOCRV6MediumDetImageProcessor {
    fn default() -> Self {
        Self {
            do_convert_rgb: true,
            do_normalize: true,
            do_rescale: true,
            do_resize: true,
            image_mean: vec![0.406, 0.456, 0.485],
            image_std: vec![0.225, 0.224, 0.229],
            rescale_factor: 1.0 / 255.0,
            resample: 2,
            limit_side_len: 736,
            limit_type: "min".into(),
            max_side_limit: 4000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextDetection {
    pub polygon: [[f32; 2]; 4],
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextDetections {
    pub detections: Vec<TextDetection>,
}

impl PPOCRV6MediumDetImageProcessor {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        serde_json::from_str(&std::fs::read_to_string(path)?)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        if !self.do_convert_rgb && image.color().channel_count() < 3 {
            bail!("PP-OCRv6 detection requires at least three input channels");
        }
        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();
        let mut pixel_values = Tensor::from_slice(rgb.as_raw())
            .view([1, height as i64, width as i64, 3])
            .permute([0, 3, 1, 2])
            .to_kind(Kind::Float)
            .to_device(device);

        if self.do_resize {
            let [target_height, target_width] = self.image_size(height as i64, width as i64)?;
            pixel_values = match self.resample {
                0 => pixel_values.upsample_nearest2d(
                    [target_height, target_width],
                    None::<f64>,
                    None::<f64>,
                ),
                2 => pixel_values.internal_upsample_bilinear2d_aa(
                    [target_height, target_width],
                    false,
                    None::<f64>,
                    None::<f64>,
                ),
                resample => bail!("unsupported PP-OCRv6 detection resampling mode {resample}"),
            };
        }
        if self.do_rescale {
            pixel_values *= self.rescale_factor;
        }
        if self.do_normalize {
            if self.image_mean.len() != 3 || self.image_std.len() != 3 {
                bail!("PP-OCRv6 detection image_mean and image_std must contain three values");
            }
            let mean = Tensor::from_slice(&self.image_mean)
                .view([1, 3, 1, 1])
                .to_device(device);
            let std = Tensor::from_slice(&self.image_std)
                .view([1, 3, 1, 1])
                .to_device(device);
            pixel_values = (pixel_values - mean) / std;
        }
        // This channel reversal is performed after normalization in the Transformers processor.
        pixel_values = pixel_values.flip([1]);
        Ok(pixel_values)
    }

    pub(crate) fn postprocess(
        &self,
        output: &Tensor,
        image: &DynamicImage,
    ) -> Result<TextDetections> {
        self.postprocess_with_thresholds(output, image, 0.3, 0.6, 1.5, 3, 1000)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn postprocess_with_thresholds(
        &self,
        output: &Tensor,
        image: &DynamicImage,
        threshold: f32,
        box_threshold: f32,
        unclip_ratio: f32,
        min_size: i32,
        max_candidates: usize,
    ) -> Result<TextDetections> {
        let size = output.size();
        if size.len() != 4 || size[0] != 1 || size[1] != 1 {
            bail!("expected PP-OCRv6 detection output [1, 1, H, W], got {size:?}");
        }
        let map_width = size[3] as u32;
        let map_height = size[2] as u32;
        let values = Vec::<f32>::try_from(
            &output
                .to_device(Device::Cpu)
                .to_kind(Kind::Float)
                .contiguous()
                .view([-1]),
        )?;
        let (dest_width, dest_height) = image.dimensions();
        Ok(TextDetections {
            detections: boxes_from_bitmap(
                &values,
                map_width,
                map_height,
                dest_width,
                dest_height,
                threshold,
                box_threshold,
                unclip_ratio,
                min_size,
                max_candidates,
            ),
        })
    }

    fn image_size(&self, height: i64, width: i64) -> Result<[i64; 2]> {
        let mut ratio = match self.limit_type.as_str() {
            "max" if height.max(width) > self.limit_side_len => {
                self.limit_side_len as f64 / height.max(width) as f64
            }
            "min" if height.min(width) < self.limit_side_len => {
                self.limit_side_len as f64 / height.min(width) as f64
            }
            "resize_long" => self.limit_side_len as f64 / height.max(width) as f64,
            "max" | "min" => 1.0,
            other => bail!("unsupported PP-OCRv6 detection limit type {other:?}"),
        };
        let mut resize_height = (height as f64 * ratio) as i64;
        let mut resize_width = (width as f64 * ratio) as i64;
        if resize_height.max(resize_width) > self.max_side_limit {
            ratio = self.max_side_limit as f64 / resize_height.max(resize_width) as f64;
            resize_height = (resize_height as f64 * ratio) as i64;
            resize_width = (resize_width as f64 * ratio) as i64;
        }
        // Python's `round` uses ties-to-even, including for exact half multiples of 32.
        resize_height = ((resize_height as f64 / 32.0).round_ties_even() as i64 * 32).max(32);
        resize_width = ((resize_width as f64 / 32.0).round_ties_even() as i64 * 32).max(32);
        Ok([resize_height, resize_width])
    }
}

#[allow(clippy::too_many_arguments)]
fn boxes_from_bitmap(
    prediction: &[f32],
    width: u32,
    height: u32,
    dest_width: u32,
    dest_height: u32,
    threshold: f32,
    box_threshold: f32,
    unclip_ratio: f32,
    min_size: i32,
    max_candidates: usize,
) -> Vec<TextDetection> {
    // Padding reproduces OpenCV's treatment of pixels outside the image as background.
    let bitmap = GrayImage::from_fn(width + 2, height + 2, |x, y| {
        if x == 0 || y == 0 || x > width || y > height {
            return Luma([0]);
        }
        let value = prediction[(y as usize - 1) * width as usize + x as usize - 1];
        Luma([if value > threshold { 255 } else { 0 }])
    });
    let width_scale = dest_width as f32 / width as f32;
    let height_scale = dest_height as f32 / height as f32;

    find_contours_with_threshold::<i32>(&bitmap, 0)
        .into_iter()
        // OpenCV's RETR_LIST traversal exposes contours in the reverse of the
        // top-to-bottom border-discovery order used by imageproc.
        .rev()
        .take(max_candidates)
        .filter_map(|contour| {
            let points = contour
                .points
                .into_iter()
                .map(|point| [(point.x - 1) as f32, (point.y - 1) as f32])
                .collect::<Vec<_>>();
            let (box_points, short_side) = mini_box(&points)?;
            if short_side < min_size as f32 {
                return None;
            }
            let score = polygon_score(prediction, width, height, &box_points);
            if score < box_threshold {
                return None;
            }
            let unclipped = unclip(&box_points, unclip_ratio)?;
            let (mut polygon, short_side) = mini_box(&unclipped)?;
            if short_side < (min_size + 2) as f32 {
                return None;
            }
            for point in &mut polygon {
                point[0] = (point[0] * width_scale)
                    .round()
                    .clamp(0.0, dest_width as f32);
                point[1] = (point[1] * height_scale)
                    .round()
                    .clamp(0.0, dest_height as f32);
            }
            Some(TextDetection { polygon, score })
        })
        .collect()
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
    // Transformers passes an int32 copy of the rotated box to cv2.fillPoly.
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

    let mask_width = (max_x - min_x + 1) as usize;
    let mask_height = (max_y - min_y + 1) as usize;
    let local_polygon = polygon
        .iter()
        .map(|point| [point[0] - min_x, point[1] - min_y])
        .collect::<Vec<_>>();
    let mask = opencv_fill_polygon(mask_width, mask_height, &local_polygon);
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for y in 0..mask_height {
        for x in 0..mask_width {
            if mask[y * mask_width + x] {
                let map_x = min_x as usize + x;
                let map_y = min_y as usize + y;
                sum += map[map_y * width as usize + map_x] as f64;
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

// Port of OpenCV's integer fillPoly scan conversion. OpenCV first draws every
// polygon edge and then fills inclusive spans between fixed-point edge pairs.
// https://github.com/opencv/opencv/blob/4.13.0/modules/imgproc/src/drawing.cpp#L1262-L1493
fn opencv_fill_polygon(width: usize, height: usize, polygon: &[[i32; 2]]) -> Vec<bool> {
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
    // cv::Line uses LineIterator with 8-connectivity and leftToRight=true.
    // https://github.com/opencv/opencv/blob/4.13.0/modules/imgproc/src/drawing.cpp#L159-L260
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
        let cross_product =
            previous_direction[0] * direction[1] - previous_direction[1] * direction[0];
        if cross_product.abs() < 1e-6 {
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
        let parameter = (vector[0] * direction[1] - vector[1] * direction[0]) / cross_product;
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
    fn detector_resize_matches_transformers_rounding() {
        let processor = PPOCRV6MediumDetImageProcessor::default();
        assert_eq!(processor.image_size(480, 640).unwrap(), [736, 992]);
        assert_eq!(processor.image_size(2000, 4000).unwrap(), [1984, 4000]);
    }

    #[test]
    fn unclip_expands_a_rectangle() {
        let polygon = [[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [0.0, 4.0]];
        let expanded = unclip(&polygon, 1.5).unwrap();
        assert!(expanded[0][0] < 0.0 && expanded[0][1] < 0.0);
        assert!(expanded[2][0] > 10.0 && expanded[2][1] > 4.0);
    }

    #[test]
    fn rotated_polygon_score_matches_opencv_fill_poly() {
        let map = (0..100).map(|value| value as f32).collect::<Vec<_>>();
        let polygon = [[2.7, 1.2], [7.9, 2.1], [6.4, 7.8], [1.1, 6.3]];
        assert_eq!(polygon_score(&map, 10, 10, &polygon), 44.0);
    }

    #[test]
    fn contour_order_matches_opencv_retr_list() {
        let mut prediction = vec![0.0; 64 * 64];
        for y in 5..=15 {
            for x in 5..=20 {
                prediction[y * 64 + x] = 1.0;
            }
        }
        for y in 40..=55 {
            for x in 30..=50 {
                prediction[y * 64 + x] = 1.0;
            }
        }
        let detections = boxes_from_bitmap(&prediction, 64, 64, 64, 64, 0.3, 0.6, 1.5, 3, 100);
        assert_eq!(detections.len(), 2);
        assert!(detections[0].polygon[0][1] > detections[1].polygon[0][1]);
    }
}
