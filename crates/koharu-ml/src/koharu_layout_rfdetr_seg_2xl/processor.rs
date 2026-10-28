//! RF-DETR image preprocessing and instance segmentation postprocessing.
//!
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/detr.py
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/models/postprocess.py

use anyhow::{Result, ensure};
use image::DynamicImage;
use serde::Serialize;
use tch::{Device, IndexOp, Kind, Tensor};

use super::{
    config::{KoharuLayoutRFDetrSeg2XLConfig, KoharuLayoutThresholds},
    model::Output,
};

#[derive(Debug, Clone)]
pub struct KoharuLayoutRFDetrImageProcessor {
    resolution: i64,
    num_select: i64,
    class_names: Vec<String>,
    recommended_thresholds: KoharuLayoutThresholds,
}

impl KoharuLayoutRFDetrImageProcessor {
    pub fn new(config: &KoharuLayoutRFDetrSeg2XLConfig) -> Result<Self> {
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
            recommended_thresholds: config.recommended_thresholds,
        })
    }

    pub(super) fn recommended_thresholds(&self) -> KoharuLayoutThresholds {
        self.recommended_thresholds
    }

    pub(super) fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "cannot segment an empty image"
        );
        let image = image.to_rgb8();
        let mut pixel_values = (Tensor::f_from_slice(image.as_raw())?
            .f_view([1, i64::from(image.height()), i64::from(image.width()), 3])?
            .f_permute([0, 3, 1, 2])?
            .f_to_device(device)?
            .f_to_kind(Kind::Float)?)
        .f_div_scalar(255.0)?;
        if image.width() != self.resolution as u32 || image.height() != self.resolution as u32 {
            // Latest upstream disables antialiasing to match the bilinear
            // Albumentations resize used during training.
            pixel_values = pixel_values.f_upsample_bilinear2d(
                [self.resolution, self.resolution],
                false,
                None,
                None,
            )?;
        }
        let mean = Tensor::f_from_slice(&[0.485f32, 0.456, 0.406])?
            .f_view([1, 3, 1, 1])?
            .f_to_device(device)?;
        let std = Tensor::f_from_slice(&[0.229f32, 0.224, 0.225])?
            .f_view([1, 3, 1, 1])?
            .f_to_device(device)?;
        Ok(pixel_values.f_sub(&mean)?.f_div(&std)?)
    }

    pub(super) fn postprocess(
        &self,
        output: &Output,
        image_width: u32,
        image_height: u32,
        thresholds: KoharuLayoutThresholds,
    ) -> Result<KoharuLayoutDetections> {
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
        ensure!(
            output.pred_masks.size() == [1, 300, 288, 288],
            "unexpected RF-DETR mask shape {:?}",
            output.pred_masks.size()
        );

        // PostProcess ranks every query/class pair, including the checkpoint's
        // fifth background logit slot, before applying the caller threshold.
        let (scores, indexes) = output.pred_logits.f_sigmoid()?.f_view([1, -1])?.f_topk(
            self.num_select,
            1,
            true,
            true,
        )?;
        let scores = scores.f_i(0)?;
        let indexes = indexes.f_i(0)?;
        let query_indexes = indexes.f_floor_divide_scalar(5)?;
        let labels = indexes.f_remainder(5)?;
        let thresholds = Tensor::f_from_slice(&thresholds.with_background())?
            .f_to_device(scores.device())?
            .f_index_select(0, &labels)?;
        let keep = scores.f_gt_tensor(&thresholds)?;
        let selected = keep.f_nonzero()?.f_view([-1])?;

        if selected.size()[0] == 0 {
            return Ok(KoharuLayoutDetections {
                image_width,
                image_height,
                detections: Vec::new(),
            });
        }

        let scores = scores.f_index_select(0, &selected)?;
        let labels = labels.f_index_select(0, &selected)?;
        let query_indexes = query_indexes.f_index_select(0, &selected)?;
        let boxes = output
            .pred_boxes
            .f_i(0)?
            .f_index_select(0, &query_indexes)?;
        let center = boxes.f_i((.., 0..2))?;
        let half_size = (boxes.f_i((.., 2..4))?).f_div_scalar(2.0)?;
        let boxes = (Tensor::f_cat(&[center.f_sub(&half_size)?, center.f_add(&half_size)?], 1)?)
            .f_mul(
                &Tensor::f_from_slice(&[
                    image_width as f32,
                    image_height as f32,
                    image_width as f32,
                    image_height as f32,
                ])?
                .f_to_device(output.pred_boxes.device())?,
            )?;

        // Gathering before interpolation is output-equivalent to upstream and
        // avoids allocating resized masks for candidates rejected by threshold.
        let masks = output
            .pred_masks
            .f_i(0)?
            .f_index_select(0, &query_indexes)?;

        let floats = Tensor::f_cat(&[scores.f_unsqueeze(1)?, boxes], 1)?;
        let floats = Vec::<f32>::try_from(floats.f_reshape([-1])?)?;
        let labels = Vec::<i64>::try_from(labels.f_reshape([-1])?)?;

        let mut detections = Vec::with_capacity(labels.len());
        for (index, label) in labels.into_iter().enumerate() {
            // RF-DETR defines masks by bilinearly projecting each native mask to
            // the source image and thresholding at zero. Resolve one mask at a
            // time to preserve that exact contract without retaining an
            // N-by-page tensor, then keep only its non-zero page-space extent.
            let mask = masks
                .f_i(index as i64)?
                .f_unsqueeze(0)?
                .f_unsqueeze(0)?
                .f_upsample_bilinear2d(
                    [i64::from(image_height), i64::from(image_width)],
                    false,
                    None,
                    None,
                )?
                .f_gt(0.0)?
                .f_to_kind(Kind::Uint8)?;
            let rows = mask
                .f_count_nonzero_dim_intlist(&[0i64, 1, 3][..])?
                .f_gt(0)?
                .f_to_kind(Kind::Int64)?;
            let columns = mask
                .f_count_nonzero_dim_intlist(&[0i64, 1, 2][..])?
                .f_gt(0)?
                .f_to_kind(Kind::Int64)?;
            let occupied = Tensor::f_stack(
                &[
                    mask.f_count_nonzero(None)?,
                    columns.f_argmax(None, false)?,
                    rows.f_argmax(None, false)?,
                    columns
                        .f_flip([0])?
                        .f_argmax(None, false)?
                        .f_sub_scalar(i64::from(image_width))?
                        .f_neg()?,
                    rows.f_flip([0])?
                        .f_argmax(None, false)?
                        .f_sub_scalar(i64::from(image_height))?
                        .f_neg()?,
                ],
                0,
            )?;
            let occupied = Vec::<i64>::try_from(occupied.f_reshape([-1])?)?;
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
                    (mask.f_i((
                        0,
                        0,
                        i64::from(y)..i64::from(bottom),
                        i64::from(x)..i64::from(right),
                    ))?)
                    .f_mul_scalar(255)?
                    .f_reshape([-1])?,
                )?
            };
            let label_id = label as usize;
            detections.push(KoharuLayoutDetection {
                label_id,
                label: self
                    .class_names
                    .get(label_id)
                    .cloned()
                    .unwrap_or_else(|| "__background__".to_owned()),
                score: floats[index * 5],
                bbox: floats[index * 5 + 1..index * 5 + 5].try_into().unwrap(),
                area,
                mask: KoharuLayoutMask {
                    x,
                    y,
                    width,
                    height,
                    pixels,
                },
            });
        }

        Ok(KoharuLayoutDetections {
            image_width,
            image_height,
            detections,
        })
    }
}

fn validate_thresholds(thresholds: KoharuLayoutThresholds) -> Result<()> {
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
pub struct KoharuLayoutDetections {
    pub image_width: u32,
    pub image_height: u32,
    pub detections: Vec<KoharuLayoutDetection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KoharuLayoutDetection {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: KoharuLayoutMask,
}

#[derive(Debug, Clone)]
pub struct KoharuLayoutMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl KoharuLayoutMask {
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
    use super::{KoharuLayoutThresholds, validate_thresholds};

    #[test]
    fn class_thresholds_must_be_probabilities() {
        let thresholds = KoharuLayoutThresholds {
            text: 0.25,
            onomatopoeia: 1.1,
            bubble: 0.5,
            panel: 0.5,
        };

        let error = validate_thresholds(thresholds).unwrap_err();
        assert!(error.to_string().contains("onomatopoeia"));
    }
}
