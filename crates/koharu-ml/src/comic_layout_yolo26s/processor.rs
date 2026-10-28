//! Ultralytics 8.4.43 YOLO segmentation preprocessing and postprocessing.
//!
//! Authoritative implementations:
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/data/augment.py
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/models/yolo/segment/predict.py
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/utils/nms.py
//! - https://github.com/ultralytics/ultralytics/blob/e6cb320fce86b41b88b15111fa37e9f6fcad1e7f/ultralytics/utils/ops.py

use anyhow::{Result, bail, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, Rgb, RgbImage, imageops};
use serde::Serialize;
use tch::{Device, IndexOp, Kind, Tensor};

use super::{config::ComicLayoutYolo26sConfig, model::Output};

#[derive(Debug, Clone)]
pub struct ComicLayoutYolo26sImageProcessor {
    input_size: i64,
    num_classes: i64,
    class_names: Vec<String>,
}

impl ComicLayoutYolo26sImageProcessor {
    pub fn new(config: &ComicLayoutYolo26sConfig) -> Result<Self> {
        ensure!(config.image_size > 0, "YOLO26 image_size must be positive");
        ensure!(
            config.image_size % 32 == 0,
            "YOLO26 image_size must be divisible by 32"
        );
        ensure!(
            config.num_classes > 0,
            "YOLO26 num_classes must be positive"
        );
        let class_names = config.class_names()?;
        ensure!(
            class_names.len() == config.num_classes as usize,
            "YOLO26 class name count {} does not match num_classes {}",
            class_names.len(),
            config.num_classes
        );
        Ok(Self {
            input_size: config.image_size,
            num_classes: config.num_classes,
            class_names,
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
            Rgb([114; 3]),
        );
        imageops::replace(
            &mut padded,
            &resized,
            i64::from(letterbox.pad_x),
            i64::from(letterbox.pad_y),
        );

        let pixel_values = (Tensor::f_from_slice(padded.as_raw())?
            .f_view([
                1,
                i64::from(letterbox.output_height),
                i64::from(letterbox.output_width),
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
        output: &Output,
        letterbox: &LetterBox,
        confidence_threshold: f32,
    ) -> Result<ComicLayoutYolo26sInstances> {
        if !(0.0..=1.0).contains(&confidence_threshold) {
            bail!("confidence threshold must be between 0 and 1");
        }

        let candidates =
            filter_end_to_end_predictions(&output.pred, self.num_classes, confidence_threshold)?;
        if candidates.is_empty() {
            return Ok(ComicLayoutYolo26sInstances {
                image_width: letterbox.original_width,
                image_height: letterbox.original_height,
                instances: Vec::new(),
            });
        }

        let selected = candidates
            .iter()
            .map(|candidate| candidate.row)
            .collect::<Vec<_>>();
        let selected = Tensor::f_from_slice(&selected)?.f_to_device(output.pred.device())?;
        let mask_coefficients = output
            .pred
            .f_i((0, .., 6..38))?
            .f_index_select(0, &selected)?
            .f_to_kind(Kind::Float)?;

        let proto = output.proto.f_i(0)?.f_to_kind(Kind::Float)?;
        let proto_size = proto.size();
        ensure!(
            proto_size.len() == 3 && proto_size[0] == 32,
            "unexpected YOLO26 prototype shape {proto_size:?}"
        );
        let masks = mask_coefficients
            .f_matmul(&proto.f_view([32, -1])?)?
            .f_view([candidates.len() as i64, 1, proto_size[1], proto_size[2]])?;
        // Public masks use the original image resolution, matching
        // `retina_masks=True` and `process_mask_native` in Ultralytics.
        let masks = scale_masks(
            &masks,
            (letterbox.original_height, letterbox.original_width),
        )?;

        let mut instances = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.into_iter().enumerate() {
            let bbox = scale_boxes(candidate.bbox, letterbox);
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

            let mask = masks.f_i((index as i64, 0, y1..y2, x1..x2))?.f_gt(0.0)?;
            let pixels = Vec::<u8>::try_from(mask.f_reshape([-1])?)?;
            let area = pixels.iter().map(|&value| u32::from(value != 0)).sum();
            if area == 0 {
                continue;
            }
            let pixels = pixels
                .into_iter()
                .map(|value| if value == 0 { 0 } else { u8::MAX })
                .collect();
            instances.push(ComicLayoutYolo26sInstance {
                label_id: candidate.label_id,
                label: self.class_names[candidate.label_id].clone(),
                score: candidate.score,
                bbox,
                area,
                mask: ComicLayoutYolo26sMask {
                    x: x1 as u32,
                    y: y1 as u32,
                    width: (x2 - x1) as u32,
                    height: (y2 - y1) as u32,
                    pixels,
                },
            });
        }

        Ok(ComicLayoutYolo26sInstances {
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
        // A one-image prediction uses `LetterBox(auto=True, stride=32)`.
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
pub struct ComicLayoutYolo26sInstances {
    pub image_width: u32,
    pub image_height: u32,
    pub instances: Vec<ComicLayoutYolo26sInstance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComicLayoutYolo26sInstance {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: ComicLayoutYolo26sMask,
}

#[derive(Debug, Clone)]
pub struct ComicLayoutYolo26sMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl ComicLayoutYolo26sMask {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.pixels.is_empty()
    }
}

#[derive(Debug)]
struct Candidate {
    row: i64,
    label_id: usize,
    score: f32,
    bbox: [f32; 4],
}

fn filter_end_to_end_predictions(
    pred: &Tensor,
    num_classes: i64,
    confidence_threshold: f32,
) -> Result<Vec<Candidate>> {
    let size = pred.size();
    if size.len() != 3 || size[0] != 1 {
        bail!("expected a single YOLO26 prediction batch, got {size:?}");
    }
    ensure!(
        size[2] == 38,
        "unexpected YOLO26 prediction channel count {}, expected 38",
        size[2]
    );
    let rows = size[1] as usize;
    let values = Vec::<f32>::try_from(pred.f_i(0)?.f_reshape([-1])?)?;
    let mut candidates = Vec::new();
    for row in 0..rows {
        let offset = row * 38;
        let score = values[offset + 4];
        if score <= confidence_threshold {
            continue;
        }
        let label_id = values[offset + 5] as usize;
        ensure!(
            label_id < num_classes as usize,
            "YOLO26 returned out-of-range label {label_id}"
        );
        candidates.push(Candidate {
            row: row as i64,
            label_id,
            score,
            bbox: [
                values[offset],
                values[offset + 1],
                values[offset + 2],
                values[offset + 3],
            ],
        });
    }
    Ok(candidates)
}

fn scale_boxes(mut bbox: [f32; 4], letterbox: &LetterBox) -> [f32; 4] {
    let gain = f64::min(
        f64::from(letterbox.output_height) / f64::from(letterbox.original_height),
        f64::from(letterbox.output_width) / f64::from(letterbox.original_width),
    );
    let pad_x = ((f64::from(letterbox.output_width)
        - (f64::from(letterbox.original_width) * gain).round_ties_even() as i64 as f64)
        / 2.0
        - 0.1)
        .round_ties_even() as i64 as f32;
    let pad_y = ((f64::from(letterbox.output_height)
        - (f64::from(letterbox.original_height) * gain).round_ties_even() as i64 as f64)
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

fn scale_masks(masks: &Tensor, shape: (u32, u32)) -> tch::Result<Tensor> {
    let size = masks.size();
    let input_height = size[2];
    let input_width = size[3];
    let output_height = i64::from(shape.0);
    let output_width = i64::from(shape.1);
    let gain = f64::min(
        input_height as f64 / output_height as f64,
        input_width as f64 / output_width as f64,
    );
    let pad_width =
        (input_width as f64 - (output_width as f64 * gain).round_ties_even() as i64 as f64) / 2.0;
    let pad_height =
        (input_height as f64 - (output_height as f64 * gain).round_ties_even() as i64 as f64) / 2.0;
    let top = ((pad_height - 0.1).round_ties_even() as i64).clamp(0, input_height);
    let left = ((pad_width - 0.1).round_ties_even() as i64).clamp(0, input_width);
    let bottom =
        (input_height - (pad_height + 0.1).round_ties_even() as i64).clamp(top + 1, input_height);
    let right =
        (input_width - (pad_width + 0.1).round_ties_even() as i64).clamp(left + 1, input_width);
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

#[cfg(test)]
mod tests {
    use super::LetterBox;

    #[test]
    fn letterbox_matches_ultralytics_rectangular_prediction() -> anyhow::Result<()> {
        let letterbox = LetterBox::new(770, 1080, 1280)?;
        assert_eq!(
            (letterbox.resized_width, letterbox.resized_height),
            (913, 1280)
        );
        assert_eq!(
            (letterbox.output_width, letterbox.output_height),
            (928, 1280)
        );
        assert_eq!((letterbox.pad_x, letterbox.pad_y), (7, 0));
        Ok(())
    }
}
