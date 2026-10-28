//! Transformers-compatible PaddleOCR-VL image and text processing.
//!
//! Original implementations:
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/paddleocr_vl/image_processing_paddleocr_vl.py
//! https://github.com/huggingface/transformers/blob/63f32a8782cb70da3365acab16f2b67947737985/src/transformers/models/paddleocr_vl/processing_paddleocr_vl.py

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use image::DynamicImage;
use koharu_torch::{Device, Kind, Tensor};
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaddleOCRVLTask {
    Ocr,
    Table,
    Formula,
    Chart,
    Spotting,
    Seal,
}

impl PaddleOCRVLTask {
    pub(crate) fn prompt(self) -> &'static str {
        match self {
            Self::Ocr => "OCR:",
            Self::Table => "Table Recognition:",
            Self::Formula => "Formula Recognition:",
            Self::Chart => "Chart Recognition:",
            Self::Spotting => "Spotting:",
            Self::Seal => "Seal Recognition:",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaddleOCRVLResult {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PaddleOCRVLImageProcessor {
    pub do_convert_rgb: bool,
    pub do_normalize: bool,
    pub do_rescale: bool,
    pub do_resize: bool,
    pub image_mean: Vec<f32>,
    pub image_std: Vec<f32>,
    pub max_pixels: i64,
    pub min_pixels: i64,
    pub merge_size: i64,
    pub patch_size: i64,
    pub rescale_factor: f64,
    pub resample: i64,
    pub temporal_patch_size: i64,
}

impl Default for PaddleOCRVLImageProcessor {
    fn default() -> Self {
        Self {
            do_convert_rgb: true,
            do_normalize: true,
            do_rescale: true,
            do_resize: true,
            image_mean: vec![0.5, 0.5, 0.5],
            image_std: vec![0.5, 0.5, 0.5],
            max_pixels: 1_003_520,
            min_pixels: 112_896,
            merge_size: 2,
            patch_size: 14,
            rescale_factor: 1.0 / 255.0,
            resample: 3,
            temporal_patch_size: 1,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Processor {
    image_processor: PaddleOCRVLImageProcessor,
    tokenizer: Tokenizer,
    image_token_id: i64,
}

impl Processor {
    pub(crate) fn from_files(
        processor_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
        image_token_id: i64,
    ) -> Result<Self> {
        let processor_path = processor_path.as_ref();
        let tokenizer_path = tokenizer_path.as_ref();
        let image_processor = serde_json::from_str(&std::fs::read_to_string(processor_path)?)
            .with_context(|| format!("failed to parse {}", processor_path.display()))?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .with_context(|| format!("failed to parse {}", tokenizer_path.display()))?;
        Ok(Self {
            image_processor,
            tokenizer,
            image_token_id,
        })
    }

    pub(crate) fn preprocess(
        &self,
        image: &DynamicImage,
        task: PaddleOCRVLTask,
        device: Device,
    ) -> Result<(Tensor, [i64; 3])> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "image dimensions must be non-zero"
        );
        let image =
            if task == PaddleOCRVLTask::Spotting && image.width() < 1500 && image.height() < 1500 {
                image.resize_exact(
                    image.width() * 2,
                    image.height() * 2,
                    image::imageops::FilterType::Lanczos3,
                )
            } else {
                image.clone()
            };
        let rgb = image.to_rgb8();
        let max_pixels = if task == PaddleOCRVLTask::Spotting {
            2048 * 28 * 28
        } else {
            self.image_processor.max_pixels
        };
        let (height, width) = smart_resize(
            i64::from(rgb.height()),
            i64::from(rgb.width()),
            self.image_processor.patch_size * self.image_processor.merge_size,
            self.image_processor.min_pixels,
            max_pixels,
        )?;

        let mut pixels = Tensor::from_slice(rgb.as_raw())?
            .view([1, i64::from(rgb.height()), i64::from(rgb.width()), 3])?
            .permute([0, 3, 1, 2])?;
        if self.image_processor.do_resize {
            if self.image_processor.resample != 3 {
                bail!("PaddleOCR-VL only supports Transformers' bicubic resampling mode");
            }
            // Torch does not implement antialiased uint8 bicubic resize on CUDA.
            // Match Transformers on CPU, then make the only host-to-device copy.
            pixels = pixels.internal_upsample_bicubic2d_aa(
                [height, width],
                false,
                None::<f64>,
                None::<f64>,
            )?;
        }
        pixels = pixels.to_device(device)?.to_kind(Kind::Float)?;
        if self.image_processor.do_rescale {
            pixels = (pixels * self.image_processor.rescale_factor)?;
        }
        if self.image_processor.do_normalize {
            ensure!(
                self.image_processor.image_mean.len() == 3
                    && self.image_processor.image_std.len() == 3,
                "PaddleOCR-VL image_mean and image_std must contain three values"
            );
            let mean = Tensor::from_slice(&self.image_processor.image_mean)?
                .view([1, 3, 1, 1])?
                .to_device(device)?;
            let std = Tensor::from_slice(&self.image_processor.image_std)?
                .view([1, 3, 1, 1])?
                .to_device(device)?;
            pixels = ((pixels - mean)? / std)?;
        }

        let grid_t = 1;
        let grid_h = height / self.image_processor.patch_size;
        let grid_w = width / self.image_processor.patch_size;
        let patch = self.image_processor.patch_size;
        let pixel_values = pixels
            .view(&[1, grid_t, 3, grid_h, patch, grid_w, patch][..])?
            .permute([0, 1, 3, 5, 2, 4, 6])?
            .reshape([grid_t * grid_h * grid_w, 3, patch, patch])?;

        Ok((pixel_values, [grid_t, grid_h, grid_w]))
    }

    pub(crate) fn encode_prompt(
        &self,
        task: PaddleOCRVLTask,
        grid: [i64; 3],
    ) -> Result<(Vec<i64>, Vec<i64>)> {
        let image_tokens = grid.into_iter().product::<i64>()
            / (self.image_processor.merge_size * self.image_processor.merge_size);
        let prompt = format!(
            "<|begin_of_sentence|>User: <|IMAGE_START|>{}<|IMAGE_END|>{}\nAssistant:\n",
            "<|IMAGE_PLACEHOLDER|>".repeat(image_tokens as usize),
            task.prompt(),
        );
        let encoding = self
            .tokenizer
            .encode(prompt, false)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let input_ids = encoding
            .get_ids()
            .iter()
            .map(|&id| i64::from(id))
            .collect::<Vec<_>>();
        let mm_token_type_ids = input_ids
            .iter()
            .map(|&id| i64::from(id == self.image_token_id))
            .collect();
        Ok((input_ids, mm_token_type_ids))
    }

    pub(crate) fn decode(&self, token_ids: &[i64]) -> Result<PaddleOCRVLResult> {
        let token_ids = token_ids
            .iter()
            .map(|&id| u32::try_from(id))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let text = self
            .tokenizer
            .decode(&token_ids, false)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(PaddleOCRVLResult { text })
    }
}

fn smart_resize(
    mut height: i64,
    mut width: i64,
    factor: i64,
    min_pixels: i64,
    max_pixels: i64,
) -> Result<(i64, i64)> {
    if height < factor {
        width = (width as f64 * factor as f64 / height as f64).round_ties_even() as i64;
        height = factor;
    }
    if width < factor {
        height = (height as f64 * factor as f64 / width as f64).round_ties_even() as i64;
        width = factor;
    }
    ensure!(
        height.max(width) as f64 / height.min(width) as f64 <= 200.0,
        "absolute aspect ratio must be smaller than 200"
    );
    let mut resized_height = ((height as f64 / factor as f64).round_ties_even() as i64) * factor;
    let mut resized_width = ((width as f64 / factor as f64).round_ties_even() as i64) * factor;
    if resized_height * resized_width > max_pixels {
        let beta = ((height * width) as f64 / max_pixels as f64).sqrt();
        resized_height =
            factor.max(((height as f64 / beta / factor as f64).floor() as i64) * factor);
        resized_width = factor.max(((width as f64 / beta / factor as f64).floor() as i64) * factor);
    } else if resized_height * resized_width < min_pixels {
        let beta = (min_pixels as f64 / (height * width) as f64).sqrt();
        resized_height = ((height as f64 * beta / factor as f64).ceil() as i64) * factor;
        resized_width = ((width as f64 * beta / factor as f64).ceil() as i64) * factor;
    }
    Ok((resized_height, resized_width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_resize_matches_transformers_fixture_shapes() -> Result<()> {
        let cases = [
            ((178, 99), (476, 252)),
            ((251, 188), (392, 308)),
            ((103, 62), (448, 280)),
            ((115, 406), (196, 644)),
            ((313, 69), (728, 168)),
            ((54, 204), (196, 672)),
        ];
        for ((height, width), expected) in cases {
            assert_eq!(
                smart_resize(height, width, 28, 112_896, 1_003_520)?,
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn smart_resize_rejects_extreme_aspect_ratios() {
        assert!(smart_resize(28, 28 * 201, 28, 112_896, 1_003_520).is_err());
    }
}
