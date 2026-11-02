//! RF-DETR image preprocessing and instance segmentation postprocessing.
//!
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/detr.py
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/models/postprocess.py
//!
//! The checkpoint was trained on letterboxed pages instead of RF-DETR's
//! square resize: albumentations 2.0.8's `LongestMaxSize(resolution,
//! area_for_downscale="image")`, then `PadIfNeeded` to a square with the page
//! top-left on white (255), then RF-DETR's `Normalize`. Predictions map back
//! by one scale to a square of the page's long side, cropped to the page.

use anyhow::{Result, ensure};
use image::DynamicImage;
use koharu_torch::{Device, IndexOp, Kind, Tensor};
use serde::Serialize;

use super::{
    config::{Config, Thresholds},
    model::Output,
};

#[derive(Debug, Clone)]
pub(super) struct ImageProcessor {
    resolution: i64,
    num_select: i64,
    class_names: Vec<String>,
}

impl ImageProcessor {
    pub(super) fn new(config: &Config) -> Result<Self> {
        ensure!(config.resolution > 0, "RF-DETR resolution must be positive");
        ensure!(
            config.resolution % 24 == 0,
            "RF-DETR resolution must be divisible by patch_size * num_windows"
        );
        ensure!(config.num_select > 0, "RF-DETR num_select must be positive");
        Ok(Self {
            resolution: config.resolution,
            num_select: config.num_select,
            class_names: config.class_names(),
        })
    }

    pub(super) fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "cannot segment an empty image"
        );
        let image = image.to_rgb8();
        let (height, width) = (i64::from(image.height()), i64::from(image.width()));
        let pixels = Tensor::from_slice(image.as_raw())?
            .view([height, width, 3])?
            .permute([2, 0, 1])?
            .to(device)?
            .to_kind(Kind::Float)?;
        // LongestMaxSize scales both sides by one factor and rounds them as
        // Python's round() does.
        let scale = self.resolution as f64 / height.max(width) as f64;
        let side = |length: i64| ((length as f64 * scale).round_ties_even() as i64).max(1);
        let (resized_height, resized_width) = (side(height), side(width));
        let resized = if (resized_height, resized_width) == (height, width) {
            pixels
        } else if scale < 1.0 {
            // cv2.INTER_AREA, which is separable.
            let rows = area_weights(height, resized_height, device)?;
            let columns = area_weights(width, resized_width, device)?;
            rows.matmul(&pixels)?
                .matmul(&columns.transpose_int(0, 1)?)?
        } else {
            // cv2.INTER_LINEAR. OpenCV weighs 8-bit pixels in 11-bit fixed
            // point, so a pixel can differ from training's by one level.
            pixels
                .unsqueeze(0)?
                .upsample_bilinear2d([resized_height, resized_width], false, None, None)?
                .squeeze_dim(0)?
        };
        // cv2.resize returns 8-bit pixels. Ties round to even here, but
        // OpenCV's path for shrinking by exactly 2 rounds them up, so a pixel
        // there can also differ by one level.
        let resized = resized.round()?;

        let canvas = Tensor::full(
            [3, self.resolution, self.resolution],
            255.0,
            (Kind::Float, device),
        )?;
        canvas
            .narrow(1, 0, resized_height)?
            .narrow(2, 0, resized_width)?
            .copy_(&resized, false)?;
        let mean = Tensor::from_slice(&[0.485f32, 0.456, 0.406])?
            .view([3, 1, 1])?
            .to(device)?;
        let std = Tensor::from_slice(&[0.229f32, 0.224, 0.225])?
            .view([3, 1, 1])?
            .to(device)?;
        Ok(canvas
            .div_scalar(255.0)?
            .sub_tensor(&mean, 1)?
            .div_tensor(&std)?
            .unsqueeze(0)?)
    }

    pub(super) fn postprocess(
        &self,
        output: &Output,
        image_width: u32,
        image_height: u32,
        thresholds: Thresholds,
    ) -> Result<Detections> {
        validate_thresholds(thresholds)?;
        let logits_size = output.pred_logits.size();
        ensure!(
            logits_size == [1, 300, 5],
            "unexpected RF-DETR logits shape {logits_size:?}"
        );
        ensure!(
            output.pred_boxes.size() == [1, 300, 4],
            "unexpected RF-DETR box shape {:?}",
            output.pred_boxes.size()
        );
        let mask_size = self.resolution / 4;
        ensure!(
            output.pred_masks.size() == [1, 300, mask_size, mask_size],
            "unexpected RF-DETR mask shape {:?}",
            output.pred_masks.size()
        );
        // The letterboxed input covers a square of the page's long side.
        let square = image_width.max(image_height);

        // PostProcess ranks every query/class pair, including the checkpoint's
        // fifth background logit slot, before applying the caller threshold.
        let (scores, indexes) =
            output
                .pred_logits
                .sigmoid()?
                .view([1, -1])?
                .topk(self.num_select, 1, true, true)?;
        let scores = scores.i(0)?;
        let indexes = indexes.i(0)?;
        let query_indexes = indexes.floor_divide_scalar(5)?;
        let labels = indexes.remainder_scalar(5)?;
        let thresholds = Tensor::from_slice(&thresholds.with_background())?
            .to(scores.device())?
            .index_select(0, &labels)?;
        let keep = scores.gt_tensor(&thresholds)?;
        let selected = keep.nonzero()?.view([-1])?;

        if selected.size()[0] == 0 {
            return Ok(Detections {
                image_width,
                image_height,
                detections: Vec::new(),
            });
        }

        let scores = scores.index_select(0, &selected)?;
        let labels = labels.index_select(0, &selected)?;
        let query_indexes = query_indexes.index_select(0, &selected)?;
        let boxes = output.pred_boxes.i(0)?.index_select(0, &query_indexes)?;
        let center = boxes.i((.., 0..2))?;
        let half_size = (boxes.i((.., 2..4))?).div_scalar(2.0)?;
        let boxes = (Tensor::cat(
            &[
                center.sub_tensor(&half_size, 1)?,
                center.add_tensor(&half_size, 1)?,
            ],
            1,
        )?)
        .mul_scalar(f64::from(square))?;

        // Gathering before interpolation is output-equivalent to upstream and
        // avoids allocating resized masks for candidates rejected by threshold.
        let masks = output.pred_masks.i(0)?.index_select(0, &query_indexes)?;

        let floats = Tensor::cat(&[scores.unsqueeze(1)?, boxes], 1)?;
        let floats = Vec::<f32>::try_from(floats.reshape([-1])?)?;
        let labels = Vec::<i64>::try_from(labels.reshape([-1])?)?;

        let mut detections = Vec::with_capacity(labels.len());
        for (index, label) in labels.into_iter().enumerate() {
            // RF-DETR defines masks by bilinearly projecting each native mask to
            // the square the input covers and thresholding at zero; the page is
            // its top-left. Resolve one mask at a time to preserve that exact
            // contract without retaining an N-by-page tensor, then keep only
            // its non-zero page-space extent.
            let mask = masks
                .i(index as i64)?
                .unsqueeze(0)?
                .unsqueeze(0)?
                .upsample_bilinear2d([i64::from(square), i64::from(square)], false, None, None)?
                .narrow(2, 0, i64::from(image_height))?
                .narrow(3, 0, i64::from(image_width))?
                .gt_scalar(0.0)?
                .to_kind(Kind::Uint8)?;
            let rows = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 3][..])?
                .gt_scalar(0)?
                .to_kind(Kind::Int64)?;
            let columns = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 2][..])?
                .gt_scalar(0)?
                .to_kind(Kind::Int64)?;
            let occupied = Tensor::stack(
                &[
                    mask.count_nonzero(None)?,
                    columns.argmax(None, false)?,
                    rows.argmax(None, false)?,
                    columns
                        .flip([0])?
                        .argmax(None, false)?
                        .sub_scalar(i64::from(image_width), 1)?
                        .neg()?,
                    rows.flip([0])?
                        .argmax(None, false)?
                        .sub_scalar(i64::from(image_height), 1)?
                        .neg()?,
                ],
                0,
            )?;
            let occupied = Vec::<i64>::try_from(occupied.reshape([-1])?)?;
            let area = occupied[0].clamp(0, i64::from(u32::MAX)) as u32;
            let (x, y, right, bottom) = if area == 0 {
                (0, 0, 0, 0)
            } else {
                (
                    occupied[1] as u32,
                    occupied[2] as u32,
                    occupied[3] as u32,
                    occupied[4] as u32,
                )
            };
            let width = right.saturating_sub(x);
            let height = bottom.saturating_sub(y);
            let pixels = if width == 0 || height == 0 {
                Vec::new()
            } else {
                Vec::<u8>::try_from(
                    (mask.i((
                        0,
                        0,
                        i64::from(y)..i64::from(bottom),
                        i64::from(x)..i64::from(right),
                    ))?)
                    .mul_scalar(255)?
                    .reshape([-1])?,
                )?
            };
            let label_id = label as usize;
            detections.push(Detection {
                label_id,
                label: self
                    .class_names
                    .get(label_id)
                    .cloned()
                    .unwrap_or_else(|| "__background__".to_owned()),
                score: floats[index * 5],
                bbox: floats[index * 5 + 1..index * 5 + 5].try_into().unwrap(),
                area,
                mask: Mask {
                    x,
                    y,
                    width,
                    height,
                    pixels,
                },
            });
        }

        Ok(Detections {
            image_width,
            image_height,
            detections,
        })
    }
}

/// cv2.resize's INTER_AREA weights for shrinking an axis of `input` pixels to
/// `output`, as an `output` × `input` matrix. Follows `computeResizeAreaTab`
/// in OpenCV's imgproc/src/resize.cpp, including its 1e-3 cut-offs.
fn area_weights(input: i64, output: i64, device: Device) -> Result<Tensor> {
    let scale = 1.0 / (output as f64 / input as f64);
    let mut weights = vec![0.0f32; (output * input) as usize];
    for (dst, row) in weights.chunks_exact_mut(input as usize).enumerate() {
        let start = dst as f64 * scale;
        let end = start + scale;
        let cell = scale.min(input as f64 - start);
        let last = (end.floor() as i64).min(input - 1);
        let first = (start.ceil() as i64).min(last);
        if first as f64 - start > 1e-3 {
            row[first as usize - 1] += ((first as f64 - start) / cell) as f32;
        }
        for src in first..last {
            row[src as usize] += (1.0 / cell) as f32;
        }
        if end - last as f64 > 1e-3 {
            row[last as usize] += ((end - last as f64).min(1.0).min(cell) / cell) as f32;
        }
    }
    Ok(Tensor::from_slice(&weights)?
        .view([output, input])?
        .to(device)?)
}

fn validate_thresholds(thresholds: Thresholds) -> Result<()> {
    for (class, threshold) in [
        ("text", thresholds.text),
        ("onomatopoeia", thresholds.onomatopoeia),
        ("bubble", thresholds.bubble),
        ("panel", thresholds.panel),
    ] {
        ensure!(
            (0.0..=1.0).contains(&threshold),
            "{class} confidence threshold must be between 0 and 1"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Detections {
    pub image_width: u32,
    pub image_height: u32,
    pub detections: Vec<Detection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: Mask,
}

#[derive(Debug, Clone)]
pub struct Mask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Mask {
    #[must_use]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let Some(local_x) = x.checked_sub(self.x) else {
            return false;
        };
        let Some(local_y) = y.checked_sub(self.y) else {
            return false;
        };
        if local_x >= self.width || local_y >= self.height {
            return false;
        }
        self.pixels
            .get(local_y as usize * self.width as usize + local_x as usize)
            .is_some_and(|value| *value != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Thresholds, validate_thresholds};

    #[test]
    fn class_thresholds_must_be_probabilities() {
        let thresholds = Thresholds {
            text: 0.25,
            onomatopoeia: 1.1,
            bubble: 0.5,
            panel: 0.5,
        };

        let error = validate_thresholds(thresholds).unwrap_err();
        assert!(error.to_string().contains("onomatopoeia"));
    }
}
