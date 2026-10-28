//! Ultralytics-compatible YOLOv8 segmentation preprocessing and decoding.
//!
//! Original implementations:
//! - https://github.com/ultralytics/ultralytics/blob/2b7fac4db483aa89542c361ad4384e9119f0573d/ultralytics/data/augment.py
//! - https://github.com/ultralytics/ultralytics/blob/2b7fac4db483aa89542c361ad4384e9119f0573d/ultralytics/utils/ops.py

use anyhow::{Result, bail};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, Rgb, RgbImage, imageops};
use serde::Serialize;
use tch::{Device, IndexOp, Kind, Tensor};

use super::{config::YoloV8mSpeechBubbleConfig, model::Output};

#[derive(Debug, Clone)]
pub struct YoloV8mSegImageProcessor {
    input_size: i64,
    num_classes: i64,
    num_masks: i64,
    class_names: Vec<String>,
    mask_threshold: f32,
    letterbox_color: u8,
}

impl YoloV8mSegImageProcessor {
    pub fn new(config: &YoloV8mSpeechBubbleConfig) -> Result<Self> {
        if config.input_size <= 0 {
            bail!("YOLOv8 input_size must be positive");
        }
        if config.num_classes <= 0 || config.num_masks <= 0 {
            bail!("YOLOv8 num_classes and num_masks must be positive");
        }
        if config.class_names.len() != config.num_classes as usize {
            bail!(
                "YOLOv8 class_names count {} does not match num_classes {}",
                config.class_names.len(),
                config.num_classes
            );
        }
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
                    // Fixed-kernel interpolation preserves OpenCV's sampling behavior;
                    // convolution mode widens the kernel while downscaling.
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
            letterbox.pad_x as i64,
            letterbox.pad_y as i64,
        );

        let pixel_values = (Tensor::f_from_slice(padded.as_raw())?
            .f_view([
                1,
                letterbox.output_height as i64,
                letterbox.output_width as i64,
                3,
            ])?
            .f_permute([0, 3, 1, 2])?
            .f_to_device(device)?
            .f_to_kind(Kind::Float)?)
        .f_div_scalar(255.0)?;
        Ok((pixel_values, letterbox))
    }

    pub fn postprocess(
        &self,
        outputs: &Output,
        letterbox: &LetterBox,
        confidence_threshold: f32,
        nms_threshold: f32,
    ) -> Result<YoloV8mSpeechBubbleInstances> {
        if !(0.0..=1.0).contains(&confidence_threshold) {
            bail!("confidence threshold must be between 0 and 1");
        }
        if !(0.0..=1.0).contains(&nms_threshold) {
            bail!("NMS threshold must be between 0 and 1");
        }

        let candidates = non_max_suppression(
            &outputs.pred,
            self.num_classes,
            self.num_masks,
            confidence_threshold,
            nms_threshold,
        )?;
        if candidates.is_empty() {
            return Ok(YoloV8mSpeechBubbleInstances {
                image_width: letterbox.original_width,
                image_height: letterbox.original_height,
                instances: Vec::new(),
            });
        }

        let selected = candidates
            .iter()
            .map(|candidate| candidate.anchor)
            .collect::<Vec<_>>();
        let selected = Tensor::f_from_slice(&selected)?.f_to_device(outputs.pred.device())?;
        let mask_coefficients = outputs
            .pred
            .f_i((
                0,
                4 + self.num_classes..4 + self.num_classes + self.num_masks,
                ..,
            ))?
            .f_transpose(0, 1)?
            .f_index_select(0, &selected)?
            .f_to_kind(Kind::Float)?;

        let proto = outputs.proto.f_i(0)?.f_to_kind(Kind::Float)?;
        let proto_size = proto.size();
        if proto_size.len() != 3 || proto_size[0] != self.num_masks {
            bail!("unexpected YOLOv8 prototype shape {:?}", proto_size);
        }
        let masks = mask_coefficients
            .f_matmul(&proto.f_view([self.num_masks, -1])?)?
            .f_view([candidates.len() as i64, 1, proto_size[1], proto_size[2]])?;
        let masks = scale_masks(
            &masks,
            (letterbox.original_height, letterbox.original_width),
        )?
        .f_sigmoid()?;

        let mut instances = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.into_iter().enumerate() {
            let bbox = scale_boxes(candidate.bbox, letterbox);
            let x1 = bbox[0].ceil().clamp(0.0, letterbox.original_width as f32) as i64;
            let y1 = bbox[1].ceil().clamp(0.0, letterbox.original_height as f32) as i64;
            let x2 = bbox[2].ceil().clamp(0.0, letterbox.original_width as f32) as i64;
            let y2 = bbox[3].ceil().clamp(0.0, letterbox.original_height as f32) as i64;
            if x2 <= x1 || y2 <= y1 {
                continue;
            }

            let mask = masks
                .f_i((index as i64, 0, y1..y2, x1..x2))?
                .f_gt(self.mask_threshold as f64)?;
            let pixels = Vec::<u8>::try_from(mask.f_reshape([-1])?)?;
            let area = pixels.iter().map(|&value| u32::from(value != 0)).sum();
            if area == 0 {
                tracing::debug!(
                    score = candidate.score,
                    ?bbox,
                    "discarding speech bubble detection with an empty mask"
                );
                continue;
            }
            let pixels = pixels
                .into_iter()
                .map(|value| if value == 0 { 0 } else { u8::MAX })
                .collect();
            instances.push(YoloV8mSpeechBubbleInstance {
                label_id: candidate.label_id,
                label: self.class_names[candidate.label_id].clone(),
                score: candidate.score,
                bbox,
                area,
                mask: YoloV8mSpeechBubbleMask {
                    x: x1 as u32,
                    y: y1 as u32,
                    width: (x2 - x1) as u32,
                    height: (y2 - y1) as u32,
                    pixels,
                },
            });
        }

        Ok(YoloV8mSpeechBubbleInstances {
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
    gain: f32,
}

impl LetterBox {
    fn new(original_width: u32, original_height: u32, input_size: i64) -> Result<Self> {
        if original_width == 0 || original_height == 0 {
            bail!("cannot segment an empty image");
        }
        let input_size = input_size as u32;
        let gain = f32::min(
            input_size as f32 / original_height as f32,
            input_size as f32 / original_width as f32,
        );
        let resized_width = ((original_width as f32 * gain).round_ties_even() as i64)
            .clamp(1, input_size as i64) as u32;
        let resized_height = ((original_height as f32 * gain).round_ties_even() as i64)
            .clamp(1, input_size as i64) as u32;
        // Ultralytics prediction uses `LetterBox(auto=True, stride=32)` for a
        // same-shaped image batch, retaining only stride-aligned padding.
        let padding_width = (input_size - resized_width) % 32;
        let padding_height = (input_size - resized_height) % 32;
        let pad_x = (padding_width as f32 / 2.0 - 0.1).round_ties_even() as i64 as u32;
        let pad_y = (padding_height as f32 / 2.0 - 0.1).round_ties_even() as i64 as u32;
        Ok(Self {
            original_width,
            original_height,
            resized_width,
            resized_height,
            output_width: resized_width + padding_width,
            output_height: resized_height + padding_height,
            pad_x,
            pad_y,
            gain,
        })
    }
}

#[derive(Debug, Clone)]
pub struct YoloV8mSpeechBubbleInstances {
    pub image_width: u32,
    pub image_height: u32,
    pub instances: Vec<YoloV8mSpeechBubbleInstance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct YoloV8mSpeechBubbleInstance {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: YoloV8mSpeechBubbleMask,
}

#[derive(Debug, Clone)]
pub struct YoloV8mSpeechBubbleMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl YoloV8mSpeechBubbleMask {
    #[must_use]
    pub fn empty(x: u32, y: u32) -> Self {
        Self {
            x,
            y,
            width: 0,
            height: 0,
            pixels: Vec::new(),
        }
    }

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
        bail!("expected a single YOLOv8 prediction batch, got {size:?}");
    }
    let expected_channels = 4 + num_classes + num_masks;
    if size[1] != expected_channels {
        bail!(
            "unexpected YOLOv8 prediction channel count {}, expected {expected_channels}",
            size[1]
        );
    }
    let anchors = size[2] as usize;
    let values = Vec::<f32>::try_from(pred.f_i(0)?.f_reshape([-1])?)?;
    let mut grouped = (0..num_classes)
        .map(|_| Vec::<Candidate>::new())
        .collect::<Vec<_>>();

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
        grouped[label_id].push(Candidate {
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

    let mut selected = Vec::new();
    for candidates in &mut grouped {
        candidates.sort_unstable_by(|left, right| right.score.total_cmp(&left.score));
        for candidate in candidates.drain(..) {
            if selected.iter().all(|kept: &Candidate| {
                kept.label_id != candidate.label_id
                    || box_iou(kept.bbox, candidate.bbox) <= nms_threshold
            }) {
                selected.push(candidate);
            }
        }
    }
    selected.sort_unstable_by(|left, right| right.score.total_cmp(&left.score));
    selected.truncate(300);
    Ok(selected)
}

fn scale_boxes(mut bbox: [f32; 4], letterbox: &LetterBox) -> [f32; 4] {
    let pad_x = letterbox.pad_x as f32;
    let pad_y = letterbox.pad_y as f32;
    bbox[0] = ((bbox[0] - pad_x) / letterbox.gain).clamp(0.0, letterbox.original_width as f32);
    bbox[1] = ((bbox[1] - pad_y) / letterbox.gain).clamp(0.0, letterbox.original_height as f32);
    bbox[2] = ((bbox[2] - pad_x) / letterbox.gain).clamp(0.0, letterbox.original_width as f32);
    bbox[3] = ((bbox[3] - pad_y) / letterbox.gain).clamp(0.0, letterbox.original_height as f32);
    bbox
}

fn scale_masks(masks: &Tensor, shape: (u32, u32)) -> tch::Result<Tensor> {
    let size = masks.size();
    let input_height = size[2];
    let input_width = size[3];
    let output_height = shape.0 as i64;
    let output_width = shape.1 as i64;
    if input_height == output_height && input_width == output_width {
        return masks.f_alias();
    }

    let gain = f64::min(
        input_height as f64 / output_height as f64,
        input_width as f64 / output_width as f64,
    );
    // Ultralytics 8.2.81 computes fractional prototype padding and removes it
    // with Python's truncating `int`, rather than reusing LetterBox rounding.
    let pad_width = (input_width as f64 - output_width as f64 * gain) / 2.0;
    let pad_height = (input_height as f64 - output_height as f64 * gain) / 2.0;
    let top = (pad_height as i64).clamp(0, input_height);
    let left = (pad_width as i64).clamp(0, input_width);
    let bottom = ((input_height as f64 - pad_height) as i64).clamp(top + 1, input_height);
    let right = ((input_width as f64 - pad_width) as i64).clamp(left + 1, input_width);
    masks
        .f_slice(2, top, bottom, 1)?
        .f_slice(3, left, right, 1)?
        .f_upsample_bilinear2d(
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
    fn letterbox_and_scale_boxes_match_ultralytics() -> anyhow::Result<()> {
        let letterbox = LetterBox::new(1000, 500, 640)?;
        assert_eq!(letterbox.resized_width, 640);
        assert_eq!(letterbox.resized_height, 320);
        assert_eq!(letterbox.pad_x, 0);
        assert_eq!(letterbox.pad_y, 0);
        assert_eq!(
            (letterbox.output_width, letterbox.output_height),
            (640, 320)
        );
        let bbox = scale_boxes([100.0, 40.0, 540.0, 280.0], &letterbox);
        assert!((bbox[0] - 156.25).abs() < 1e-3);
        assert!((bbox[1] - 62.5).abs() < 1e-3);
        assert!((bbox[2] - 843.75).abs() < 1e-3);
        assert!((bbox[3] - 437.5).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn box_iou_uses_union_area() {
        assert!(
            (box_iou([0.0, 0.0, 10.0, 10.0], [5.0, 5.0, 15.0, 15.0]) - 25.0 / 175.0).abs() < 1e-6
        );
    }
}
