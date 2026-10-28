//! BallonsTranslator-compatible preprocessing, rearranged inference, and DBNet decoding.
//!
//! Original implementations:
//! - https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/textdetector/ctd/inference.py
//! - https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/textdetector/db_utils.py
//! - https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/textdetector/ctd/textmask.py
//! - https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/utils/textblock.py
//! - https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/utils/imgproc_utils.py

use anyhow::{Context, Result, anyhow, bail};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GenericImageView, GrayImage, Luma, RgbImage, imageops::crop_imm};
use imageproc::{contours::find_contours_with_threshold, contrast::otsu_level, point::Point};
use koharu_torch::{Device, Kind, Tensor};
use rayon::prelude::*;
use serde::Serialize;

use super::model::Output;

const DBNET_BINARY_THRESHOLD: f32 = 0.3;
const LINE_SCORE_THRESHOLD: f32 = 0.6;
const MAX_LINE_CANDIDATES: usize = 1000;
const LINE_UNCLIP_RATIO: f32 = 1.5;
const MASK_SCORE_THRESHOLD: f32 = 0.1;

pub type Quad = [[i32; 2]; 4];
type ModelQuad = [[f32; 2]; 4];

#[derive(Debug, Clone, Serialize)]
pub struct TextBlock {
    pub xyxy: [i32; 4],
    pub lines: Vec<Quad>,
    pub language: String,
    pub vertical: bool,
    pub angle: i32,
    pub detected_font_size: f32,
}

pub fn preprocess(image: &DynamicImage, device: Device) -> Result<(Tensor, [u32; 4])> {
    let (original_width, original_height) = image.dimensions();
    if original_width == 0 || original_height == 0 {
        bail!("empty image");
    }

    let scale = (1280.0 / original_width as f64).min(1280.0 / original_height as f64);
    let resized_width = ((original_width as f64 * scale).round_ties_even() as u32).max(1);
    let resized_height = ((original_height as f64 * scale).round_ties_even() as u32).max(1);
    // `preprocess_img` letterboxes uint8 pixels with linear interpolation,
    // pads only the bottom/right edges, then converts RGB HWC to float CHW.
    // https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/textdetector/ctd/inference.py#L206-L220
    let source = image.to_rgb8();
    let mut resized = RgbImage::new(resized_width, resized_height);
    Resizer::new()
        .resize(
            &source,
            &mut resized,
            &ResizeOptions::new().resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear)),
        )
        .map_err(|error| anyhow!("failed to resize comic text detector input: {error}"))?;
    let mut letterboxed = RgbImage::new(1280, 1280);
    image::imageops::replace(&mut letterboxed, &resized, 0, 0);
    let pixel_values = image_to_tensor(&DynamicImage::ImageRgb8(letterboxed), device)?;

    Ok((
        pixel_values,
        [
            original_width,
            original_height,
            resized_width,
            resized_height,
        ],
    ))
}

pub fn postprocess(
    outputs: Output,
    dimensions: [u32; 4],
    source: &DynamicImage,
) -> Result<(GrayImage, Vec<TextBlock>)> {
    let [
        original_width,
        original_height,
        resized_width,
        resized_height,
    ] = dimensions;
    let maps = Tensor::cat(&[outputs.mask, outputs.line_maps], 1)?
        .narrow(2, 0, resized_height as i64)?
        .narrow(3, 0, resized_width as i64)?;
    // Upstream converts the cropped maps to uint8 before resizing the mask
    // back to the caller's dimensions.
    let values = tensor_to_u8_vec(maps.narrow(1, 0, 1)?.contiguous()?.view([-1])?)?;
    let resized_plane = resized_width as usize * resized_height as usize;
    let map = gray_from_slice(resized_width, resized_height, &values[..resized_plane])?;
    let mut raw_mask = GrayImage::new(original_width, original_height);
    Resizer::new()
        .resize(
            &map,
            &mut raw_mask,
            &ResizeOptions::new().resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear)),
        )
        .map_err(|error| anyhow!("failed to resize comic text detector map: {error}"))?;
    let shrink = tensor_to_f32_vec(maps.narrow(1, 1, 1)?.contiguous()?.view([-1])?)?;

    let lines = extract_line_polygons(
        &shrink,
        resized_width,
        resized_height,
        original_width,
        original_height,
    );
    Ok(finalize_detection(source, raw_mask, lines))
}

pub fn rearranged_inference<F>(
    source: &DynamicImage,
    device: Device,
    forward: F,
) -> Result<Option<(GrayImage, Vec<TextBlock>)>>
where
    F: Fn(&Tensor) -> Result<Output>,
{
    // https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/textdetector/ctd/inference.py#L23-L150
    let (source_width, source_height) = source.dimensions();
    if source_width == 0 || source_height == 0 {
        bail!("empty image");
    }

    let transposed = source_height < source_width;
    let (width, height) = if transposed {
        (source_height, source_width)
    } else {
        (source_width, source_height)
    };
    let aspect_ratio = height as f32 / width as f32;
    let downscale_ratio = height as f32 / 1280.0;
    if downscale_ratio <= 2.5 || aspect_ratio <= 3.0 {
        return Ok(None);
    }

    let oriented = if transposed {
        let source = source.to_rgb8();
        RgbImage::from_fn(width, height, |x, y| *source.get_pixel(y, x))
    } else {
        source.to_rgb8()
    };
    let strips_per_composite = ((2 * 1280) / width).max(2) as usize;
    let patch_height = width
        .checked_mul(strips_per_composite as u32)
        .context("comic text detector rearranged patch is too large")?;
    let patch_count = height.div_ceil(patch_height) as usize;
    let patch_step = if patch_count > 1 {
        (height - patch_height) / (patch_count as u32 - 1)
    } else {
        0
    };
    let patch_starts = (0..patch_count)
        .map(|index| index as u32 * patch_step)
        .collect::<Vec<_>>();
    let composite_count = patch_count.div_ceil(strips_per_composite);
    let pad_count = composite_count * strips_per_composite - patch_count;
    let mut composite_maps = Vec::with_capacity(composite_count);
    let mut map_size = 0u32;

    for first_composite in (0..composite_count).step_by(4) {
        let batch_len = 4.min(composite_count - first_composite);
        let mut tensors = Vec::with_capacity(batch_len);
        let mut padding = Vec::with_capacity(batch_len);
        for offset in 0..batch_len {
            let composite_index = first_composite + offset;
            let square_size = patch_height.max(1280);
            let mut composite = RgbImage::new(square_size, square_size);
            for strip_index in 0..strips_per_composite {
                let patch_index = composite_index * strips_per_composite + strip_index;
                if patch_index >= patch_count {
                    break;
                }
                let top = patch_starts[patch_index];
                if transposed {
                    for y in 0..patch_height {
                        for x in 0..width {
                            composite.put_pixel(
                                y,
                                strip_index as u32 * width + x,
                                *oriented.get_pixel(x, top + y),
                            );
                        }
                    }
                } else {
                    let patch = crop_imm(&oriented, 0, top, width, patch_height).to_image();
                    image::imageops::replace(
                        &mut composite,
                        &patch,
                        strip_index as i64 * width as i64,
                        0,
                    );
                }
            }
            let mut resized = RgbImage::new(1280, 1280);
            if square_size == 1280 {
                resized = composite;
            } else {
                Resizer::new()
                    .resize(
                        &composite,
                        &mut resized,
                        &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Box)),
                    )
                    .map_err(|error| {
                        anyhow!("failed to resize rearranged comic text batch: {error}")
                    })?;
            }
            tensors.push(image_to_tensor(&DynamicImage::ImageRgb8(resized), device)?);
            padding.push(1280u32.saturating_sub(patch_height));
        }
        let batch = Tensor::cat(&tensors, 0)?;
        let outputs = forward(&batch)?;
        let maps = Tensor::cat(&[outputs.mask, outputs.line_maps], 1)?;
        for (local_index, &pad) in padding.iter().enumerate() {
            let mut map = maps.narrow(0, local_index as i64, 1)?;
            if pad > 0 {
                let output_pad = map.size()[3] as u32 * pad / 1280;
                let keep = map.size()[3] - output_pad as i64;
                map = map.narrow(2, 0, keep)?.narrow(3, 0, keep)?;
            }
            map_size = map.size()[3] as u32;
            composite_maps.push(tensor_to_f32_vec(map.contiguous()?.view([-1])?)?);
        }
    }

    let output_step = patch_step * map_size / patch_height;
    let strip_width = map_size / strips_per_composite as u32;
    let output_height = strip_width * height / width;
    let mut restored = vec![0.0f32; 3 * output_height as usize * strip_width as usize];
    let restored_plane = output_height as usize * strip_width as usize;
    let composite_plane = map_size as usize * map_size as usize;
    let patch_total = composite_maps.len() * strips_per_composite - pad_count;
    for (composite_index, map) in composite_maps.iter().enumerate() {
        for strip_index in 0..strips_per_composite {
            let patch_index = composite_index * strips_per_composite + strip_index;
            if patch_index >= patch_total {
                break;
            }
            let relative_top = patch_starts[patch_index] as f64 / height as f64;
            let top = (relative_top * output_height as f64).round_ties_even() as u32;
            let bottom = (top + map_size).min(output_height);
            let left = strip_index as u32 * strip_width;
            for channel in 0..3usize {
                for y in 0..bottom - top {
                    for x in 0..strip_width {
                        let source_index = if transposed {
                            channel * composite_plane
                                + (left + x) as usize * map_size as usize
                                + y as usize
                        } else {
                            channel * composite_plane
                                + y as usize * map_size as usize
                                + (left + x) as usize
                        };
                        let target_index = channel * restored_plane
                            + (top + y) as usize * strip_width as usize
                            + x as usize;
                        restored[target_index] += map[source_index];
                    }
                }
            }
            if patch_index > 0 {
                let overlap = map_size - output_step;
                for channel in 0..3usize {
                    for y in top..(top + overlap).min(output_height) {
                        for x in 0..strip_width {
                            let index = channel * restored_plane
                                + y as usize * strip_width as usize
                                + x as usize;
                            restored[index] /= 2.0;
                        }
                    }
                }
            }
        }
    }

    let (map_width, map_height, maps) = if transposed {
        let mut maps = vec![0.0f32; restored.len()];
        for channel in 0..3usize {
            for y in 0..output_height {
                for x in 0..strip_width {
                    maps[channel * restored_plane
                        + x as usize * output_height as usize
                        + y as usize] = restored
                        [channel * restored_plane + y as usize * strip_width as usize + x as usize];
                }
            }
        }
        (output_height, strip_width, maps)
    } else {
        (strip_width, output_height, restored)
    };
    let map_plane = map_width as usize * map_height as usize;
    let map = GrayImage::from_fn(map_width, map_height, |x, y| {
        let value = maps[y as usize * map_width as usize + x as usize];
        Luma([(value.clamp(0.0, 1.0) * 255.0) as u8])
    });
    let mut mask = GrayImage::new(source_width, source_height);
    Resizer::new()
        .resize(
            &map,
            &mut mask,
            &ResizeOptions::new().resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear)),
        )
        .map_err(|error| anyhow!("failed to restore rearranged comic text map: {error}"))?;
    let lines = extract_line_polygons(
        &maps[map_plane..2 * map_plane],
        map_width,
        map_height,
        source_width,
        source_height,
    );
    Ok(Some(finalize_detection(source, mask, lines)))
}

fn finalize_detection(
    source: &DynamicImage,
    raw_mask: GrayImage,
    lines: Vec<ModelQuad>,
) -> (GrayImage, Vec<TextBlock>) {
    let blocks = group_text_lines(&lines, &raw_mask);
    let refined = refine_mask(source, &raw_mask, &blocks);
    let mask = dilate_binary(&refined, 2, MorphShape::Ellipse);
    (mask, blocks)
}

fn image_to_tensor(image: &DynamicImage, device: Device) -> Result<Tensor> {
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();
    Ok((Tensor::from_slice(rgb.as_raw())?
        .view([height as i64, width as i64, 3])?
        .permute([2, 0, 1])?
        .unsqueeze(0)?
        .to_device(device)?
        .to_kind(Kind::Float)?
        / 255.0)?)
}

fn tensor_to_u8_vec(tensor: Tensor) -> Result<Vec<u8>> {
    let tensor = (tensor.clamp(0.0, 1.0)? * 255.0)?;
    let tensor = tensor
        .to_kind(Kind::Uint8)?
        .to_device(Device::Cpu)?
        .contiguous()?
        .view([-1])?;
    Ok(Vec::<u8>::try_from(&tensor)?)
}

fn tensor_to_f32_vec(tensor: Tensor) -> Result<Vec<f32>> {
    let tensor = tensor
        .to_kind(Kind::Float)?
        .to_device(Device::Cpu)?
        .contiguous()?
        .view([-1])?;
    Ok(Vec::<f32>::try_from(&tensor)?)
}

fn gray_from_slice(width: u32, height: u32, pixels: &[u8]) -> Result<GrayImage> {
    GrayImage::from_raw(width, height, pixels.to_vec())
        .context("failed to create gray image from comic text detector tensor")
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
    fn width(self) -> f32 {
        self.max_x - self.min_x
    }

    fn height(self) -> f32 {
        self.max_y - self.min_y
    }

    fn corners(self, expand: f32) -> ModelQuad {
        let points = [
            [self.min_x - expand, self.min_y - expand],
            [self.max_x + expand, self.min_y - expand],
            [self.max_x + expand, self.max_y + expand],
            [self.min_x - expand, self.max_y + expand],
        ];
        points.map(|[x, y]| [x * self.cos - y * self.sin, x * self.sin + y * self.cos])
    }
}

fn extract_line_polygons(
    map: &[f32],
    map_width: u32,
    map_height: u32,
    dest_width: u32,
    dest_height: u32,
) -> Vec<ModelQuad> {
    let bitmap = GrayImage::from_fn(map_width, map_height, |x, y| {
        Luma([
            if map[y as usize * map_width as usize + x as usize] > DBNET_BINARY_THRESHOLD {
                255
            } else {
                0
            },
        ])
    });
    find_contours_with_threshold::<i32>(&bitmap, 0)
        .into_iter()
        .take(MAX_LINE_CANDIDATES)
        .filter_map(|contour| {
            let points = contour.points;
            if points.len() < 3 {
                return None;
            }
            let rect = minimum_area_rect(&points)?;
            if rect.width().min(rect.height()) < 2.0 {
                return None;
            }
            let score = polygon_score(map, map_width, map_height, &points);
            if score <= LINE_SCORE_THRESHOLD {
                return None;
            }
            let perimeter = 2.0 * (rect.width() + rect.height());
            let expand = if perimeter > f32::EPSILON {
                rect.width() * rect.height() * LINE_UNCLIP_RATIO / perimeter
            } else {
                0.0
            };
            let mut quad = rect.corners(expand);
            for point in &mut quad {
                point[0] = (point[0] / map_width as f32 * dest_width as f32)
                    .round()
                    .clamp(0.0, dest_width as f32);
                point[1] = (point[1] / map_height as f32 * dest_height as f32)
                    .round()
                    .clamp(0.0, dest_height as f32);
            }
            Some(quad)
        })
        .collect()
}

fn minimum_area_rect(points: &[Point<i32>]) -> Option<RotatedRect> {
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
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
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

fn convex_hull(points: &[Point<i32>]) -> Vec<[f32; 2]> {
    let mut points = points
        .iter()
        .map(|point| [point.x as f32, point.y as f32])
        .collect::<Vec<_>>();
    points.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() <= 2 {
        return points;
    }

    let mut lower = Vec::new();
    for point in &points {
        while lower.len() >= 2
            && cross(lower[lower.len() - 2], lower[lower.len() - 1], *point) <= 0.0
        {
            lower.pop();
        }
        lower.push(*point);
    }
    let mut upper = Vec::new();
    for point in points.iter().rev() {
        while upper.len() >= 2
            && cross(upper[upper.len() - 2], upper[upper.len() - 1], *point) <= 0.0
        {
            upper.pop();
        }
        upper.push(*point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn cross(origin: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - origin[0]) * (b[1] - origin[1]) - (a[1] - origin[1]) * (b[0] - origin[0])
}

fn polygon_score(map: &[f32], map_width: u32, map_height: u32, polygon: &[Point<i32>]) -> f32 {
    let min_x = polygon
        .iter()
        .map(|point| point.x)
        .min()
        .unwrap_or(0)
        .max(0) as u32;
    let min_y = polygon
        .iter()
        .map(|point| point.y)
        .min()
        .unwrap_or(0)
        .max(0) as u32;
    let max_x = polygon
        .iter()
        .map(|point| point.x)
        .max()
        .unwrap_or(0)
        .clamp(0, map_width.saturating_sub(1) as i32) as u32;
    let max_y = polygon
        .iter()
        .map(|point| point.y)
        .max()
        .unwrap_or(0)
        .clamp(0, map_height.saturating_sub(1) as i32) as u32;
    let polygon = polygon
        .iter()
        .map(|point| [point.x as f32, point.y as f32])
        .collect::<Vec<_>>();
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if point_in_polygon([x as f32, y as f32], &polygon) {
                sum += map[y as usize * map_width as usize + x as usize] as f64;
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

fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        if point_on_segment(point, previous, current) {
            return true;
        }
        if (current[1] > point[1]) != (previous[1] > point[1])
            && point[0]
                < (previous[0] - current[0]) * (point[1] - current[1]) / (previous[1] - current[1])
                    + current[0]
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn point_on_segment(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> bool {
    cross(a, b, point).abs() < 1e-3
        && point[0] >= a[0].min(b[0])
        && point[0] <= a[0].max(b[0])
        && point[1] >= a[1].min(b[1])
        && point[1] <= a[1].max(b[1])
}

#[derive(Clone)]
struct WorkingBlock {
    bbox: [f32; 4],
    lines: Vec<ModelQuad>,
    vertical: bool,
    angle: f32,
    font_size: f32,
    vector: [f32; 2],
    norm: f32,
    merged: bool,
}

impl WorkingBlock {
    fn from_line(quad: ModelQuad) -> Self {
        let (quad, vertical) = sort_line_quad(quad);
        let mut block = Self {
            bbox: quad_bbox(&quad),
            lines: vec![quad],
            vertical,
            angle: 0.0,
            font_size: 0.0,
            vector: [0.0, 0.0],
            norm: 0.0,
            merged: false,
        };
        block.recalculate();
        block
    }

    fn center(&self) -> [f32; 2] {
        [
            (self.bbox[0] + self.bbox[2]) * 0.5,
            (self.bbox[1] + self.bbox[3]) * 0.5,
        ]
    }

    fn recalculate(&mut self) {
        self.bbox = lines_bbox(&self.lines);
        let mut vertical_vector = [0.0f32; 2];
        let mut horizontal_vector = [0.0f32; 2];
        for line in &self.lines {
            let top = midpoint(line[0], line[1]);
            let right = midpoint(line[1], line[2]);
            let bottom = midpoint(line[2], line[3]);
            let left = midpoint(line[3], line[0]);
            vertical_vector[0] += bottom[0] - top[0];
            vertical_vector[1] += bottom[1] - top[1];
            horizontal_vector[0] += right[0] - left[0];
            horizontal_vector[1] += right[1] - left[1];
        }
        let vertical_norm = vector_norm(vertical_vector);
        let horizontal_norm = vector_norm(horizontal_vector);
        if self.vertical {
            self.vector = vertical_vector;
            self.norm = vertical_norm;
            self.font_size = (horizontal_norm / self.lines.len() as f32).round();
            self.angle = vertical_vector[1]
                .atan2(vertical_vector[0])
                .to_degrees()
                .trunc()
                - 90.0;
        } else {
            self.vector = horizontal_vector;
            self.norm = horizontal_norm;
            self.font_size = (vertical_norm / self.lines.len() as f32).round();
            self.angle = horizontal_vector[1]
                .atan2(horizontal_vector[0])
                .to_degrees()
                .trunc();
        }
        if self.angle.abs() < 3.0 {
            self.angle = 0.0;
        }
    }

    fn into_public(self) -> TextBlock {
        TextBlock {
            xyxy: self.bbox.map(|coordinate| coordinate as i32),
            lines: self
                .lines
                .into_iter()
                .map(|line| line.map(|point| point.map(|coordinate| coordinate as i32)))
                .collect(),
            language: "unknown".to_string(),
            vertical: self.vertical,
            angle: self.angle as i32,
            detected_font_size: self.font_size,
        }
    }
}

fn group_text_lines(lines: &[ModelQuad], mask: &GrayImage) -> Vec<TextBlock> {
    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();
    for &line in lines {
        let bbox = quad_bbox(&line);
        if mean_mask(mask, bbox) < MASK_SCORE_THRESHOLD {
            continue;
        }
        let block = WorkingBlock::from_line(line);
        if block.vertical {
            vertical.push(block);
        } else {
            horizontal.push(block);
        }
    }
    horizontal.sort_unstable_by(|a, b| a.center()[1].total_cmp(&b.center()[1]));
    vertical.sort_unstable_by(|a, b| b.center()[0].total_cmp(&a.center()[0]));

    let mut grouped = merge_text_lines(horizontal, 2.0);
    grouped.extend(merge_text_lines(vertical, 1.7));
    let mut grouped = sort_regions(grouped);
    let mut kept: Vec<WorkingBlock> = Vec::with_capacity(grouped.len());
    for block in grouped.drain(..) {
        let area = bbox_area(block.bbox).max(f32::EPSILON);
        let contained = kept
            .iter()
            .any(|existing| bbox_intersection(block.bbox, existing.bbox) / area > 0.9);
        if !contained {
            kept.push(block);
        }
    }
    kept.into_iter().map(WorkingBlock::into_public).collect()
}

fn merge_text_lines(mut blocks: Vec<WorkingBlock>, font_size_tolerance: f32) -> Vec<WorkingBlock> {
    if blocks.len() < 2 {
        return blocks;
    }
    let mut merged = Vec::new();
    for index in 0..blocks.len() {
        if blocks[index].merged {
            continue;
        }
        for other in index + 1..blocks.len() {
            let (left, right) = blocks.split_at_mut(other);
            try_merge_text_line(&mut left[index], &mut right[0], font_size_tolerance);
        }
        blocks[index].bbox = lines_bbox(&blocks[index].lines);
        merged.push(blocks[index].clone());
    }
    merged
}

fn try_merge_text_line(
    block: &mut WorkingBlock,
    other: &mut WorkingBlock,
    font_size_tolerance: f32,
) -> bool {
    if other.merged || block.font_size <= 0.0 || other.font_size <= 0.0 {
        return false;
    }
    let first_count = block.lines.len();
    let other_count = other.lines.len();
    let average_font_size = (block.font_size * first_count as f32
        + other.font_size * other_count as f32)
        / (first_count + other_count) as f32;
    let cosine = dot(block.vector, other.vector) / (block.norm * other.norm).max(f32::EPSILON);
    let vector_sum = [
        block.vector[0] + other.vector[0],
        block.vector[1] + other.vector[1],
    ];
    let first_bbox = quad_bbox(block.lines.last().expect("block contains a line"));
    let other_bbox = quad_bbox(&other.lines[0]);
    let distance_x = first_bbox[0].max(other_bbox[0]) - first_bbox[2].min(other_bbox[2]);
    let distance_y = first_bbox[1].max(other_bbox[1]) - first_bbox[3].min(other_bbox[3]);
    let first_width = first_bbox[2] - first_bbox[0];
    let other_width = other_bbox[2] - other_bbox[0];
    let first_height = first_bbox[3] - first_bbox[1];
    let other_height = other_bbox[3] - other_bbox[1];

    if !quads_intersect(block.lines.last().unwrap(), &other.lines[0]) {
        if block.vertical {
            if distance_y > 0.0
                || distance_x > average_font_size * 0.8
                || distance_y.abs() / first_height.min(other_height).max(1.0) < 0.4
            {
                return false;
            }
        } else {
            let threshold = if average_font_size < 24.0 { 0.6 } else { 0.5 };
            if distance_x > 0.0
                || distance_y > average_font_size * threshold
                || distance_x.abs() / first_width.min(other_width).max(1.0) < 0.3
            {
                return false;
            }
        }
        let ratio = block.font_size / other.font_size;
        if ratio > font_size_tolerance
            || ratio.recip() > font_size_tolerance
            || cosine.abs() < 0.866
        {
            return false;
        }
    }

    block.lines.extend(other.lines.iter().copied());
    block.vector = vector_sum;
    block.norm = vector_norm(vector_sum);
    block.font_size = average_font_size;
    block.angle = vector_sum[1].atan2(vector_sum[0]).to_degrees().round();
    if block.vertical {
        block.angle -= 90.0;
    }
    block.bbox = lines_bbox(&block.lines);
    other.merged = true;
    true
}

fn sort_regions(regions: Vec<WorkingBlock>) -> Vec<WorkingBlock> {
    let right_to_left = regions.iter().any(|region| region.vertical);
    let mut input = regions;
    input.sort_unstable_by(|a, b| a.center()[1].total_cmp(&b.center()[1]));
    let mut output: Vec<WorkingBlock> = Vec::with_capacity(input.len());
    for region in input {
        let mut insertion = None;
        for (index, existing) in output.iter().enumerate() {
            if region.center()[1] > existing.bbox[3] {
                continue;
            }
            if region.center()[1] < existing.bbox[1] {
                insertion = Some((index + 1).min(output.len()));
                break;
            }
            if (right_to_left && region.center()[0] > existing.center()[0])
                || (!right_to_left && region.center()[0] < existing.center()[0])
            {
                insertion = Some(index);
                break;
            }
        }
        if let Some(index) = insertion {
            output.insert(index, region);
        } else {
            output.push(region);
        }
    }
    output
}

fn sort_line_quad(quad: ModelQuad) -> (ModelQuad, bool) {
    let edge_a = distance(quad[0], quad[1]);
    let edge_b = distance(quad[1], quad[2]);
    let square = (edge_a - edge_b).abs() < 1e-3;
    let long_vector = if edge_a >= edge_b {
        [quad[1][0] - quad[0][0], quad[1][1] - quad[0][1]]
    } else {
        [quad[2][0] - quad[1][0], quad[2][1] - quad[1][1]]
    };
    let vertical = !square && long_vector[0].abs() * 1.2 <= long_vector[1].abs();
    let mut points = quad;
    if vertical {
        points.sort_unstable_by(|a, b| a[1].total_cmp(&b[1]));
        let mut top = [points[0], points[1]];
        let mut bottom = [points[2], points[3]];
        top.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
        bottom.sort_unstable_by(|a, b| b[0].total_cmp(&a[0]));
        ([top[0], top[1], bottom[0], bottom[1]], true)
    } else {
        points.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
        let mut left = [points[0], points[1]];
        let mut right = [points[2], points[3]];
        left.sort_unstable_by(|a, b| a[1].total_cmp(&b[1]));
        right.sort_unstable_by(|a, b| a[1].total_cmp(&b[1]));
        ([left[0], right[0], right[1], left[1]], false)
    }
}

fn quads_intersect(a: &ModelQuad, b: &ModelQuad) -> bool {
    [a, b].into_iter().all(|polygon| {
        (0..4).all(|index| {
            let edge = [
                polygon[(index + 1) % 4][0] - polygon[index][0],
                polygon[(index + 1) % 4][1] - polygon[index][1],
            ];
            let axis = [-edge[1], edge[0]];
            let (a_min, a_max) = project_quad(a, axis);
            let (b_min, b_max) = project_quad(b, axis);
            a_max >= b_min && b_max >= a_min
        })
    })
}

fn project_quad(quad: &ModelQuad, axis: [f32; 2]) -> (f32, f32) {
    quad.iter()
        .map(|point| point[0] * axis[0] + point[1] * axis[1])
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

fn mean_mask(mask: &GrayImage, bbox: [f32; 4]) -> f32 {
    let x1 = bbox[0].floor().max(0.0) as u32;
    let y1 = bbox[1].floor().max(0.0) as u32;
    let x2 = bbox[2].ceil().clamp(0.0, mask.width() as f32) as u32;
    let y2 = bbox[3].ceil().clamp(0.0, mask.height() as f32) as u32;
    if x2 <= x1 || y2 <= y1 {
        return 0.0;
    }
    let mut sum = 0u64;
    for y in y1..y2 {
        for x in x1..x2 {
            sum += mask.get_pixel(x, y)[0] as u64;
        }
    }
    sum as f32 / ((x2 - x1) * (y2 - y1)) as f32 / 255.0
}

fn refine_mask(source: &DynamicImage, predicted: &GrayImage, blocks: &[TextBlock]) -> GrayImage {
    let source = source.to_rgb8();
    let mut refined = GrayImage::new(predicted.width(), predicted.height());
    let block_masks = blocks
        .par_iter()
        .filter_map(|block| {
            let [x1, y1, x2, y2] = enlarge_window(
                block.xyxy.map(|coordinate| coordinate as f32),
                predicted.width(),
                predicted.height(),
                2.5,
            );
            if x2 <= x1 || y2 <= y1 {
                return None;
            }
            let image_crop = crop_imm(&source, x1, y1, x2 - x1, y2 - y1).to_image();
            let mask_crop = crop_imm(predicted, x1, y1, x2 - x1, y2 - y1).to_image();
            let candidates = mask_candidates(&image_crop, &mask_crop);
            Some((x1, y1, merge_mask_candidates(candidates, &mask_crop)))
        })
        .collect::<Vec<_>>();
    for (x1, y1, merged) in block_masks {
        for (x, y, pixel) in merged.enumerate_pixels() {
            if pixel[0] != 0 {
                refined.put_pixel(x1 + x, y1 + y, Luma([255]));
            }
        }
    }
    refined
}

fn enlarge_window(bbox: [f32; 4], image_width: u32, image_height: u32, ratio: f32) -> [u32; 4] {
    let x1 = bbox[0].floor().clamp(0.0, image_width as f32) as i32;
    let y1 = bbox[1].floor().clamp(0.0, image_height as f32) as i32;
    let x2 = bbox[2].ceil().clamp(0.0, image_width as f32) as i32;
    let y2 = bbox[3].ceil().clamp(0.0, image_height as f32) as i32;
    let width = (x2 - x1).max(0) as f32;
    let height = (y2 - y1).max(0) as f32;
    if width <= 0.0 || height <= 0.0 {
        return [0, 0, 0, 0];
    }
    let b = width + height;
    let c = (1.0 - ratio) * width * height;
    let root = (-b + (b * b - 4.0 * c).sqrt()) * 0.5;
    let requested = (root * 0.5).round().max(0.0) as i32;
    let delta_x = requested.min(x1).min(image_width as i32 - x2).max(0);
    let delta_y = requested.min(y1).min(image_height as i32 - y2).max(0);
    [
        (x1 - delta_x) as u32,
        (y1 - delta_y) as u32,
        (x2 + delta_x) as u32,
        (y2 + delta_y) as u32,
    ]
}

struct MaskCandidate {
    mask: GrayImage,
    xor: u64,
}

fn mask_candidates(image: &RgbImage, predicted: &GrayImage) -> Vec<MaskCandidate> {
    let mut candidates = top_color_masks(image, predicted);
    let mut otsu_candidates = (0..3)
        .map(|channel| {
            let channel = GrayImage::from_fn(image.width(), image.height(), |x, y| {
                Luma([image.get_pixel(x, y)[channel]])
            });
            let level = otsu_level(&channel);
            let thresholded = GrayImage::from_fn(channel.width(), channel.height(), |x, y| {
                Luma([if channel.get_pixel(x, y)[0] > level {
                    255
                } else {
                    0
                }])
            });
            best_polarity(thresholded, predicted)
        })
        .collect::<Vec<_>>();
    otsu_candidates.sort_unstable_by_key(|candidate| candidate.xor);
    if let Some(best) = otsu_candidates.into_iter().next() {
        candidates.push(best);
    }
    candidates
}

fn top_color_masks(image: &RgbImage, predicted: &GrayImage) -> Vec<MaskCandidate> {
    let gray = DynamicImage::ImageRgb8(image.clone()).to_luma8();
    let eroded = erode_gray(predicted, 1, MorphShape::Square);
    let mut candidate_pixels = Vec::new();
    for (gray_pixel, mask_pixel) in gray.pixels().zip(eroded.pixels()) {
        if mask_pixel[0] > 127 {
            candidate_pixels.push(gray_pixel[0]);
        }
    }
    if candidate_pixels.is_empty() {
        return Vec::new();
    }
    let mut minimum = *candidate_pixels.iter().min().unwrap() as f32;
    let mut maximum = *candidate_pixels.iter().max().unwrap() as f32;
    if minimum == maximum {
        minimum -= 0.5;
        maximum += 0.5;
    }
    let bin_width = (maximum - minimum) / 255.0;
    let mut histogram = [0u32; 255];
    for pixel in candidate_pixels {
        let index = (((pixel as f32 - minimum) / bin_width).floor() as usize).min(254);
        histogram[index] += 1;
    }
    let mut colors = (0..255usize).collect::<Vec<_>>();
    colors.sort_unstable_by(|a, b| histogram[*b].cmp(&histogram[*a]).then_with(|| a.cmp(b)));
    let tolerance = histogram.iter().sum::<u32>() as f32 * 0.001;
    let mut selected = Vec::new();
    for index in colors {
        let color = minimum + index as f32 * bin_width;
        if selected
            .iter()
            .all(|selected: &f32| (*selected - color).abs() > 10.0)
        {
            selected.push(color);
        }
        if selected.len() >= 3 || (histogram[index] as f32) < tolerance {
            break;
        }
    }
    selected
        .into_iter()
        .map(|color| {
            let top = (color + 30.0).min(255.0);
            let bottom = top - 60.0;
            let thresholded = GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
                let value = gray.get_pixel(x, y)[0] as f32;
                Luma([if value >= bottom && value <= top {
                    255
                } else {
                    0
                }])
            });
            best_polarity(thresholded, predicted)
        })
        .collect()
}

fn best_polarity(mask: GrayImage, predicted: &GrayImage) -> MaskCandidate {
    let normal_xor = xor_sum(&mask, predicted);
    let inverted = GrayImage::from_fn(mask.width(), mask.height(), |x, y| {
        Luma([255 - mask.get_pixel(x, y)[0]])
    });
    let inverted_xor = xor_sum(&inverted, predicted);
    if inverted_xor < normal_xor {
        MaskCandidate {
            mask: inverted,
            xor: inverted_xor,
        }
    } else {
        MaskCandidate {
            mask,
            xor: normal_xor,
        }
    }
}

fn merge_mask_candidates(mut candidates: Vec<MaskCandidate>, predicted: &GrayImage) -> GrayImage {
    candidates.sort_unstable_by_key(|candidate| candidate.xor);
    let predicted = erode_gray(predicted, 1, MorphShape::Ellipse);
    let predicted = GrayImage::from_fn(predicted.width(), predicted.height(), |x, y| {
        Luma([if predicted.get_pixel(x, y)[0] > 60 {
            255
        } else {
            0
        }])
    });
    let mut merged = GrayImage::new(predicted.width(), predicted.height());
    for candidate in candidates {
        for component in connected_components(&candidate.mask, true, true) {
            let bbox_area =
                (component.max_x - component.min_x + 1) * (component.max_y - component.min_y + 1);
            if bbox_area >= 3 {
                accept_component(&mut merged, &predicted, &component);
            }
        }
    }
    merged = dilate_binary(&merged, 2, MorphShape::Square);

    let holes = connected_components(&merged, false, true);
    let mut areas = holes
        .iter()
        .map(|component| component.pixels.len())
        .collect::<Vec<_>>();
    areas.sort_unstable();
    let area_threshold = if areas.len() > 1 {
        areas[areas.len() - 2]
    } else {
        areas.last().copied().unwrap_or(0)
    };
    for component in holes {
        if component.pixels.len() < area_threshold {
            accept_component(&mut merged, &predicted, &component);
        }
    }
    merged
}

struct Component {
    pixels: Vec<(u32, u32)>,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

fn connected_components(
    image: &GrayImage,
    foreground_white: bool,
    diagonal: bool,
) -> Vec<Component> {
    let width = image.width();
    let height = image.height();
    let image_pixels = image.as_raw();
    let mut visited = vec![false; width as usize * height as usize];
    let mut components = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * width as usize + x as usize;
            let foreground = image_pixels[index] != 0;
            if visited[index] || foreground != foreground_white {
                continue;
            }
            visited[index] = true;
            let mut pending = vec![(x, y)];
            let mut pixels = Vec::new();
            let mut min_x = x;
            let mut min_y = y;
            let mut max_x = x;
            let mut max_y = y;
            while let Some((cx, cy)) = pending.pop() {
                pixels.push((cx, cy));
                min_x = min_x.min(cx);
                min_y = min_y.min(cy);
                max_x = max_x.max(cx);
                max_y = max_y.max(cy);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if (dx == 0 && dy == 0) || (!diagonal && dx != 0 && dy != 0) {
                            continue;
                        }
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                            continue;
                        }
                        let nx = nx as u32;
                        let ny = ny as u32;
                        let next_index = ny as usize * width as usize + nx as usize;
                        let next_foreground = image_pixels[next_index] != 0;
                        if !visited[next_index] && next_foreground == foreground_white {
                            visited[next_index] = true;
                            pending.push((nx, ny));
                        }
                    }
                }
            }
            components.push(Component {
                pixels,
                min_x,
                min_y,
                max_x,
                max_y,
            });
        }
    }
    components
}

fn accept_component(merged: &mut GrayImage, predicted: &GrayImage, component: &Component) {
    let width = component.max_x - component.min_x + 1;
    let height = component.max_y - component.min_y + 1;
    let mut component_mask = vec![false; width as usize * height as usize];
    for &(x, y) in &component.pixels {
        component_mask
            [(y - component.min_y) as usize * width as usize + (x - component.min_x) as usize] =
            true;
    }
    let mut before = 0u64;
    let mut after = 0u64;
    for y in component.min_y..=component.max_y {
        for x in component.min_x..=component.max_x {
            let current = merged.get_pixel(x, y)[0];
            let in_component = component_mask
                [(y - component.min_y) as usize * width as usize + (x - component.min_x) as usize];
            let candidate = if current != 0 || in_component { 255 } else { 0 };
            let predicted = predicted.get_pixel(x, y)[0];
            before += (current ^ predicted) as u64;
            after += (candidate ^ predicted) as u64;
        }
    }
    if after < before {
        for &(x, y) in &component.pixels {
            merged.put_pixel(x, y, Luma([255]));
        }
    }
}

#[derive(Clone, Copy)]
enum MorphShape {
    Square,
    Ellipse,
}

fn erode_gray(image: &GrayImage, radius: u32, shape: MorphShape) -> GrayImage {
    if radius == 0 {
        return image.clone();
    }
    let offsets = kernel_offsets(radius, shape);
    let width = image.width();
    let height = image.height();
    let pixels = image.as_raw();
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let mut value = u8::MAX;
        for &(dx, dy) in &offsets {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx >= 0 && ny >= 0 && nx < width as i32 && ny < height as i32 {
                value = value.min(pixels[ny as usize * width as usize + nx as usize]);
            }
        }
        Luma([value])
    })
}

fn dilate_binary(image: &GrayImage, radius: u32, shape: MorphShape) -> GrayImage {
    if radius == 0 {
        return image.clone();
    }
    let offsets = kernel_offsets(radius, shape);
    let width = image.width();
    let height = image.height();
    let pixels = image.as_raw();
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let on = offsets.iter().any(|&(dx, dy)| {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            nx >= 0
                && ny >= 0
                && nx < width as i32
                && ny < height as i32
                && pixels[ny as usize * width as usize + nx as usize] != 0
        });
        Luma([if on { 255 } else { 0 }])
    })
}

fn kernel_offsets(radius: u32, shape: MorphShape) -> Vec<(i32, i32)> {
    let radius = radius as i32;
    let mut output = Vec::with_capacity(((radius * 2 + 1) * (radius * 2 + 1)) as usize);
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let inside = match shape {
                MorphShape::Square => true,
                MorphShape::Ellipse => {
                    let radius_f = radius as f64;
                    let dy_f = dy as f64;
                    let horizontal = if radius == 0 {
                        0
                    } else {
                        (radius_f * (1.0 - dy_f * dy_f / (radius_f * radius_f)).max(0.0).sqrt())
                            .round() as i32
                    };
                    dx.abs() <= horizontal
                }
            };
            if inside {
                output.push((dx, dy));
            }
        }
    }
    output
}

fn xor_sum(a: &GrayImage, b: &GrayImage) -> u64 {
    a.as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(a, b)| (a ^ b) as u64)
        .sum()
}

fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn vector_norm(vector: [f32; 2]) -> f32 {
    vector[0].hypot(vector[1])
}

fn quad_bbox(quad: &ModelQuad) -> [f32; 4] {
    quad.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |bbox, point| {
            [
                bbox[0].min(point[0]),
                bbox[1].min(point[1]),
                bbox[2].max(point[0]),
                bbox[3].max(point[1]),
            ]
        },
    )
}

fn lines_bbox(lines: &[ModelQuad]) -> [f32; 4] {
    lines.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |bbox, line| {
            let line = quad_bbox(line);
            [
                bbox[0].min(line[0]),
                bbox[1].min(line[1]),
                bbox[2].max(line[2]),
                bbox[3].max(line[3]),
            ]
        },
    )
}

fn bbox_area(bbox: [f32; 4]) -> f32 {
    (bbox[2] - bbox[0]).max(0.0) * (bbox[3] - bbox[1]).max(0.0)
}

fn bbox_intersection(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0)
}
