//! Transformers-compatible PP-DocLayout-v3 image processing and output decoding.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/image_processing_pp_doclayout_v3.py

use std::cmp::Ordering;

use anyhow::{Context, Result, bail};
use fast_image_resize::{ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GenericImageView, GrayImage, Luma};
use imageproc::{
    contours::{BorderType, find_contours_with_threshold},
    geometry::{approximate_polygon_dp, arc_length, contour_area},
};
use serde::{Deserialize, Serialize};
use tch::{Device, IndexOp, Kind, Tensor};

use super::model::Output;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PPDocLayoutV3ImageProcessor {
    pub size: SizeDict,
    pub do_resize: bool,
    pub resample: i64,
    pub do_rescale: bool,
    pub rescale_factor: f64,
    pub do_normalize: bool,
    pub image_mean: Vec<f32>,
    pub image_std: Vec<f32>,
    pub labels: Vec<String>,
}

impl Default for PPDocLayoutV3ImageProcessor {
    fn default() -> Self {
        Self {
            size: SizeDict {
                height: 800,
                width: 800,
            },
            do_resize: true,
            resample: 3,
            do_rescale: true,
            rescale_factor: 1.0 / 255.0,
            do_normalize: true,
            image_mean: vec![0.0, 0.0, 0.0],
            image_std: vec![1.0, 1.0, 1.0],
            labels: Vec::new(),
        }
    }
}

impl PPDocLayoutV3ImageProcessor {
    pub fn with_labels(mut self, labels: Vec<String>) -> Self {
        self.labels = labels;
        self
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();

        let mut pixel_values = Tensor::f_from_slice(rgb.as_raw())?
            .f_view([1, height as i64, width as i64, 3])?
            .f_permute([0, 3, 1, 2])?
            .f_to_kind(Kind::Float)?;

        if self.do_resize {
            pixel_values = match self.resample {
                0 => pixel_values.f_upsample_nearest2d(
                    [self.size.height, self.size.width],
                    None::<f64>,
                    None::<f64>,
                )?,
                2 => pixel_values.f_upsample_bilinear2d(
                    [self.size.height, self.size.width],
                    false,
                    None::<f64>,
                    None::<f64>,
                )?,
                // Torchvision falls back from Lanczos to bicubic for tensor inputs.
                1 | 3 => pixel_values.f_upsample_bicubic2d(
                    [self.size.height, self.size.width],
                    false,
                    None::<f64>,
                    None::<f64>,
                )?,
                resample => bail!("unsupported PP-DocLayout-V3 resampling mode {resample}"),
            }
            .f_clamp(0.0, 255.0)?
            .f_round()?;
        }

        if self.do_rescale {
            pixel_values = pixel_values.f_mul_scalar(self.rescale_factor)?;
        }
        if self.do_normalize {
            if self.image_mean.len() != 3 || self.image_std.len() != 3 {
                bail!("PP-DocLayout-V3 image_mean and image_std must contain three values");
            }
            let mean = Tensor::f_from_slice(&self.image_mean)?.f_view([1, 3, 1, 1])?;
            let std = Tensor::f_from_slice(&self.image_std)?.f_view([1, 3, 1, 1])?;
            pixel_values = pixel_values.f_sub(&mean)?.f_div(&std)?;
        }

        Ok(pixel_values.f_to_device(device)?)
    }

    pub fn postprocess(
        &self,
        outputs: &Output,
        image: &DynamicImage,
        threshold: f32,
    ) -> Result<PPDocLayoutV3Detections> {
        let (target_width, target_height) = image.dimensions();
        let mut detections =
            self.postprocess_batch(outputs, &[(target_height, target_width)], threshold)?;
        detections.pop().context("missing PP-DocLayout-V3 result")
    }

    /// `target_sizes` follows Transformers and uses `(height, width)`.
    pub fn postprocess_batch(
        &self,
        outputs: &Output,
        target_sizes: &[(u32, u32)],
        threshold: f32,
    ) -> Result<Vec<PPDocLayoutV3Detections>> {
        let logits = &outputs.logits;
        let batch_size = logits.size()[0] as usize;
        if target_sizes.len() != batch_size {
            bail!(
                "target size count {} does not match PP-DocLayout-V3 batch size {}",
                target_sizes.len(),
                batch_size
            );
        }

        let pred_boxes = center_to_corners(&outputs.pred_boxes)?;
        let num_queries = logits.size()[1];
        let num_classes = logits.size()[2];

        let mut scale_values = Vec::with_capacity(batch_size * 4);
        for &(target_height, target_width) in target_sizes {
            scale_values.extend_from_slice(&[
                target_width as f32,
                target_height as f32,
                target_width as f32,
                target_height as f32,
            ]);
        }
        let scale = Tensor::f_from_slice(&scale_values)?
            .f_view([batch_size as i64, 1, 4])?
            .f_to_device(pred_boxes.device())?;
        let pred_boxes = pred_boxes.f_mul(&scale)?;

        let scores_all = logits.f_sigmoid()?;
        let (scores, flat_index) =
            scores_all
                .f_flatten(1, -1)?
                .f_topk(num_queries, -1, true, true)?;
        let labels = flat_index.f_remainder(num_classes)?;
        let query_index = flat_index.f_floor_divide_scalar(num_classes)?;

        let boxes = pred_boxes.f_gather(
            1,
            &query_index
                .f_unsqueeze(-1)?
                .f_repeat([1, 1, pred_boxes.size()[2]])?,
            false,
        )?;

        let mask_size = outputs.out_masks.size();
        let mask_height = mask_size[2] as usize;
        let mask_width = mask_size[3] as usize;
        let order_seq = get_order_seq(&outputs.order_logits)?.f_gather(1, &query_index, false)?;

        let mut results = Vec::with_capacity(batch_size);
        let mask_area = mask_height * mask_width;
        for (batch, &(target_height, target_width)) in target_sizes.iter().enumerate() {
            let scores = Vec::<f32>::try_from(scores.f_i(batch as i64)?)?;
            let labels = Vec::<i64>::try_from(labels.f_i(batch as i64)?)?;
            let boxes =
                Vec::<f32>::try_from(boxes.f_i(batch as i64)?.f_contiguous()?.f_view([-1])?)?;
            let orders = Vec::<i64>::try_from(order_seq.f_i(batch as i64)?)?;
            let query_indices = Vec::<i64>::try_from(query_index.f_i(batch as i64)?)?;

            let mut candidates = Vec::new();
            for query in 0..scores.len() {
                let score = scores[query];
                if score >= threshold {
                    let offset = query * 4;
                    let bbox = [
                        boxes[offset],
                        boxes[offset + 1],
                        boxes[offset + 2],
                        boxes[offset + 3],
                    ];
                    candidates.push(CandidateRegion {
                        query: query_indices[query].max(0) as usize,
                        order_seq: orders[query],
                        label_id: labels[query].max(0) as usize,
                        score,
                        bbox,
                    });
                }
            }

            candidates.sort_by_key(|candidate| candidate.order_seq);
            let selected_queries = candidates
                .iter()
                .map(|candidate| candidate.query as i64)
                .collect::<Vec<_>>();
            let masks = if selected_queries.is_empty() {
                Vec::new()
            } else {
                let selected_queries = Tensor::f_from_slice(&selected_queries)?
                    .f_to_device(outputs.out_masks.device())?;
                Vec::<u8>::try_from(
                    outputs
                        .out_masks
                        .f_i(batch as i64)?
                        .f_index_select(0, &selected_queries)?
                        .f_sigmoid()?
                        .f_gt(threshold as f64)?
                        .f_contiguous()?
                        .f_view([-1])?,
                )?
            };

            let mut regions = Vec::with_capacity(candidates.len());
            for (idx, candidate) in candidates.into_iter().enumerate() {
                let label = self
                    .labels
                    .get(candidate.label_id)
                    .cloned()
                    .unwrap_or_else(|| format!("LABEL_{}", candidate.label_id));
                let mask_offset = idx * mask_area;
                let mask = &masks[mask_offset..mask_offset + mask_area];
                regions.push(PPDocLayoutV3Region {
                    order_seq: candidate.order_seq,
                    label_id: candidate.label_id,
                    label,
                    score: candidate.score,
                    bbox: candidate.bbox,
                    polygon_points: polygon_from_mask(
                        candidate.bbox,
                        mask,
                        mask_width,
                        mask_height,
                        self.size.width as f32 / target_width as f32 / 4.0,
                        self.size.height as f32 / target_height as f32 / 4.0,
                    ),
                });
            }

            results.push(PPDocLayoutV3Detections { regions });
        }

        Ok(results)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SizeDict {
    pub height: i64,
    pub width: i64,
}

impl Default for SizeDict {
    fn default() -> Self {
        Self {
            height: 800,
            width: 800,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PPDocLayoutV3Detections {
    pub regions: Vec<PPDocLayoutV3Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PPDocLayoutV3Region {
    pub order_seq: i64,
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub polygon_points: Option<Vec<[f32; 2]>>,
}

struct CandidateRegion {
    query: usize,
    order_seq: i64,
    label_id: usize,
    score: f32,
    bbox: [f32; 4],
}

fn center_to_corners(boxes: &Tensor) -> tch::Result<Tensor> {
    let centers = boxes.f_slice(-1, 0, 2, 1)?;
    let dims = boxes.f_slice(-1, 2, 4, 1)?;
    let top_left = centers.f_sub(&dims.f_mul_scalar(0.5)?)?;
    let bottom_right = centers.f_add(&dims.f_mul_scalar(0.5)?)?;
    Tensor::f_cat(&[top_left, bottom_right], -1)
}

// https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/image_processing_pp_doclayout_v3.py#L98-L127
fn get_order_seq(order_logits: &Tensor) -> tch::Result<Tensor> {
    let order_scores = order_logits.f_sigmoid()?;
    let size = order_scores.size();
    let batch_size = size[0];
    let sequence_length = size[1];

    let upper_votes =
        order_scores
            .f_triu(1)?
            .f_sum_dim_intlist(&[1i64][..], false, None::<Kind>)?;
    let transposed_scores = order_scores.f_transpose(1, 2)?;
    let lower_votes = transposed_scores
        .f_ones_like()?
        .f_sub(&transposed_scores)?
        .f_tril(-1)?
        .f_sum_dim_intlist(&[1i64][..], false, None::<Kind>)?;
    let order_votes = upper_votes.f_add(&lower_votes)?;
    let order_pointers = order_votes.f_argsort(1, false)?;
    let ranks = Tensor::f_arange(sequence_length, (Kind::Int64, order_logits.device()))?
        .f_expand([batch_size, sequence_length], true)?;
    let mut order_seq = order_pointers.f_empty_like()?;
    order_seq.f_scatter_(1, &order_pointers, &ranks)
}

// https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/pp_doclayout_v3/image_processing_pp_doclayout_v3.py#L129-L220
fn polygon_from_mask(
    bbox: [f32; 4],
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    scale_width: f32,
    scale_height: f32,
) -> Option<Vec<[f32; 2]>> {
    let x_min = bbox[0] as i32;
    let y_min = bbox[1] as i32;
    let x_max = bbox[2] as i32;
    let y_max = bbox[3] as i32;
    let rect = rect_polygon_i32(x_min, y_min, x_max, y_max);
    let box_width = x_max - x_min;
    let box_height = y_max - y_min;
    if box_width <= 0 || box_height <= 0 {
        return Some(rect);
    }

    let x_start = scaled_bound(x_min, scale_width, mask_width);
    let x_end = scaled_bound(x_max, scale_width, mask_width);
    let y_start = scaled_bound(y_min, scale_height, mask_height);
    let y_end = scaled_bound(y_max, scale_height, mask_height);
    if x_start >= x_end || y_start >= y_end {
        return Some(rect);
    }

    let crop_width = x_end - x_start;
    let crop_height = y_end - y_start;
    let mut crop = GrayImage::new(crop_width as u32, crop_height as u32);
    for y in 0..crop_height {
        for x in 0..crop_width {
            let value = mask[(y_start + y) * mask_width + x_start + x];
            crop.put_pixel(x as u32, y as u32, Luma([if value != 0 { 255 } else { 0 }]));
        }
    }

    let mut resized = GrayImage::new(box_width as u32, box_height as u32);
    Resizer::new()
        .resize(
            &crop,
            &mut resized,
            &ResizeOptions::new().resize_alg(ResizeAlg::Nearest),
        )
        .expect("source and destination masks have the same pixel type");
    let mut polygon = mask_to_polygon(&resized)?;
    if polygon.len() < 4 {
        return Some(rect);
    }

    for point in &mut polygon {
        point[0] += x_min as f32;
        point[1] += y_min as f32;
    }
    Some(polygon)
}

fn mask_to_polygon(mask: &GrayImage) -> Option<Vec<[f32; 2]>> {
    // OpenCV treats pixels outside the image as background when tracing an external contour.
    // imageproc only starts an outer contour after observing an in-image background pixel, so
    // pad the mask to preserve upstream behavior when foreground reaches the crop boundary.
    let mut padded = GrayImage::new(mask.width() + 2, mask.height() + 2);
    image::imageops::replace(&mut padded, mask, 1, 1);
    let contours = find_contours_with_threshold::<i32>(&padded, 0);
    let contour = contours
        .iter()
        .filter(|contour| contour.border_type == BorderType::Outer)
        .max_by(|left, right| {
            contour_area(&left.points)
                .partial_cmp(&contour_area(&right.points))
                .unwrap_or(Ordering::Equal)
        })?;

    if contour.points.len() < 3 {
        return None;
    }

    let epsilon = 0.004 * arc_length(&contour.points, true);
    let approximated = approximate_polygon_dp(&contour.points, epsilon.max(f64::EPSILON), true);
    if approximated.is_empty() {
        return None;
    }

    let points = approximated
        .into_iter()
        .map(|point| [point.x as f32 - 1.0, point.y as f32 - 1.0])
        .collect::<Vec<_>>();
    Some(extract_custom_vertices(&points))
}

fn extract_custom_vertices(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut vertices = Vec::new();
    for idx in 0..points.len() {
        let previous = points[(idx + points.len() - 1) % points.len()];
        let current = points[idx];
        let next = points[(idx + 1) % points.len()];
        let vector_1 = [previous[0] - current[0], previous[1] - current[1]];
        let vector_2 = [next[0] - current[0], next[1] - current[1]];
        let cross_product = vector_1[1] * vector_2[0] - vector_1[0] * vector_2[1];
        if cross_product >= 0.0 {
            continue;
        }

        let norm_1 = vector_norm(vector_1);
        let norm_2 = vector_norm(vector_2);
        if norm_1 == 0.0 || norm_2 == 0.0 {
            vertices.push(current);
            continue;
        }

        let angle_cos = ((vector_1[0] * vector_2[0] + vector_1[1] * vector_2[1])
            / (norm_1 * norm_2))
            .clamp(-1.0, 1.0);
        let angle = angle_cos.acos().to_degrees();
        if (angle - 45.0).abs() < 1.0 {
            let dir = [
                vector_1[0] / norm_1 + vector_2[0] / norm_2,
                vector_1[1] / norm_1 + vector_2[1] / norm_2,
            ];
            let dir_norm = vector_norm(dir);
            if dir_norm == 0.0 {
                vertices.push(current);
                continue;
            }
            let step = (norm_1 + norm_2) / 2.0;
            vertices.push([
                current[0] + dir[0] / dir_norm * step,
                current[1] + dir[1] / dir_norm * step,
            ]);
        } else {
            vertices.push(current);
        }
    }

    vertices
}

fn vector_norm(vector: [f32; 2]) -> f32 {
    (vector[0] * vector[0] + vector[1] * vector[1]).sqrt()
}

fn scaled_bound(value: i32, scale: f32, limit: usize) -> usize {
    ((value as f32 * scale).round_ties_even() as i32).clamp(0, limit as i32) as usize
}

fn rect_polygon_i32(x_min: i32, y_min: i32, x_max: i32, y_max: i32) -> Vec<[f32; 2]> {
    vec![
        [x_min as f32, y_min as f32],
        [x_max as f32, y_min as f32],
        [x_max as f32, y_max as f32],
        [x_min as f32, y_max as f32],
    ]
}
