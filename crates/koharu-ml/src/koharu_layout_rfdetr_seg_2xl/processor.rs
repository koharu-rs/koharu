//! RF-DETR image preprocessing and instance segmentation postprocessing.
//!
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/detr.py
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/models/postprocess.py

use anyhow::{Result, ensure};
use image::DynamicImage;
use koharu_torch::{Device, IndexOp, Kind, Tensor};
use serde::Serialize;

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
        let mut pixel_values = Tensor::from_slice(image.as_raw())
            .view([1, i64::from(image.height()), i64::from(image.width()), 3])
            .permute([0, 3, 1, 2])
            .to_device(device)
            .to_kind(Kind::Float)
            / 255.0;
        if image.width() != self.resolution as u32 || image.height() != self.resolution as u32 {
            // Latest upstream disables antialiasing to match the bilinear
            // Albumentations resize used during training.
            pixel_values = pixel_values.upsample_bilinear2d(
                [self.resolution, self.resolution],
                false,
                None,
                None,
            );
        }
        let mean = Tensor::from_slice(&[0.485f32, 0.456, 0.406])
            .view([1, 3, 1, 1])
            .to_device(device);
        let std = Tensor::from_slice(&[0.229f32, 0.224, 0.225])
            .view([1, 3, 1, 1])
            .to_device(device);
        Ok((pixel_values - mean) / std)
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
        let (scores, indexes) =
            output
                .pred_logits
                .sigmoid()
                .view([1, -1])
                .topk(self.num_select, 1, true, true);
        let scores = scores.i(0);
        let indexes = indexes.i(0);
        let query_indexes = indexes.floor_divide_scalar(5);
        let labels = indexes.remainder(5);
        let thresholds = Tensor::from_slice(&thresholds.with_background())
            .to_device(scores.device())
            .index_select(0, &labels);
        let keep = scores.gt_tensor(&thresholds);
        let selected = keep.nonzero().view([-1]);

        if selected.size()[0] == 0 {
            return Ok(KoharuLayoutDetections {
                image_width,
                image_height,
                detections: Vec::new(),
            });
        }

        let scores = scores.index_select(0, &selected);
        let labels = labels.index_select(0, &selected);
        let query_indexes = query_indexes.index_select(0, &selected);
        let boxes = output.pred_boxes.i(0).index_select(0, &query_indexes);
        let center = boxes.i((.., 0..2));
        let half_size = boxes.i((.., 2..4)) / 2.0;
        let boxes = Tensor::cat(&[&center - &half_size, &center + &half_size], 1)
            * Tensor::from_slice(&[
                image_width as f32,
                image_height as f32,
                image_width as f32,
                image_height as f32,
            ])
            .to_device(output.pred_boxes.device());

        // Gathering before interpolation is output-equivalent to upstream and
        // avoids allocating resized masks for candidates rejected by threshold.
        let masks = output.pred_masks.i(0).index_select(0, &query_indexes);

        let floats = Tensor::cat(&[scores.unsqueeze(1), boxes], 1);
        let floats = tensor_to_vec_f32(&floats)?;
        let labels = tensor_to_vec_i64(&labels)?;

        let mut detections = Vec::with_capacity(labels.len());
        for (index, label) in labels.into_iter().enumerate() {
            // RF-DETR defines masks by bilinearly projecting each native mask to
            // the source image and thresholding at zero. Resolve one mask at a
            // time to preserve that exact contract without retaining an
            // N-by-page tensor, then keep only its non-zero page-space extent.
            let mask = masks
                .i(index as i64)
                .unsqueeze(0)
                .unsqueeze(0)
                .upsample_bilinear2d(
                    [i64::from(image_height), i64::from(image_width)],
                    false,
                    None,
                    None,
                )
                .gt(0.0)
                .to_kind(Kind::Uint8);
            let rows = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 3][..])
                .gt(0)
                .to_kind(Kind::Int64);
            let columns = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 2][..])
                .gt(0)
                .to_kind(Kind::Int64);
            let occupied = Tensor::stack(
                &[
                    mask.count_nonzero(None),
                    columns.argmax(None, false),
                    rows.argmax(None, false),
                    i64::from(image_width) - columns.flip([0]).argmax(None, false),
                    i64::from(image_height) - rows.flip([0]).argmax(None, false),
                ],
                0,
            );
            let occupied = tensor_to_vec_i64(&occupied)?;
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
                tensor_to_vec_u8(
                    &(mask.i((
                        0,
                        0,
                        i64::from(y)..i64::from(bottom),
                        i64::from(x)..i64::from(right),
                    )) * 255),
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

fn tensor_to_vec_f32(tensor: &Tensor) -> Result<Vec<f32>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Float)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0.0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

fn tensor_to_vec_i64(tensor: &Tensor) -> Result<Vec<i64>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Int64)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

fn tensor_to_vec_u8(tensor: &Tensor) -> Result<Vec<u8>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Uint8)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
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
