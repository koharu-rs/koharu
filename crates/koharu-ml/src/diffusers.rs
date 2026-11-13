//! diffusers' inpainting, which the flow-matching models here follow where
//! their own pipelines don't inpaint: FLUX.2 Klein's fit of the image under a
//! megapixel, `VaeImageProcessor`'s crop region, fill, and overlay, and the
//! step an img2img strength starts from.
//! https://github.com/huggingface/diffusers/blob/a37f6f8394ac2a7ee8360c3abea811efe54512b1/src/diffusers/pipelines/flux2/image_processor.py
//! https://github.com/huggingface/diffusers/blob/a37f6f8394ac2a7ee8360c3abea811efe54512b1/src/diffusers/image_processor.py

use anyhow::{Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg};
use image::{DynamicImage, GrayImage, Luma, RgbImage, imageops};

/// The token grid: the autoencoder's 8x downsampling, then 2x2 patches.
pub(crate) const TOKEN: u32 = 16;
/// The area FLUX.2 Klein's pipeline fits an image to inpaint under.
const AREA: u32 = 1024 * 1024;
pub(crate) const LANCZOS: ResizeAlg = ResizeAlg::Convolution(FilterType::Lanczos3);

/// A box as `(left, top, right, bottom)`.
pub(crate) type Region = (u32, u32, u32, u32);

/// The image and mask an inpainting starts from: the image fitted under 1
/// megapixel, then both resized to the token grid.
pub(crate) fn fit_inpaint(
    image: &DynamicImage,
    mask: &DynamicImage,
) -> Result<(RgbImage, GrayImage)> {
    let mut fitted = image.to_rgb8();
    if u64::from(fitted.width()) * u64::from(fitted.height()) > u64::from(AREA) {
        fitted = resize_to_target_area(&fitted, AREA)?;
    }
    let width = (fitted.width() / TOKEN) * TOKEN;
    let height = (fitted.height() / TOKEN) * TOKEN;
    ensure!(width > 0 && height > 0);
    Ok((
        crate::image::resize(&fitted, width, height, LANCZOS)?,
        crate::image::resize(&mask.to_luma8(), width, height, LANCZOS)?,
    ))
}

/// `_resize_to_target_area`: scaled to about `target_area` pixels, keeping the
/// aspect ratio.
pub(crate) fn resize_to_target_area(image: &RgbImage, target_area: u32) -> Result<RgbImage> {
    let scale =
        (f64::from(target_area) / (f64::from(image.width()) * f64::from(image.height()))).sqrt();
    let width = (f64::from(image.width()) * scale).floor().max(1.0) as u32;
    let height = (f64::from(image.height()) * scale).floor().max(1.0) as u32;
    crate::image::resize(image, width, height, LANCZOS)
}

/// `get_crop_region`: the mask's bounding box grown by `pad` and widened to
/// `width` x `height`'s aspect ratio, inside the mask.
pub(crate) fn crop_region(mask: &GrayImage, width: u32, height: u32, pad: u32) -> Option<Region> {
    let mut left = mask.width();
    let mut top = mask.height();
    let mut right = 0;
    let mut bottom = 0;
    for (x, y, pixel) in mask.enumerate_pixels() {
        if pixel.0[0] == 0 {
            continue;
        }
        left = left.min(x);
        top = top.min(y);
        right = right.max(x + 1);
        bottom = bottom.max(y + 1);
    }
    if right <= left || bottom <= top {
        return None;
    }

    let mut x1 = i64::from(left.saturating_sub(pad));
    let mut y1 = i64::from(top.saturating_sub(pad));
    let mut x2 = i64::from(right.saturating_add(pad).min(mask.width()));
    let mut y2 = i64::from(bottom.saturating_add(pad).min(mask.height()));
    let crop_ratio = (x2 - x1) as f64 / (y2 - y1) as f64;
    let processing_ratio = f64::from(width) / f64::from(height);

    if crop_ratio > processing_ratio {
        let desired_height = (x2 - x1) as f64 / processing_ratio;
        let difference = (desired_height - (y2 - y1) as f64) as i64;
        y1 -= difference / 2;
        y2 += difference - difference / 2;
        if y2 >= i64::from(mask.height()) {
            let overflow = y2 - i64::from(mask.height());
            y2 -= overflow;
            y1 -= overflow;
        }
        if y1 < 0 {
            y2 -= y1;
            y1 = 0;
        }
        if y2 >= i64::from(mask.height()) {
            y2 = i64::from(mask.height());
        }
    } else {
        let desired_width = (y2 - y1) as f64 * processing_ratio;
        let difference = (desired_width - (x2 - x1) as f64) as i64;
        x1 -= difference / 2;
        x2 += difference - difference / 2;
        if x2 >= i64::from(mask.width()) {
            let overflow = x2 - i64::from(mask.width());
            x2 -= overflow;
            x1 -= overflow;
        }
        if x1 < 0 {
            x2 -= x1;
            x1 = 0;
        }
        if x2 >= i64::from(mask.width()) {
            x2 = i64::from(mask.width());
        }
    }

    Some((
        u32::try_from(x1).ok()?,
        u32::try_from(y1).ok()?,
        u32::try_from(x2).ok()?,
        u32::try_from(y2).ok()?,
    ))
}

/// The image and mask cropped to `region` and resized to fill `width` x
/// `height`, or the whole of both without a region.
pub(crate) fn crop_to_region(
    image: &RgbImage,
    mask: &GrayImage,
    region: Option<Region>,
) -> Result<(RgbImage, GrayImage)> {
    let Some((x1, y1, x2, y2)) = region else {
        return Ok((image.clone(), mask.clone()));
    };
    let (width, height) = image.dimensions();
    let image_crop = imageops::crop_imm(image, x1, y1, x2 - x1, y2 - y1).to_image();
    let mask_crop =
        DynamicImage::ImageLuma8(imageops::crop_imm(mask, x1, y1, x2 - x1, y2 - y1).to_image());
    Ok((
        resize_and_fill(&DynamicImage::ImageRgb8(image_crop), width, height)?.to_rgb8(),
        resize_and_fill(&mask_crop, width, height)?.to_luma8(),
    ))
}

/// `_resize_and_fill`: resized to cover `width` x `height` on one side,
/// centred, the rest left black.
fn resize_and_fill(image: &DynamicImage, width: u32, height: u32) -> Result<DynamicImage> {
    let ratio = f64::from(width) / f64::from(height);
    let source_ratio = f64::from(image.width()) / f64::from(image.height());
    let resized_width = if ratio > source_ratio {
        width
    } else {
        image.width() * height / image.height()
    };
    let resized_height = if ratio <= source_ratio {
        height
    } else {
        image.height() * width / image.width()
    };
    let resized = crate::image::resize(&image.to_rgb8(), resized_width, resized_height, LANCZOS)?;
    let mut output = RgbImage::new(width, height);
    imageops::overlay(
        &mut output,
        &resized,
        i64::from((width - resized_width) / 2),
        i64::from((height - resized_height) / 2),
    );
    Ok(DynamicImage::ImageRgb8(output))
}

/// `_resize_and_crop`, without the resize its callers here never need: the
/// centre `width` x `height` of `image`.
pub(crate) fn resize_and_crop(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    ensure!(
        image.width() >= width && image.height() >= height,
        "cannot center-crop {}x{} image to {}x{}",
        image.width(),
        image.height(),
        width,
        height
    );
    let left = (image.width() - width) / 2;
    let top = (image.height() - height) / 2;
    Ok(imageops::crop_imm(image, left, top, width, height).to_image())
}

/// The mask black below half and white from half up.
pub(crate) fn binarize(mask: &mut GrayImage) {
    for pixel in mask.pixels_mut() {
        *pixel = Luma([if pixel.0[0] < 128 { 0 } else { 255 }]);
    }
}

/// `apply_overlay`: the generated image placed back into the crop `region` of
/// `init_image` and blended into it by `mask`.
pub(crate) fn apply_overlay(
    mask: &GrayImage,
    init_image: &RgbImage,
    image: RgbImage,
    region: Option<Region>,
) -> Result<RgbImage> {
    let image = if let Some((x1, y1, x2, y2)) = region {
        let crop = resize_and_crop(&image, x2 - x1, y2 - y1)?;
        let mut output = RgbImage::new(init_image.width(), init_image.height());
        imageops::overlay(&mut output, &crop, i64::from(x1), i64::from(y1));
        output
    } else {
        image
    };

    ensure!(image.dimensions() == init_image.dimensions());
    ensure!(mask.dimensions() == init_image.dimensions());
    let mut output = image;
    for (x, y, pixel) in output.enumerate_pixels_mut() {
        let amount = f32::from(mask.get_pixel(x, y).0[0]) / 255.0;
        let original = init_image.get_pixel(x, y).0;
        for (channel, original) in original.iter().enumerate() {
            pixel.0[channel] = (f32::from(pixel.0[channel]) * amount
                + f32::from(*original) * (1.0 - amount))
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    Ok(output)
}

/// The strength stable-diffusion.cpp takes for diffusers' `strength` over
/// `steps`. diffusers' `get_timesteps` keeps the steps after
/// `int(steps - steps * strength)`, in that order of float operations; sd.cpp
/// runs `int(steps * its strength) + 1` steps, its product in `float`, so it's
/// given the middle of the fraction that truncates to one fewer than diffusers
/// keeps, which no rounding moves.
pub(crate) fn strength(strength: f64, steps: usize) -> f32 {
    if strength >= 1.0 {
        return 1.0;
    }
    let init_timestep = (steps as f64 * strength).min(steps as f64);
    let kept = steps - (steps as f64 - init_timestep).max(0.0) as usize;
    ((kept as f64 - 0.5) / steps as f64) as f32
}

#[cfg(test)]
mod tests {
    use image::{GrayImage, Luma, Rgb, RgbImage};

    use super::{apply_overlay, strength};

    /// The steps sd.cpp runs from an img2img strength, as `image.cpp` at
    /// 58bb3c1 counts them: `t_enc + 1` steps, `t_enc` truncated from
    /// `steps * strength` in `float`.
    fn sd_cpp_steps(strength: f32, steps: usize) -> usize {
        if strength >= 1.0 {
            return steps;
        }
        let t_enc = (steps as f32 * strength) as usize;
        t_enc.min(steps - 1) + 1
    }

    /// The steps diffusers' `get_timesteps` keeps.
    fn diffusers_steps(strength: f64, steps: usize) -> usize {
        let init_timestep = (steps as f64 * strength).min(steps as f64);
        let t_start = (steps as f64 - init_timestep).max(0.0) as usize;
        steps - t_start
    }

    #[test]
    fn strength_runs_the_steps_diffusers_keeps() {
        for steps in [1, 2, 3, 4, 6, 8, 20, 28, 50] {
            for tenths in 1..=10 {
                let diffusers = f64::from(tenths) / 10.0;
                assert_eq!(
                    sd_cpp_steps(strength(diffusers, steps), steps),
                    diffusers_steps(diffusers, steps),
                    "strength {diffusers} over {steps} steps"
                );
            }
        }
    }

    #[test]
    fn overlay_without_a_crop_preserves_unmasked_pixels() {
        let initial = RgbImage::from_pixel(2, 1, Rgb([10, 20, 30]));
        let generated = RgbImage::from_pixel(2, 1, Rgb([200, 210, 220]));
        let mut mask = GrayImage::new(2, 1);
        mask.put_pixel(1, 0, Luma([u8::MAX]));

        let output = apply_overlay(&mask, &initial, generated, None).unwrap();

        assert_eq!(output.get_pixel(0, 0), initial.get_pixel(0, 0));
        assert_eq!(output.get_pixel(1, 0), &Rgb([200, 210, 220]));
    }
}
