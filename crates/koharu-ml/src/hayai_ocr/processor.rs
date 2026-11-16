//! Hayai OCR image and text processing.
//!
//! Image preprocessing is Hayai's own `HayaiImageProcessor` with its released
//! settings:
//! https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2.7-supernova/blob/517bdeeb64b68c08939e12f7211e99943c1e1640/processing_hayai.py
//! https://huggingface.co/JustANormalTinkerer/hayai-ocr-v2.7-supernova/blob/517bdeeb64b68c08939e12f7211e99943c1e1640/preprocessor_config.json

use std::path::Path;

use anyhow::{Context, Result};
use fast_image_resize::{FilterType, ResizeAlg};
use image::DynamicImage;
use koharu_torch::{Device, Kind, Tensor};
use tokenizers::Tokenizer as HuggingFaceTokenizer;

use super::config::siglip2;

/// Pillow's bilinear filter, as convolution.
const RESIZE: ResizeAlg = ResizeAlg::Convolution(FilterType::Bilinear);
/// The patch budget, which the model card recommends.
const MAX_PATCHES: i64 = 256;
/// The fewest patches a crop is upscaled to.
const MIN_PATCHES: i64 = 64;
/// How far a crop is upscaled toward the budget before `MIN_PATCHES` rules.
const MAX_SCALE: f64 = 4.0;

#[derive(Debug)]
pub(super) struct ImageProcessor {
    device: Device,
}

/// One preprocessed crop in NaFlex layout.
#[derive(Debug)]
pub(super) struct ImageInput {
    /// Patchified pixels `(patches, channels * patch_size^2)`; generation
    /// pads them to the batch's longest crop, as upstream does.
    pub(super) pixel_values: Tensor,
    /// Patch-grid dimensions of the image `(height_patches, width_patches)`.
    pub(super) spatial_shape: [i64; 2],
}

impl ImageProcessor {
    pub(super) fn new(device: Device) -> Self {
        Self { device }
    }

    pub(super) fn preprocess(&self, image: &DynamicImage) -> Result<ImageInput> {
        if image.width() == 0 || image.height() == 0 {
            anyhow::bail!("cannot recognize an empty image");
        }

        let patch_size = siglip2::PATCH_SIZE;
        let (height, width) = target_size(i64::from(image.height()), i64::from(image.width()));
        let patch_height = height / patch_size;
        let patch_width = width / patch_size;
        let (height, width) = (height as u32, width as u32);

        // Upstream resizes with Pillow's bilinear filter; `fast_image_resize`
        // uses the same triangle convolution but can differ at coefficient
        // rounding.
        let image = image.to_rgb8();
        let image = if image.width() == width && image.height() == height {
            image
        } else {
            crate::image::resize(&image, width, height, RESIZE)?
        };

        // (height, width, channels) -> (patches, patch_size^2 * channels) with
        // patch order (patch row, patch column, pixel row, pixel column), then
        // [0, 255] to [-1, 1].
        let patch = patch_size;
        let pixel_values = Tensor::from_slice(image.as_raw())?
            .to(self.device)?
            .reshape([patch_height, patch, patch_width, patch, 3])?
            .permute([0, 2, 1, 3, 4])?
            .reshape([patch_height * patch_width, -1])?
            .to_kind(Kind::Float)?
            .mul_scalar(2.0 / 255.0)?
            .sub_scalar(1.0, 1)?;

        Ok(ImageInput {
            pixel_values,
            spatial_shape: [patch_height, patch_width],
        })
    }
}

/// Upstream `_naflex_target_size`: the largest scale, up to `MAX_SCALE`,
/// whose size rounded up to whole patches fits `MAX_PATCHES`, and for a crop
/// that leaves under `MIN_PATCHES`, the largest that fits `MIN_PATCHES`.
fn target_size(height: i64, width: i64) -> (i64, i64) {
    const EPS: f64 = 1e-5;
    let patch_size = siglip2::PATCH_SIZE;
    let scaled = |scale: f64, size: i64| -> i64 {
        ((size as f64 * scale / patch_size as f64).ceil() as i64 * patch_size).max(patch_size)
    };
    let search = |mut low: f64, mut high: f64, budget: i64| -> f64 {
        while high - low >= EPS {
            let scale = 0.5 * (low + high);
            let patches = scaled(scale, height) / patch_size * (scaled(scale, width) / patch_size);
            if patches <= budget {
                low = scale;
            } else {
                high = scale;
            }
        }
        low
    };

    let mut scale = search(EPS, MAX_SCALE, MAX_PATCHES);
    if scaled(scale, height) / patch_size * (scaled(scale, width) / patch_size) < MIN_PATCHES {
        scale = search(scale, 100.0, MIN_PATCHES.min(MAX_PATCHES));
    }
    (scaled(scale, height), scaled(scale, width))
}

#[derive(Debug)]
pub(super) struct Tokenizer {
    inner: HuggingFaceTokenizer,
    bos_token_id: i64,
    eos_token_id: i64,
    pad_token_id: i64,
}

impl Tokenizer {
    pub(super) fn from_file(path: &Path) -> Result<Self> {
        let inner = HuggingFaceTokenizer::from_file(path)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let id = |token: &str| -> Result<i64> {
            inner
                .token_to_id(token)
                .map(i64::from)
                .ok_or_else(|| anyhow::anyhow!("Hayai OCR tokenizer lacks a {token} token"))
        };
        Ok(Self {
            bos_token_id: id("<bos>")?,
            eos_token_id: id("<eos>")?,
            pad_token_id: id("<pad>")?,
            inner,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.inner.get_vocab_size(true)
    }

    pub(super) fn bos_token_id(&self) -> i64 {
        self.bos_token_id
    }

    pub(super) fn eos_token_id(&self) -> i64 {
        self.eos_token_id
    }

    pub(super) fn pad_token_id(&self) -> i64 {
        self.pad_token_id
    }

    /// Decodes generated ids, dropping special tokens like the upstream
    /// `generate` cleanup pass.
    pub(super) fn decode(&self, token_ids: &[i64]) -> Result<String> {
        let token_ids = token_ids
            .iter()
            .copied()
            .filter(|&id| id != self.eos_token_id && id != self.pad_token_id)
            .map(u32::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.inner
            .decode(&token_ids, true)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected sizes are upstream `_naflex_target_size`'s on the same inputs.

    #[test]
    fn resize_budget_matches_upstream_binary_search() {
        // A square image fills the 256-patch budget with a 16x16 patch grid.
        assert_eq!(target_size(1024, 1024), (256, 256));
        // A wide line keeps its rough aspect ratio while rounding up to patch
        // size and staying within the patch budget.
        assert_eq!(target_size(120, 900), (96, 672));
    }

    #[test]
    fn upscaling_stops_at_four_times() {
        assert_eq!(target_size(32, 32), (128, 128));
    }

    #[test]
    fn tiny_images_are_upscaled_to_the_minimum_patch_count() {
        assert_eq!(target_size(4, 4), (128, 128));
    }
}
