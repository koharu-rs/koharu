//! RORem image and mask processing.
//!
//! The reference pipeline resizes both inputs to a square model resolution and
//! optionally dilates the mask before inference:
//! https://github.com/leeruibin/RORem/blob/891ab8a17bbcfb16a773c078cb256aae8cb30468/inference_RORem.py

use anyhow::{Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GenericImageView, GrayImage, Luma, RgbImage};
use imageproc::{distance_transform::Norm, morphology::dilate};

#[derive(Debug, Clone, PartialEq)]
pub struct RoremMixedOptions {
    /// Square inference resolution. RORem mixed was trained at 512 and 1024.
    pub resolution: u32,
    /// Mask growth in model-space pixels.
    pub mask_dilation: u8,
    pub num_inference_steps: i32,
    pub guidance_scale: f32,
    pub strength: f32,
    pub seed: i64,
}

impl Default for RoremMixedOptions {
    fn default() -> Self {
        Self {
            resolution: 512,
            mask_dilation: 0,
            num_inference_steps: 30,
            guidance_scale: 8.0,
            strength: 0.999,
            seed: -1,
        }
    }
}

pub(super) struct Processor;

impl Processor {
    pub(super) fn validate(
        image: &DynamicImage,
        mask: &GrayImage,
        prompt: &str,
        negative_prompt: &str,
        options: &RoremMixedOptions,
    ) -> Result<()> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "image dimensions must be non-zero"
        );
        ensure!(
            image.dimensions() == mask.dimensions(),
            "image and mask dimensions differ: image={:?}, mask={:?}",
            image.dimensions(),
            mask.dimensions()
        );
        ensure!(
            matches!(options.resolution, 512 | 1024),
            "RORem mixed resolution must be 512 or 1024"
        );
        ensure!(
            options.num_inference_steps > 0,
            "num_inference_steps must be greater than zero"
        );
        ensure!(
            options.guidance_scale.is_finite() && options.guidance_scale > 0.0,
            "guidance_scale must be finite and greater than zero"
        );
        ensure!(
            options.strength.is_finite() && options.strength > 0.0 && options.strength < 1.0,
            "RORem mixed strength must be finite, greater than zero, and less than one"
        );
        ensure!(
            !prompt.contains('\0'),
            "prompt contains an interior NUL byte"
        );
        ensure!(
            !negative_prompt.contains('\0'),
            "negative prompt contains an interior NUL byte"
        );
        Ok(())
    }

    pub(super) fn resize_image(image: &RgbImage, resolution: u32) -> Result<RgbImage> {
        if image.dimensions() == (resolution, resolution) {
            return Ok(image.clone());
        }
        let mut output = RgbImage::new(resolution, resolution);
        Resizer::new().resize(
            image,
            &mut output,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
                .use_alpha(false),
        )?;
        Ok(output)
    }

    pub(super) fn resize_mask(
        mask: &GrayImage,
        resolution: u32,
        dilation: u8,
    ) -> Result<GrayImage> {
        let mut binary = mask.clone();
        for pixel in binary.pixels_mut() {
            *pixel = Luma([if pixel.0[0] < 128 { 0 } else { 255 }]);
        }
        let mut output = if binary.dimensions() == (resolution, resolution) {
            binary
        } else {
            let mut output = GrayImage::new(resolution, resolution);
            Resizer::new().resize(
                &binary,
                &mut output,
                &ResizeOptions::new()
                    .resize_alg(ResizeAlg::Nearest)
                    .use_alpha(false),
            )?;
            output
        };
        if dilation > 0 {
            output = dilate(&output, Norm::LInf, dilation);
        }
        Ok(output)
    }

    pub(super) fn composite(
        original: &RgbImage,
        native_mask: &GrayImage,
        generated: &RgbImage,
    ) -> Result<RgbImage> {
        // The reference script returns its square model input size. Koharu restores
        // the caller's dimensions and exact unmasked pixels after the same square
        // inference so large-page inpainting does not silently shrink the page.
        let generated = Self::resize_rgb(generated, original.width(), original.height())?;
        let mask = Self::resize_gray(native_mask, original.width(), original.height())?;
        let mut output = generated;
        for (index, &masked) in mask.as_raw().iter().enumerate() {
            if masked == 0 {
                let offset = index * 3;
                output.as_flat_samples_mut().samples[offset..offset + 3]
                    .copy_from_slice(&original.as_raw()[offset..offset + 3]);
            }
        }
        Ok(output)
    }

    fn resize_rgb(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
        if image.dimensions() == (width, height) {
            return Ok(image.clone());
        }
        let mut output = RgbImage::new(width, height);
        Resizer::new().resize(
            image,
            &mut output,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
                .use_alpha(false),
        )?;
        Ok(output)
    }

    fn resize_gray(image: &GrayImage, width: u32, height: u32) -> Result<GrayImage> {
        if image.dimensions() == (width, height) {
            return Ok(image.clone());
        }
        let mut output = GrayImage::new(width, height);
        Resizer::new().resize(
            image,
            &mut output,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Nearest)
                .use_alpha(false),
        )?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, imageops};

    use super::*;

    #[test]
    fn defaults_match_converted_model_card() {
        assert_eq!(
            RoremMixedOptions::default(),
            RoremMixedOptions {
                resolution: 512,
                mask_dilation: 0,
                num_inference_steps: 30,
                guidance_scale: 8.0,
                strength: 0.999,
                seed: -1,
            }
        );
    }

    #[test]
    fn mask_is_binary_and_can_be_dilated() {
        let mut mask = GrayImage::new(512, 512);
        mask.put_pixel(256, 256, Luma([128]));

        let mask = Processor::resize_mask(&mask, 512, 1).unwrap();

        assert_eq!(mask.get_pixel(255, 255), &Luma([255]));
        assert_eq!(mask.get_pixel(256, 256), &Luma([255]));
        assert_eq!(mask.get_pixel(257, 257), &Luma([255]));
        assert_eq!(mask.get_pixel(258, 258), &Luma([0]));
    }

    #[test]
    fn compositing_restores_original_size_and_unmasked_pixels() {
        let original = RgbImage::from_pixel(3840, 2160, Rgb([10, 20, 30]));
        let generated = RgbImage::from_pixel(512, 512, Rgb([200, 210, 220]));
        let mut mask = GrayImage::new(512, 512);
        imageops::replace(
            &mut mask,
            &GrayImage::from_pixel(256, 512, Luma([255])),
            256,
            0,
        );

        let output = Processor::composite(&original, &mask, &generated).unwrap();

        assert_eq!(output.dimensions(), original.dimensions());
        assert_eq!(output.get_pixel(0, 1080), &Rgb([10, 20, 30]));
        assert_eq!(output.get_pixel(3839, 1080), &Rgb([200, 210, 220]));
    }
}
