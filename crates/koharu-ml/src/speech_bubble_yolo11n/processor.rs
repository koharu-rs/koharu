//! Ultralytics 8.3.227 YOLO segmentation preprocessing and postprocessing.
//!
//! Authoritative implementations:
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/data/augment.py
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/engine/predictor.py
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/models/yolo/segment/predict.py
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/utils/nms.py
//! - https://github.com/ultralytics/ultralytics/blob/e15d6f50dc618d542c1cd9f3d968c76981c90c9f/ultralytics/utils/ops.py

use anyhow::{Result, bail, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, Rgb, RgbImage, imageops};
use koharu_torch::{Device, IndexOp, Kind, Tensor};
use serde::Serialize;

use super::{config::Yolo11nSpeechBubbleConfig, model::Output};

#[derive(Debug, Clone)]
pub struct Yolo11nSegImageProcessor {
    input_size: i64,
    num_classes: i64,
    num_masks: i64,
    class_names: Vec<String>,
    mask_threshold: f32,
    letterbox_color: u8,
}

impl Yolo11nSegImageProcessor {
    pub fn new(config: &Yolo11nSpeechBubbleConfig) -> Result<Self> {
        ensure!(config.input_size > 0, "YOLO11 input_size must be positive");
        ensure!(
            config.input_size % 32 == 0,
            "YOLO11 input_size must be divisible by 32"
        );
        ensure!(
            config.num_classes > 0 && config.num_masks > 0,
            "YOLO11 num_classes and num_masks must be positive"
        );
        ensure!(
            config.class_names.len() == config.num_classes as usize,
            "YOLO11 class_names count {} does not match num_classes {}",
            config.class_names.len(),
            config.num_classes
        );
        ensure!(
            0.0 < config.mask_threshold && config.mask_threshold < 1.0,
            "mask threshold must be between 0 and 1"
        );
        Ok(Self {
            input_size: config.input_size,
            num_classes: config.num_classes,
            num_masks: config.num_masks,
            class_names: config.class_names.clone(),
            mask_threshold: config.mask_threshold,
            letterbox_color: config.letterbox_color,
        })
    }

    pub fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<(Tensor, LetterBox)> {
        let letterbox = LetterBox::new(image.width(), image.height(), self.input_size)?;
        let image = image.to_rgb8();
        let resized = if image.width() == letterbox.resized_width
            && image.height() == letterbox.resized_height
        {
            image
        } else {
            let mut resized = RgbImage::new(letterbox.resized_width, letterbox.resized_height);
            Resizer::new().resize(
                &image,
                &mut resized,
                &ResizeOptions::new()
                    .resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear))
                    .use_alpha(false),
            )?;
            resized
        };

        let mut padded = RgbImage::from_pixel(
            letterbox.output_width,
            letterbox.output_height,
            Rgb([self.letterbox_color; 3]),
        );
        imageops::replace(
            &mut padded,
            &resized,
            i64::from(letterbox.pad_x),
            i64::from(letterbox.pad_y),
        );

        let pixel_values = (Tensor::from_slice(padded.as_raw())?
            .view([
                1,
                i64::from(letterbox.output_height),
                i64::from(letterbox.output_width),
                3,
            ])?
            .permute([0, 3, 1, 2])?
            .to_device(device)?
            .to_kind(Kind::Float)?
            / 255.0)?;
        Ok((pixel_values, letterbox))
    }

    pub fn postprocess(
        &self,
        output: &Output,
        letterbox: &LetterBox,
        confidence_threshold: f32,
        nms_threshold: f32,
    ) -> Result<Yolo11nSpeechBubbleInstances> {
        if !(0.0..=1.0).contains(&confidence_threshold) {
            bail!("confidence threshold must be between 0 and 1");
        }
        if !(0.0..=1.0).contains(&nms_threshold) {
            bail!("NMS threshold must be between 0 and 1");
        }

        let candidates = non_max_suppression(
            &output.pred,
            self.num_classes,
            self.num_masks,
            confidence_threshold,
            nms_threshold,
        )?;
        if candidates.is_empty() {
            return Ok(Yolo11nSpeechBubbleInstances {
                image_width: letterbox.original_width,
                image_height: letterbox.original_height,
                instances: Vec::new(),
            });
        }

        let selected = candidates
            .iter()
            .map(|candidate| candidate.anchor)
            .collect::<Vec<_>>();
        let selected = Tensor::from_slice(&selected)?.to_device(output.pred.device())?;
        let mask_coefficients = output
            .pred
            .i((
                0,
                4 + self.num_classes..4 + self.num_classes + self.num_masks,
                ..,
            ))?
            .transpose(0, 1)?
            .index_select(0, &selected)?
            .to_kind(Kind::Float)?;

        let proto = output.proto.i(0)?.to_kind(Kind::Float)?;
        let proto_size = proto.size();
        ensure!(
            proto_size.len() == 3 && proto_size[0] == self.num_masks,
            "unexpected YOLO11 prototype shape {proto_size:?}"
        );
        let masks = mask_coefficients
            .matmul(&proto.view([self.num_masks, -1])?)?
            .view([candidates.len() as i64, 1, proto_size[1], proto_size[2]])?;
        // The public result requires original-resolution masks, so this follows
        // Ultralytics' `retina_masks=True` `process_mask_native` path exactly.
        let masks = scale_masks(
            &masks,
            (letterbox.original_height, letterbox.original_width),
        )?;
        let mask_logit_threshold =
            f64::from((self.mask_threshold / (1.0 - self.mask_threshold)).ln());

        let mut instances = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.into_iter().enumerate() {
            let bbox = scale_boxes(candidate.bbox, letterbox);
            // `crop_mask` rounds each coordinate before cropping on CPU.
            let x1 = ((f64::from(bbox[0])).round_ties_even() as i64)
                .clamp(0, i64::from(letterbox.original_width));
            let y1 = ((f64::from(bbox[1])).round_ties_even() as i64)
                .clamp(0, i64::from(letterbox.original_height));
            let x2 = ((f64::from(bbox[2])).round_ties_even() as i64)
                .clamp(0, i64::from(letterbox.original_width));
            let y2 = ((f64::from(bbox[3])).round_ties_even() as i64)
                .clamp(0, i64::from(letterbox.original_height));
            if x2 <= x1 || y2 <= y1 {
                continue;
            }

            let mask = masks
                .i((index as i64, 0, y1..y2, x1..x2))?
                .gt(mask_logit_threshold)?;
            let pixels = Vec::<u8>::try_from(mask.reshape([-1])?)?;
            let area = pixels.iter().map(|&value| u32::from(value != 0)).sum();
            if area == 0 {
                continue;
            }
            let pixels = pixels
                .into_iter()
                .map(|value| if value == 0 { 0 } else { u8::MAX })
                .collect();
            instances.push(Yolo11nSpeechBubbleInstance {
                label_id: candidate.label_id,
                label: self.class_names[candidate.label_id].clone(),
                score: candidate.score,
                bbox,
                area,
                mask: Yolo11nSpeechBubbleMask {
                    x: x1 as u32,
                    y: y1 as u32,
                    width: (x2 - x1) as u32,
                    height: (y2 - y1) as u32,
                    pixels,
                },
            });
        }

        Ok(Yolo11nSpeechBubbleInstances {
            image_width: letterbox.original_width,
            image_height: letterbox.original_height,
            instances,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LetterBox {
    original_width: u32,
    original_height: u32,
    resized_width: u32,
    resized_height: u32,
    output_width: u32,
    output_height: u32,
    pad_x: u32,
    pad_y: u32,
}

impl LetterBox {
    fn new(original_width: u32, original_height: u32, input_size: i64) -> Result<Self> {
        if original_width == 0 || original_height == 0 {
            bail!("cannot segment an empty image");
        }
        let input_size = input_size as u32;
        let gain = f64::min(
            f64::from(input_size) / f64::from(original_height),
            f64::from(input_size) / f64::from(original_width),
        );
        let resized_width = ((f64::from(original_width) * gain).round_ties_even() as i64)
            .clamp(1, i64::from(input_size)) as u32;
        let resized_height = ((f64::from(original_height) * gain).round_ties_even() as i64)
            .clamp(1, i64::from(input_size)) as u32;
        // `Model.predict` defaults to `rect=True`; a single-image batch therefore
        // uses `LetterBox(auto=True, stride=32)`.
        let padding_width = (input_size - resized_width) % 32;
        let padding_height = (input_size - resized_height) % 32;
        let pad_x = (f64::from(padding_width) / 2.0 - 0.1).round_ties_even() as i64 as u32;
        let pad_y = (f64::from(padding_height) / 2.0 - 0.1).round_ties_even() as i64 as u32;
        Ok(Self {
            original_width,
            original_height,
            resized_width,
            resized_height,
            output_width: resized_width + padding_width,
            output_height: resized_height + padding_height,
            pad_x,
            pad_y,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Yolo11nSpeechBubbleInstances {
    pub image_width: u32,
    pub image_height: u32,
    pub instances: Vec<Yolo11nSpeechBubbleInstance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Yolo11nSpeechBubbleInstance {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: Yolo11nSpeechBubbleMask,
}

#[derive(Debug, Clone)]
pub struct Yolo11nSpeechBubbleMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Yolo11nSpeechBubbleMask {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.pixels.is_empty()
    }
}

#[derive(Debug)]
struct Candidate {
    anchor: i64,
    label_id: usize,
    score: f32,
    bbox: [f32; 4],
}

fn non_max_suppression(
    pred: &Tensor,
    num_classes: i64,
    num_masks: i64,
    confidence_threshold: f32,
    nms_threshold: f32,
) -> Result<Vec<Candidate>> {
    let size = pred.size();
    if size.len() != 3 || size[0] != 1 {
        bail!("expected a single YOLO11 prediction batch, got {size:?}");
    }
    let expected_channels = 4 + num_classes + num_masks;
    ensure!(
        size[1] == expected_channels,
        "unexpected YOLO11 prediction channel count {}, expected {expected_channels}",
        size[1]
    );
    let anchors = size[2] as usize;
    let values = Vec::<f32>::try_from(pred.i(0)?.reshape([-1])?)?;
    let mut candidates = Vec::new();

    for anchor in 0..anchors {
        let mut label_id = 0;
        let mut score = f32::NEG_INFINITY;
        for class in 0..num_classes as usize {
            let class_score = values[(4 + class) * anchors + anchor];
            if class_score > score {
                label_id = class;
                score = class_score;
            }
        }
        if score <= confidence_threshold {
            continue;
        }
        let center_x = values[anchor];
        let center_y = values[anchors + anchor];
        let width = values[2 * anchors + anchor];
        let height = values[3 * anchors + anchor];
        candidates.push(Candidate {
            anchor: anchor as i64,
            label_id,
            score,
            bbox: [
                center_x - width * 0.5,
                center_y - height * 0.5,
                center_x + width * 0.5,
                center_y + height * 0.5,
            ],
        });
    }

    candidates.sort_unstable_by(|left, right| right.score.total_cmp(&left.score));
    candidates.truncate(30_000);
    let mut selected = Vec::new();
    for candidate in candidates {
        if selected.iter().all(|kept: &Candidate| {
            kept.label_id != candidate.label_id
                || box_iou(kept.bbox, candidate.bbox) <= nms_threshold
        }) {
            selected.push(candidate);
            if selected.len() == 300 {
                break;
            }
        }
    }
    Ok(selected)
}

fn scale_boxes(mut bbox: [f32; 4], letterbox: &LetterBox) -> [f32; 4] {
    let gain = f64::min(
        f64::from(letterbox.output_height) / f64::from(letterbox.original_height),
        f64::from(letterbox.output_width) / f64::from(letterbox.original_width),
    );
    let pad_x = ((f64::from(letterbox.output_width) - f64::from(letterbox.original_width) * gain)
        / 2.0
        - 0.1)
        .round_ties_even() as i64 as f32;
    let pad_y = ((f64::from(letterbox.output_height) - f64::from(letterbox.original_height) * gain)
        / 2.0
        - 0.1)
        .round_ties_even() as i64 as f32;
    let gain = gain as f32;
    bbox[0] = ((bbox[0] - pad_x) / gain).clamp(0.0, letterbox.original_width as f32);
    bbox[1] = ((bbox[1] - pad_y) / gain).clamp(0.0, letterbox.original_height as f32);
    bbox[2] = ((bbox[2] - pad_x) / gain).clamp(0.0, letterbox.original_width as f32);
    bbox[3] = ((bbox[3] - pad_y) / gain).clamp(0.0, letterbox.original_height as f32);
    bbox
}

fn scale_masks(masks: &Tensor, shape: (u32, u32)) -> koharu_torch::Result<Tensor> {
    let size = masks.size();
    let input_height = size[2];
    let input_width = size[3];
    let output_height = i64::from(shape.0);
    let output_width = i64::from(shape.1);
    let gain = f64::min(
        input_height as f64 / output_height as f64,
        input_width as f64 / output_width as f64,
    );
    let pad_width = (input_width as f64 - output_width as f64 * gain) / 2.0;
    let pad_height = (input_height as f64 - output_height as f64 * gain) / 2.0;
    let top = ((pad_height - 0.1).round_ties_even() as i64).clamp(0, input_height);
    let left = ((pad_width - 0.1).round_ties_even() as i64).clamp(0, input_width);
    let bottom =
        (input_height - (pad_height + 0.1).round_ties_even() as i64).clamp(top + 1, input_height);
    let right =
        (input_width - (pad_width + 0.1).round_ties_even() as i64).clamp(left + 1, input_width);
    masks
        .slice(2, top, bottom, 1)?
        .slice(3, left, right, 1)?
        .upsample_bilinear2d(
            [output_height, output_width],
            false,
            None::<f64>,
            None::<f64>,
        )
}

fn box_iou(left: [f32; 4], right: [f32; 4]) -> f32 {
    let intersection_width = (left[2].min(right[2]) - left[0].max(right[0])).max(0.0);
    let intersection_height = (left[3].min(right[3]) - left[1].max(right[1])).max(0.0);
    let intersection = intersection_width * intersection_height;
    if intersection <= 0.0 {
        return 0.0;
    }
    let left_area = (left[2] - left[0]).max(0.0) * (left[3] - left[1]).max(0.0);
    let right_area = (right[2] - right[0]).max(0.0) * (right[3] - right[1]).max(0.0);
    intersection / (left_area + right_area - intersection).max(f32::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::{LetterBox, box_iou, scale_boxes};

    #[test]
    fn letterbox_matches_ultralytics_rectangular_prediction() -> anyhow::Result<()> {
        let letterbox = LetterBox::new(770, 1080, 1600)?;
        assert_eq!(
            (letterbox.resized_width, letterbox.resized_height),
            (1141, 1600)
        );
        assert_eq!(
            (letterbox.output_width, letterbox.output_height),
            (1152, 1600)
        );
        assert_eq!((letterbox.pad_x, letterbox.pad_y), (5, 0));
        Ok(())
    }

    #[test]
    fn scale_boxes_matches_ultralytics() -> anyhow::Result<()> {
        let letterbox = LetterBox::new(770, 1080, 1600)?;
        let bbox = scale_boxes(
            [878.079_35, 1_206.559_6, 1_055.228, 1_454.154_4],
            &letterbox,
        );
        assert!((bbox[0] - 588.65).abs() < 0.02);
        assert!((bbox[1] - 814.43).abs() < 0.02);
        assert!((bbox[2] - 708.23).abs() < 0.02);
        assert!((bbox[3] - 981.55).abs() < 0.02);
        Ok(())
    }

    #[test]
    fn box_iou_uses_union_area() {
        assert!(
            (box_iou([0.0, 0.0, 10.0, 10.0], [5.0, 5.0, 15.0, 15.0]) - 25.0 / 175.0).abs() < 1e-6
        );
    }
}
