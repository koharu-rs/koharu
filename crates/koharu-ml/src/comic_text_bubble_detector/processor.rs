//! RT-DETR image processing and comic-translate detection postprocessing.
//!
//! Original implementations:
//! - https://github.com/huggingface/transformers/blob/394b1a0eaa8e6199e372334da0aff3753a117fdb/src/transformers/models/rt_detr/image_processing_rt_detr.py
//! - https://github.com/ogkalu2/comic-translate/blob/ca3261fd1a8d4805f6b9cc0669847d463ccb8a41/modules/detection/utils/slicer.py
//! - https://github.com/ogkalu2/comic-translate/blob/ca3261fd1a8d4805f6b9cc0669847d463ccb8a41/modules/detection/base.py
//! - https://github.com/ogkalu2/comic-translate/blob/ca3261fd1a8d4805f6b9cc0669847d463ccb8a41/modules/detection/utils/content.py
//! - https://github.com/ogkalu2/comic-translate/blob/ca3261fd1a8d4805f6b9cc0669847d463ccb8a41/modules/detection/utils/geometry.py

use anyhow::{Context, Result, bail};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::DynamicImage;
use koharu_torch::{Device, IndexOp, Kind, Tensor};
use serde::{Deserialize, Serialize};

use super::model::RTDetrV2ObjectDetectionOutput;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RTDetrImageProcessor {
    pub do_resize: bool,
    pub size: SizeDict,
    pub resample: u8,
    pub do_rescale: bool,
    pub rescale_factor: f32,
    pub do_normalize: bool,
    pub image_mean: Vec<f32>,
    pub image_std: Vec<f32>,
    pub do_pad: bool,
}

impl Default for RTDetrImageProcessor {
    fn default() -> Self {
        Self {
            do_resize: true,
            size: SizeDict {
                height: 640,
                width: 640,
            },
            resample: 2,
            do_rescale: true,
            rescale_factor: 1.0 / 255.0,
            do_normalize: false,
            image_mean: vec![0.485, 0.456, 0.406],
            image_std: vec![0.229, 0.224, 0.225],
            do_pad: false,
        }
    }
}

impl RTDetrImageProcessor {
    pub fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        let image = DynamicImage::ImageRgb8(image.to_rgb8());
        let resized = if self.do_resize {
            let mut resized =
                DynamicImage::new_rgb8(self.size.width as u32, self.size.height as u32);
            let resize_alg = match self.resample {
                0 => ResizeAlg::Nearest,
                1 => ResizeAlg::Convolution(FilterType::Lanczos3),
                2 => ResizeAlg::Convolution(FilterType::Bilinear),
                3 => ResizeAlg::Convolution(FilterType::CatmullRom),
                4 => ResizeAlg::Convolution(FilterType::Box),
                5 => ResizeAlg::Convolution(FilterType::Hamming),
                value => bail!("unsupported PIL resampling filter {value}"),
            };
            Resizer::new()
                .resize(
                    &image,
                    &mut resized,
                    &ResizeOptions::new().resize_alg(resize_alg).use_alpha(false),
                )
                .context("failed to resize RT-DETR input")?;
            resized
        } else {
            image
        };

        let rgb = resized.to_rgb8();
        let (width, height) = rgb.dimensions();
        let mut pixels = Vec::with_capacity((height * width * 3) as usize);
        for pixel in rgb.pixels() {
            for channel in 0..3 {
                let mut value = pixel[channel] as f32;
                if self.do_rescale {
                    value *= self.rescale_factor;
                }
                if self.do_normalize {
                    value = (value - self.image_mean[channel]) / self.image_std[channel];
                }
                pixels.push(value);
            }
        }

        // For a single image, Transformers pads to the image's own dimensions.
        // Therefore `do_pad` does not change this tensor's shape or values.
        let _ = self.do_pad;
        Ok(Tensor::from_slice(&pixels)?
            .view([1, height as i64, width as i64, 3])?
            .permute([0, 3, 1, 2])?
            .to_device(device)?)
    }

    pub(super) fn post_process_object_detection(
        &self,
        outputs: &RTDetrV2ObjectDetectionOutput,
        threshold: f32,
        target_sizes: &[(u32, u32)],
        use_focal_loss: bool,
    ) -> Result<Vec<ObjectDetectionOutput>> {
        let logits = &outputs.logits;
        let batch_size = logits.size()[0] as usize;
        if target_sizes.len() != batch_size {
            bail!(
                "target size count {} does not match detector batch size {}",
                target_sizes.len(),
                batch_size
            );
        }

        let mut boxes = center_to_corners_format(&outputs.pred_boxes)?;
        let mut scale_values = Vec::with_capacity(batch_size * 4);
        for &(height, width) in target_sizes {
            scale_values.extend_from_slice(&[
                width as f32,
                height as f32,
                width as f32,
                height as f32,
            ]);
        }
        boxes = (&boxes
            * Tensor::from_slice(&scale_values)?
                .view([batch_size as i64, 1, 4])?
                .to_device(boxes.device())?)?;

        let num_top_queries = logits.size()[1];
        let num_classes = logits.size()[2];
        let (scores, labels, boxes) = if use_focal_loss {
            let (scores, index) =
                logits
                    .sigmoid()?
                    .flatten(1, -1)?
                    .topk(num_top_queries, -1, true, true)?;
            let labels = index.remainder(num_classes)?;
            let index = index.floor_divide_scalar(num_classes)?;
            let boxes = boxes.gather(
                1,
                &index.unsqueeze(-1)?.repeat([1, 1, boxes.size()[2]])?,
                false,
            )?;
            (scores, labels, boxes)
        } else {
            let probabilities = logits.softmax(-1, None::<Kind>)?;
            let probabilities = probabilities.slice(-1, 0, num_classes - 1, 1)?;
            let (scores, labels) = probabilities.max_dim(-1, false)?;
            (scores, labels, boxes)
        };

        let mut results = Vec::with_capacity(batch_size);
        for batch in 0..batch_size {
            let scores = Vec::<f32>::try_from(scores.i(batch as i64)?)?;
            let labels = Vec::<i64>::try_from(labels.i(batch as i64)?)?;
            let boxes = Vec::<f32>::try_from(boxes.i(batch as i64)?.contiguous()?.view([-1])?)?;
            let mut result = ObjectDetectionOutput::default();
            for query in 0..scores.len() {
                if scores[query] <= threshold {
                    continue;
                }
                let offset = query * 4;
                result.scores.push(scores[query]);
                result.labels.push(labels[query]);
                result.boxes.push([
                    boxes[offset],
                    boxes[offset + 1],
                    boxes[offset + 2],
                    boxes[offset + 3],
                ]);
            }
            results.push(result);
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
            height: 640,
            width: 640,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ObjectDetectionOutput {
    pub scores: Vec<f32>,
    pub labels: Vec<i64>,
    pub boxes: Vec<[f32; 4]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextBlock {
    pub xyxy: [i32; 4],
    pub bubble_xyxy: Option<[i32; 4]>,
    pub text_class: String,
    pub direction: String,
    pub font_color: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(super) struct ImageSlicer {
    height_to_width_ratio_threshold: f32,
    target_slice_ratio: f32,
    overlap_height_ratio: f32,
    min_slice_height_ratio: f32,
    merge_iou_threshold: f32,
    duplicate_iou_threshold: f32,
    merge_y_distance_threshold: f32,
    containment_threshold: f32,
}

impl Default for ImageSlicer {
    fn default() -> Self {
        Self {
            height_to_width_ratio_threshold: 3.5,
            target_slice_ratio: 3.0,
            overlap_height_ratio: 0.2,
            min_slice_height_ratio: 0.7,
            merge_iou_threshold: 0.2,
            duplicate_iou_threshold: 0.5,
            merge_y_distance_threshold: 0.1,
            containment_threshold: 0.85,
        }
    }
}

impl ImageSlicer {
    fn should_slice(&self, image: &DynamicImage) -> bool {
        image.height() as f32 / image.width() as f32 > self.height_to_width_ratio_threshold
    }

    fn calculate_slice_params(&self, image: &DynamicImage) -> (u32, u32, u32, u32) {
        let slice_width = image.width();
        let slice_height = (slice_width as f32 * self.target_slice_ratio) as u32;
        let effective_slice_height =
            (slice_height as f32 * (1.0 - self.overlap_height_ratio)) as u32;
        let mut num_slices = image.height().div_ceil(effective_slice_height);
        let last_slice_start = (num_slices - 1) * effective_slice_height;
        let last_slice_height = image.height() - last_slice_start;
        if (last_slice_height as f32 / slice_height as f32) < self.min_slice_height_ratio
            && num_slices > 1
        {
            num_slices -= 1;
        }
        (
            slice_width,
            slice_height,
            effective_slice_height,
            num_slices,
        )
    }

    fn get_slice(
        &self,
        image: &DynamicImage,
        slice_number: u32,
        effective_slice_height: u32,
        slice_height: u32,
    ) -> (DynamicImage, u32, u32) {
        let start_y = slice_number * effective_slice_height;
        let end_y = if slice_number == image.height().div_ceil(effective_slice_height) - 1 {
            image.height()
        } else {
            (start_y + slice_height).min(image.height())
        };
        (
            image.crop_imm(0, start_y, image.width(), end_y - start_y),
            start_y,
            end_y,
        )
    }

    fn adjust_box_coordinates(&self, boxes: &mut [[f32; 4]], start_y: u32) {
        for bbox in boxes {
            bbox[1] += start_y as f32;
            bbox[3] += start_y as f32;
        }
    }

    fn box_contained(&self, box1: [f32; 4], box2: [f32; 4]) -> (bool, f32, u8) {
        let area1 = box_area(box1);
        let area2 = box_area(box2);
        let intersection = intersection_area(box1, box2);
        if intersection <= 0.0 {
            return (false, 0.0, 0);
        }
        let containment_ratio = intersection / area1.min(area2);
        if containment_ratio >= self.containment_threshold {
            (true, containment_ratio, if area1 > area2 { 1 } else { 2 })
        } else {
            (false, containment_ratio, 0)
        }
    }

    fn merge_overlapping_boxes(
        &self,
        mut boxes: Vec<[f32; 4]>,
        image_height: u32,
    ) -> Vec<[f32; 4]> {
        let y_distance_threshold = self.merge_y_distance_threshold * image_height as f32;
        let mut index = 0;
        while index + 1 < boxes.len() {
            let mut next = index + 1;
            while next < boxes.len() {
                let box1 = boxes[index];
                let box2 = boxes[next];
                let area1 = box_area(box1);
                let area2 = box_area(box2);
                let (contained, _, which_contains) = self.box_contained(box1, box2);
                if contained {
                    if which_contains == 2 {
                        boxes[index] = box2;
                    }
                    boxes.remove(next);
                    continue;
                }

                if calculate_iou(box1, box2) >= self.duplicate_iou_threshold {
                    if area2 > area1 {
                        boxes[index] = box2;
                    }
                    boxes.remove(next);
                    continue;
                }

                let width1 = box1[2] - box1[0];
                let height1 = box1[3] - box1[1];
                let width2 = box2[2] - box2[0];
                let height2 = box2[3] - box2[1];
                let y_distance = (box1[1] - box2[3]).abs().min((box1[3] - box2[1]).abs());
                let local_y_threshold = y_distance_threshold.min(height1.max(height2) * 0.1);
                let x_overlap = (box1[2].min(box2[2]) - box1[0].max(box2[0])).max(0.0);
                let x_overlap_ratio = x_overlap / width1.min(width2).max(1.0);
                let size_ratio = area1.min(area2) / area1.max(area2).max(1.0);
                if y_distance < local_y_threshold
                    && x_overlap_ratio > self.merge_iou_threshold
                    && size_ratio > 0.3
                    && (box1[0] - box2[0]).abs() < 0.5 * width1.max(width2)
                    && (box1[2] - box2[2]).abs() < 0.5 * width1.max(width2)
                {
                    let merged = merge_boxes(box1, box2);
                    if box_area(merged) > 3.0 * area1.max(area2) {
                        next += 1;
                        continue;
                    }
                    boxes[index] = merged;
                    boxes.remove(next);
                } else {
                    next += 1;
                }
            }
            index += 1;
        }
        boxes
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn process_slices_for_detection<F>(
        &self,
        image: &DynamicImage,
        mut detect: F,
    ) -> Result<(Vec<[f32; 4]>, Vec<[f32; 4]>)>
    where
        F: FnMut(&DynamicImage) -> Result<(Vec<[f32; 4]>, Vec<[f32; 4]>)>,
    {
        if !self.should_slice(image) {
            return detect(image);
        }

        let (_, slice_height, effective_slice_height, _) = self.calculate_slice_params(image);
        let num_slices = image.height().div_ceil(effective_slice_height);
        let (first_slice, _, _) = self.get_slice(image, 0, effective_slice_height, slice_height);
        let (mut bubble_boxes, mut text_boxes) = detect(&first_slice)?;
        for slice_number in 1..num_slices {
            let (slice, start_y, _) =
                self.get_slice(image, slice_number, effective_slice_height, slice_height);
            let (mut slice_bubbles, mut slice_texts) = detect(&slice)?;
            self.adjust_box_coordinates(&mut slice_bubbles, start_y);
            self.adjust_box_coordinates(&mut slice_texts, start_y);
            bubble_boxes.extend(slice_bubbles);
            text_boxes.extend(slice_texts);
        }

        Ok((
            self.merge_overlapping_boxes(bubble_boxes, image.height()),
            self.merge_overlapping_boxes(text_boxes, image.height()),
        ))
    }
}

pub(super) fn create_text_blocks(
    image: &DynamicImage,
    text_boxes: Vec<[f32; 4]>,
    bubble_boxes: Vec<[f32; 4]>,
) -> Vec<TextBlock> {
    let text_boxes = filter_and_fix_bboxes(text_boxes, image.width(), image.height());
    let bubble_boxes = filter_and_fix_bboxes(bubble_boxes, image.width(), image.height());
    let text_boxes = merge_overlapping_boxes(&text_boxes);

    text_boxes
        .into_iter()
        .map(|xyxy| {
            let bubble_xyxy = bubble_boxes.iter().copied().find(|bubble| {
                does_rectangle_fit(*bubble, xyxy) || calculate_iou(*bubble, xyxy) >= 0.2
            });
            TextBlock {
                xyxy: xyxy.map(|value| value as i32),
                bubble_xyxy: bubble_xyxy.map(|bbox| bbox.map(|value| value as i32)),
                text_class: if bubble_xyxy.is_some() {
                    "text_bubble".to_owned()
                } else {
                    "text_free".to_owned()
                },
                // FontEngine is a separate upstream model. Its failure/default path
                // yields these values and keeps this detector self-contained.
                direction: String::new(),
                font_color: Vec::new(),
            }
        })
        .collect()
}

fn center_to_corners_format(boxes: &Tensor) -> koharu_torch::Result<Tensor> {
    let centers = boxes.slice(-1, 0, 2, 1)?;
    let dimensions = boxes.slice(-1, 2, 4, 1)?;
    Tensor::cat(
        &[
            (&centers - (&dimensions * 0.5)?)?,
            (centers + (dimensions * 0.5)?)?,
        ],
        -1,
    )
}

fn filter_and_fix_bboxes(
    boxes: Vec<[f32; 4]>,
    image_width: u32,
    image_height: u32,
) -> Vec<[f32; 4]> {
    boxes
        .into_iter()
        .filter_map(|mut bbox| {
            bbox[0] = bbox[0].clamp(0.0, image_width as f32);
            bbox[2] = bbox[2].clamp(0.0, image_width as f32);
            bbox[1] = bbox[1].clamp(0.0, image_height as f32);
            bbox[3] = bbox[3].clamp(0.0, image_height as f32);
            ((bbox[2] - bbox[0]) > 5.0 && (bbox[3] - bbox[1]) > 5.0).then_some(bbox)
        })
        .collect()
}

fn merge_overlapping_boxes(boxes: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let mut accepted = Vec::new();
    for (index, &bbox) in boxes.iter().enumerate() {
        let mut merged = bbox;
        for (other_index, &other) in boxes.iter().enumerate() {
            if index != other_index
                && (is_mostly_contained(merged, other, 0.3)
                    || is_mostly_contained(other, merged, 0.3))
            {
                merged = merge_boxes(merged, other);
            }
        }
        if accepted
            .iter()
            .any(|&accepted| accepted == merged || calculate_iou(accepted, merged) >= 0.5)
        {
            continue;
        }
        accepted.retain(|&accepted| accepted != merged && calculate_iou(accepted, merged) < 0.5);
        accepted.push(merged);
    }
    accepted
}

fn is_mostly_contained(outer: [f32; 4], inner: [f32; 4], threshold: f32) -> bool {
    let outer_area = box_area(outer);
    let inner_area = box_area(inner);
    outer_area >= inner_area
        && inner_area != 0.0
        && intersection_area(outer, inner) / inner_area >= threshold
}

fn merge_boxes(box1: [f32; 4], box2: [f32; 4]) -> [f32; 4] {
    [
        box1[0].min(box2[0]),
        box1[1].min(box2[1]),
        box1[2].max(box2[2]),
        box1[3].max(box2[3]),
    ]
}

fn box_area(bbox: [f32; 4]) -> f32 {
    (bbox[2] - bbox[0]) * (bbox[3] - bbox[1])
}

fn intersection_area(box1: [f32; 4], box2: [f32; 4]) -> f32 {
    (box1[2].min(box2[2]) - box1[0].max(box2[0])).max(0.0)
        * (box1[3].min(box2[3]) - box1[1].max(box2[1])).max(0.0)
}

fn calculate_iou(box1: [f32; 4], box2: [f32; 4]) -> f32 {
    let intersection = intersection_area(box1, box2);
    let union = box_area(box1) + box_area(box2) - intersection;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn does_rectangle_fit(bigger: [f32; 4], smaller: [f32; 4]) -> bool {
    let (left1, right1) = (bigger[0].min(bigger[2]), bigger[0].max(bigger[2]));
    let (top1, bottom1) = (bigger[1].min(bigger[3]), bigger[1].max(bigger[3]));
    let (left2, right2) = (smaller[0].min(smaller[2]), smaller[0].max(smaller[2]));
    let (top2, bottom2) = (smaller[1].min(smaller[3]), smaller[1].max(smaller[3]));
    left1 <= left2 && right1 >= right2 && top1 <= top2 && bottom1 >= bottom2
}
