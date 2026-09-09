//! Hayai OCR image and text processing.
//!
//! Canonical image preprocessing is the SigLIP2 NaFlex image processor
//! (`Siglip2ImageProcessor`) with the released settings of
//! `google/siglip2-base-patch16-naflex`:
//! https://huggingface.co/google/siglip2-base-patch16-naflex/blob/main/preprocessor_config.json
//! https://github.com/huggingface/transformers/blob/main/src/transformers/models/siglip2/image_processing_siglip2.py

use std::path::Path;

use anyhow::{Context, Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, RgbImage};
use koharu_torch::{Device, Kind, Tensor};
use tokenizers::Tokenizer as HuggingFaceTokenizer;

use super::config::siglip2;

const RESCALE_FACTOR: f64 = 1.0 / 255.0;
const IMAGE_MEAN: [f32; 3] = [0.5, 0.5, 0.5];
const IMAGE_STD: [f32; 3] = [0.5, 0.5, 0.5];

#[derive(Debug)]
pub(super) struct ImageProcessor {
    mean: Tensor,
    std: Tensor,
    device: Device,
}

/// One preprocessed crop in NaFlex layout.
#[derive(Debug)]
pub(super) struct ImageInput {
    /// Patchified pixels `(1, MAX_NUM_PATCHES, channels * patch_size^2)`,
    /// zero-padded in the patch dimension exactly like the upstream processor.
    pub(super) pixel_values: Tensor,
    /// Patch padding mask `(1, MAX_NUM_PATCHES)`; `0` marks padded patches.
    pub(super) attention_mask: Tensor,
    /// Patch-grid dimensions of the unpadded image `(height_patches, width_patches)`.
    pub(super) spatial_shape: [i64; 2],
}

impl ImageProcessor {
    pub(super) fn new(device: Device) -> Result<Self> {
        let mean = Tensor::from_slice(&IMAGE_MEAN)
            .view([1, 3, 1, 1])
            .to_device(device);
        let std = Tensor::from_slice(&IMAGE_STD)
            .view([1, 3, 1, 1])
            .to_device(device);
        Ok(Self { mean, std, device })
    }

    pub(super) fn preprocess(&self, image: &DynamicImage) -> Result<ImageInput> {
        if image.width() == 0 || image.height() == 0 {
            anyhow::bail!("cannot recognize an empty image");
        }

        let patch_size = siglip2::PATCH_SIZE;
        let (height, width) = get_image_size_for_max_num_patches(
            i64::from(image.height()),
            i64::from(image.width()),
            patch_size,
            siglip2::MAX_NUM_PATCHES as i64,
        );
        let height = height as u32;
        let width = width as u32;
        let patch_height = (height / patch_size as u32) as i64;
        let patch_width = (width / patch_size as u32) as i64;
        let max_patches = siglip2::MAX_NUM_PATCHES as i64;
        let num_patches = patch_height * patch_width;
        ensure!(
            num_patches <= max_patches,
            "image resize produced {num_patches} patches above the {max_patches} limit"
        );

        // Upstream resizes with Pillow/torchvision bilinear filtering;
        // `fast_image_resize` uses the same triangle convolution but can
        // differ at coefficient rounding.
        let image = image.to_rgb8();
        let image = if image.width() == width && image.height() == height {
            image
        } else {
            resize_bilinear(&image, width, height)?
        };

        let mut pixel_values = Tensor::from_slice(image.as_raw())
            .view([1, i64::from(height), i64::from(width), 3])
            .permute([0, 3, 1, 2])
            .to_device(self.device)
            .to_kind(Kind::Float);
        pixel_values *= RESCALE_FACTOR;
        pixel_values = (&pixel_values - &self.mean) / &self.std;

        // (1, channels, height, width) -> (1, patches, patch_size^2 * channels)
        // with patch order (patch row, patch column, pixel row, pixel column).
        let patch = patch_size;
        let mut pixel_values = pixel_values
            .reshape([1, 3, patch_height, patch, patch_width, patch])
            .permute([0, 2, 4, 3, 5, 1])
            .reshape([1, num_patches, -1]);

        // The upstream processor pads every batch entry to `max_num_patches`;
        // those padded prefix tokens are consumed by the encoder and decoder.
        if num_patches < max_patches {
            let padding = Tensor::zeros(
                [1, max_patches - num_patches, pixel_values.size()[2]],
                (Kind::Float, self.device),
            );
            pixel_values = Tensor::cat(&[pixel_values, padding], 1);
        }

        let attention_mask = if num_patches < max_patches {
            Tensor::cat(
                &[
                    Tensor::ones([num_patches], (Kind::Int, self.device)),
                    Tensor::zeros([max_patches - num_patches], (Kind::Int, self.device)),
                ],
                0,
            )
        } else {
            Tensor::ones([max_patches], (Kind::Int, self.device))
        };

        Ok(ImageInput {
            pixel_values,
            attention_mask: attention_mask.view([1, max_patches]),
            spatial_shape: [patch_height, patch_width],
        })
    }
}

// Ported from `get_image_size_for_max_num_patches`: a binary search over the
// scale whose result is rounded up to whole patches on each axis.
fn get_image_size_for_max_num_patches(
    image_height: i64,
    image_width: i64,
    patch_size: i64,
    max_num_patches: i64,
) -> (i64, i64) {
    const EPS: f64 = 1e-5;
    let scaled_size = |scale: f64, size: i64| -> i64 {
        let scaled = ((size as f64 * scale) / patch_size as f64).ceil() as i64 * patch_size;
        scaled.max(patch_size)
    };

    let mut scale_min = EPS / 10.0;
    let mut scale_max = 100.0f64;
    while scale_max - scale_min >= EPS {
        let scale = (scale_min + scale_max) / 2.0;
        let target_height = scaled_size(scale, image_height);
        let target_width = scaled_size(scale, image_width);
        let num_patches = (target_height / patch_size) * (target_width / patch_size);
        if num_patches <= max_num_patches {
            scale_min = scale;
        } else {
            scale_max = scale;
        }
    }
    let scale = scale_min;
    (
        scaled_size(scale, image_height),
        scaled_size(scale, image_width),
    )
}

fn resize_bilinear(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    let mut resized = RgbImage::new(width, height);
    Resizer::new()
        .resize(
            image,
            &mut resized,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                .use_alpha(false),
        )
        .map_err(|error| anyhow::anyhow!("failed to resize Hayai OCR input: {error}"))?;
    Ok(resized)
}

#[derive(Debug)]
pub(crate) struct Tokenizer {
    inner: HuggingFaceTokenizer,
    bos_token_id: i64,
    eos_token_id: i64,
    pad_token_id: i64,
}

impl Tokenizer {
    pub(crate) fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
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

    pub(crate) fn len(&self) -> usize {
        self.inner.get_vocab_size(true)
    }

    pub(crate) fn bos_token_id(&self) -> i64 {
        self.bos_token_id
    }

    pub(crate) fn eos_token_id(&self) -> i64 {
        self.eos_token_id
    }

    pub(crate) fn pad_token_id(&self) -> i64 {
        self.pad_token_id
    }

    /// Decodes generated ids, dropping special tokens like the upstream
    /// `generate` cleanup pass.
    pub(crate) fn decode(&self, token_ids: &[i64]) -> Result<String> {
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

    #[test]
    fn resize_budget_matches_upstream_binary_search() {
        // A square image fills the 256-patch budget with a 16x16 patch grid.
        assert_eq!(
            get_image_size_for_max_num_patches(1024, 1024, 16, 256),
            (256, 256)
        );
        // A wide line keeps its rough aspect ratio while rounding up to patch
        // size and staying within the patch budget.
        let (height, width) = get_image_size_for_max_num_patches(120, 900, 16, 256);
        assert_eq!(height % 16, 0);
        assert_eq!(width % 16, 0);
        assert!((height / 16) * (width / 16) <= 256);
        assert_eq!(height, 96);
        assert_eq!(width, 672);
    }

    #[test]
    fn tiny_images_are_upscaled_to_fill_the_patch_budget() {
        assert_eq!(
            get_image_size_for_max_num_patches(4, 4, 16, 256),
            (256, 256)
        );
    }
}
