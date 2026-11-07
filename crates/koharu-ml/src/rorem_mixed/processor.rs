//! RORem image and mask processing.
//!
//! The reference pipeline resizes both inputs to a square model resolution and
//! optionally dilates the mask before inference:
//! https://github.com/leeruibin/RORem/blob/891ab8a17bbcfb16a773c078cb256aae8cb30468/inference_RORem.py

use anyhow::Result;
use fast_image_resize::{FilterType, ResizeAlg};
use image::{GrayImage, Luma, RgbImage};
use imageproc::{distance_transform::Norm, morphology::dilate};

/// Pillow's default for a `resize` that names no filter, the reference script's
/// for both inputs.
const BICUBIC: ResizeAlg = ResizeAlg::Convolution(FilterType::CatmullRom);
const LANCZOS: ResizeAlg = ResizeAlg::Convolution(FilterType::Lanczos3);

/// The page at the model's square `resolution`.
pub(super) fn resize_image(image: &RgbImage, resolution: u32) -> Result<RgbImage> {
    crate::image::resize(image, resolution, resolution, BICUBIC)
}

/// The mask at the model's square `resolution`, made binary at half as the
/// reference pipeline's mask processor does after the resize, and grown by
/// `dilation` pixels there. The reference script's own dilation names an
/// undefined variable, so it has no behavior to follow.
pub(super) fn resize_mask(mask: &GrayImage, resolution: u32, dilation: u8) -> Result<GrayImage> {
    let mut output = crate::image::resize(mask, resolution, resolution, BICUBIC)?;
    for pixel in output.pixels_mut() {
        *pixel = Luma([if pixel.0[0] < 128 { 0 } else { 255 }]);
    }
    if dilation > 0 {
        output = dilate(&output, Norm::LInf, dilation);
    }
    Ok(output)
}

/// The generated square at `original`'s size, with `original`'s pixels
/// outside the mask.
pub(super) fn composite(
    original: &RgbImage,
    native_mask: &GrayImage,
    generated: &RgbImage,
) -> Result<RgbImage> {
    // The reference script returns its square model input size. Koharu restores
    // the caller's dimensions and exact unmasked pixels after the same square
    // inference so large-page inpainting does not silently shrink the page.
    let mut output = crate::image::resize(generated, original.width(), original.height(), LANCZOS)?;
    let mask = crate::image::resize(
        native_mask,
        original.width(),
        original.height(),
        ResizeAlg::Nearest,
    )?;
    for (index, &masked) in mask.as_raw().iter().enumerate() {
        if masked == 0 {
            let offset = index * 3;
            output.as_flat_samples_mut().samples[offset..offset + 3]
                .copy_from_slice(&original.as_raw()[offset..offset + 3]);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use image::{Rgb, imageops};

    use super::*;

    #[test]
    fn mask_is_binary_and_can_be_dilated() {
        let mut mask = GrayImage::new(512, 512);
        mask.put_pixel(256, 256, Luma([128]));

        let mask = resize_mask(&mask, 512, 1).unwrap();

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

        let output = composite(&original, &mask, &generated).unwrap();

        assert_eq!(output.dimensions(), original.dimensions());
        assert_eq!(output.get_pixel(0, 1080), &Rgb([10, 20, 30]));
        assert_eq!(output.get_pixel(3839, 1080), &Rgb([200, 210, 220]));
    }
}
