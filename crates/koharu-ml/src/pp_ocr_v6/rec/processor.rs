//! Transformers-compatible PP-OCRv6 text recognition processing and CTC decoding.
//!
//! Original implementation:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/pp_ocrv6_small_rec/image_processing_pp_ocrv6_small_rec.py

use std::path::Path;

use anyhow::{Context, Result, bail};
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use tch::{Device, Kind, Tensor};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PPOCRV6MediumRecImageProcessor {
    pub size: SizeDict,
    pub pad_size: SizeDict,
    pub do_resize: bool,
    pub do_rescale: bool,
    pub do_convert_rgb: bool,
    pub do_normalize: bool,
    pub do_pad: bool,
    pub image_mean: Vec<f32>,
    pub image_std: Vec<f32>,
    pub rescale_factor: f64,
    pub resample: i64,
    pub max_image_width: i64,
    pub character_list: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SizeDict {
    pub height: i64,
    pub width: i64,
}

impl Default for PPOCRV6MediumRecImageProcessor {
    fn default() -> Self {
        Self {
            size: SizeDict {
                height: 48,
                width: 320,
            },
            pad_size: SizeDict {
                height: 48,
                width: 320,
            },
            do_resize: true,
            do_rescale: true,
            do_convert_rgb: true,
            do_normalize: true,
            do_pad: true,
            image_mean: vec![0.485, 0.456, 0.406],
            image_std: vec![0.229, 0.224, 0.225],
            rescale_factor: 1.0 / 255.0,
            resample: 2,
            max_image_width: 3200,
            character_list: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextRecognition {
    pub text: String,
    pub score: f32,
}

impl PPOCRV6MediumRecImageProcessor {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        serde_json::from_str(&std::fs::read_to_string(path)?)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        if !self.do_convert_rgb && image.color().channel_count() < 3 {
            bail!("PP-OCRv6 recognition requires at least three input channels");
        }
        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();
        let mut pixel_values = Tensor::f_from_slice(rgb.as_raw())?
            .f_view([1, height as i64, width as i64, 3])?
            .f_permute([0, 3, 1, 2])?
            .f_to_kind(Kind::Float)?
            .f_to_device(device)?;

        let target_width = self.target_width(height as i64, width as i64);
        if self.do_resize {
            // Transformers explicitly disables antialiasing here to match cv2.resize.
            pixel_values = match self.resample {
                0 => pixel_values.f_upsample_nearest2d(
                    [self.size.height, target_width],
                    None::<f64>,
                    None::<f64>,
                )?,
                2 => pixel_values.f_upsample_bilinear2d(
                    [self.size.height, target_width],
                    false,
                    None::<f64>,
                    None::<f64>,
                )?,
                3 => pixel_values.f_upsample_bicubic2d(
                    [self.size.height, target_width],
                    false,
                    None::<f64>,
                    None::<f64>,
                )?,
                resample => bail!("unsupported PP-OCRv6 recognition resampling mode {resample}"),
            };
        }
        // Transformers converts RGB input to BGR before normalization.
        pixel_values = pixel_values.f_flip([1])?;
        if self.do_rescale {
            pixel_values = pixel_values.f_mul_scalar(self.rescale_factor)?;
        }
        if self.do_normalize {
            if self.image_mean.len() != 3 || self.image_std.len() != 3 {
                bail!("PP-OCRv6 recognition image_mean and image_std must contain three values");
            }
            let mean = Tensor::f_from_slice(&self.image_mean)?
                .f_view([1, 3, 1, 1])?
                .f_to_device(device)?;
            let std = Tensor::f_from_slice(&self.image_std)?
                .f_view([1, 3, 1, 1])?
                .f_to_device(device)?;
            pixel_values = pixel_values.f_sub(&mean)?.f_div(&std)?;
        }
        if self.do_pad && target_width < self.pad_size.width {
            pixel_values =
                pixel_values.f_constant_pad_nd([0, self.pad_size.width - target_width, 0, 0])?;
        }
        Ok(pixel_values)
    }

    pub(crate) fn postprocess(&self, probabilities: &Tensor) -> Result<TextRecognition> {
        let size = probabilities.size();
        if size.len() != 3 || size[0] != 1 {
            bail!("expected PP-OCRv6 recognition output [1, T, C], got {size:?}");
        }
        let (scores, indices) = probabilities.f_max_dim(-1, false)?;
        let scores = Vec::<f32>::try_from(
            &scores
                .f_to_device(Device::Cpu)?
                .f_to_kind(Kind::Float)?
                .f_view([-1])?,
        )?;
        let indices = Vec::<i64>::try_from(&indices.f_to_device(Device::Cpu)?.f_view([-1])?)?;
        let mut text = String::new();
        let mut selected_scores = Vec::new();
        for (position, (&index, &score)) in indices.iter().zip(&scores).enumerate() {
            if index == 0 || (position > 0 && index == indices[position - 1]) {
                continue;
            }
            let character = self
                .character_list
                .get(index as usize)
                .with_context(|| format!("PP-OCRv6 recognition emitted invalid token {index}"))?;
            text.push_str(character);
            selected_scores.push(score);
        }
        let score = if selected_scores.is_empty() {
            f32::NAN
        } else {
            selected_scores.iter().sum::<f32>() / selected_scores.len() as f32
        };
        Ok(TextRecognition { text, score })
    }

    fn target_width(&self, height: i64, width: i64) -> i64 {
        let default_ratio = self.size.width as f64 / self.size.height as f64;
        let ratio = (width as f64 / height as f64).max(default_ratio);
        let mut target_width = (self.size.height as f64 * ratio) as i64;
        if target_width > self.max_image_width {
            target_width = self.max_image_width;
        } else {
            let resized_width =
                (self.size.height as f64 * width as f64 / height as f64).ceil() as i64;
            if target_width >= resized_width {
                target_width = resized_width;
            }
        }
        target_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognition_width_matches_transformers() {
        let processor = PPOCRV6MediumRecImageProcessor::default();
        assert_eq!(processor.target_width(48, 100), 100);
        assert_eq!(processor.target_width(100, 100), 48);
        assert_eq!(processor.target_width(10, 1000), 3200);
    }
}
