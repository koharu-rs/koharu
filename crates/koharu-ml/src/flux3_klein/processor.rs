//! FLUX.3 Image Klein image preprocessing.
//!
//! References follow diffusers' `Flux3ImageProcessor`, which follows Black
//! Forest Labs' reference implementation:
//! https://github.com/huggingface/diffusers/blob/86ef420b17363c28e36ebad6b58555e9aa8feda2/src/diffusers/modular_pipelines/flux3/image_processor.py
//! https://github.com/black-forest-labs/flux-3-image-klein/blob/62e90b9278b73ecc52927770c6965dc6b1966498/src/flux3/pipeline.py#L129-L171
//! Neither inpaints, so inpainting keeps diffusers' FLUX.2 Klein helpers:
//! https://github.com/huggingface/diffusers/blob/a37f6f8394ac2a7ee8360c3abea811efe54512b1/src/diffusers/pipelines/flux2/image_processor.py

use anyhow::{Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg};
use image::{DynamicImage, GrayImage, Luma, RgbImage, imageops};

/// The token grid: the autoencoder's 8x downsampling, then 2x2 patches.
const TOKEN: u32 = 16;
const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 4096;
/// The area FLUX.2 Klein's pipeline fits several references and an image to
/// inpaint under.
const REFERENCE_AREA: u32 = 1024 * 1024;
const LANCZOS: ResizeAlg = ResizeAlg::Convolution(FilterType::Lanczos3);

/// A box as `(left, top, right, bottom)`.
pub(super) type Region = (u32, u32, u32, u32);

/// The references a generation conditions on. One is kept whole: fitted
/// under 4 megapixels and padded to the token grid with its reflection, with
/// the box the result is cropped back to. Several are each fitted under 1
/// megapixel and center-cropped.
pub(super) fn references(images: &[DynamicImage]) -> Result<(Vec<RgbImage>, Option<Region>)> {
    if let [image] = images {
        let image = resize_if_canvas_exceeds_area(&image.to_rgb8())?;
        let (width, height) = canvas_size(image.width(), image.height());
        let (canvas, region) = reflect_fill(&image, width, height);
        return Ok((vec![canvas], Some(region)));
    }
    let references = images
        .iter()
        .map(fit_reference)
        .collect::<Result<Vec<_>>>()?;
    Ok((references, None))
}

/// One of several references, or the reference image of an inpainting:
/// fitted under 1 megapixel and center-cropped to the token grid.
pub(super) fn fit_reference(image: &DynamicImage) -> Result<RgbImage> {
    let mut image = image.to_rgb8();
    if u64::from(image.width()) * u64::from(image.height()) > u64::from(REFERENCE_AREA) {
        image = resize_to_target_area(&image, REFERENCE_AREA)?;
    }
    let width = (image.width() / TOKEN) * TOKEN;
    let height = (image.height() / TOKEN) * TOKEN;
    ensure!(width > 0 && height > 0);
    resize_and_crop(&image, width, height)
}

/// The image and mask an inpainting starts from: the image fitted under 1
/// megapixel, then both resized to the token grid.
pub(super) fn fit_inpaint(
    image: &DynamicImage,
    mask: &DynamicImage,
) -> Result<(RgbImage, GrayImage)> {
    let mut fitted = image.to_rgb8();
    if u64::from(fitted.width()) * u64::from(fitted.height()) > u64::from(REFERENCE_AREA) {
        fitted = resize_to_target_area(&fitted, REFERENCE_AREA)?;
    }
    let width = (fitted.width() / TOKEN) * TOKEN;
    let height = (fitted.height() / TOKEN) * TOKEN;
    ensure!(width > 0 && height > 0);
    Ok((
        crate::image::resize(&fitted, width, height, LANCZOS)?,
        crate::image::resize(&mask.to_luma8(), width, height, LANCZOS)?,
    ))
}

/// `_get_canvas_height_width`: the size rounded up to the token grid, sides
/// clamped to 64..=4096.
fn canvas_size(width: u32, height: u32) -> (u32, u32) {
    let canvas = |side: u32| (side.div_ceil(TOKEN) * TOKEN).clamp(MIN_SIDE, MAX_SIDE);
    (canvas(width), canvas(height))
}

/// `_resize_if_canvas_exceeds_area`: downscaled, keeping the aspect ratio,
/// until its canvas fits 4 megapixels and its sides 4096 pixels.
fn resize_if_canvas_exceeds_area(image: &RgbImage) -> Result<RgbImage> {
    const TARGET_AREA: f64 = 4.0 * 1024.0 * 1024.0;
    let (width, height) = (f64::from(image.width()), f64::from(image.height()));
    let mut scale = 1f64
        .min((TARGET_AREA / (width * height)).sqrt())
        .min(f64::from(MAX_SIDE) / width.max(height));
    let (fit_width, fit_height) = loop {
        // Python's round(), ties to even.
        let fit = |side: f64| ((side * scale).round_ties_even() as u32).max(1);
        let (fit_width, fit_height) = (fit(width), fit(height));
        let (canvas_width, canvas_height) = canvas_size(fit_width, fit_height);
        if f64::from(canvas_width) * f64::from(canvas_height) <= TARGET_AREA {
            break (fit_width, fit_height);
        }
        scale *= 0.995;
    };
    crate::image::resize(image, fit_width, fit_height, LANCZOS)
}

/// FLUX.3's `_resize_and_fill`: `image` centred on a `width` x `height`
/// canvas, the rest filled with its reflection, and the box it occupies.
fn reflect_fill(image: &RgbImage, width: u32, height: u32) -> (RgbImage, Region) {
    let left = (width - image.width()) / 2;
    let top = (height - image.height()) / 2;
    // `np.pad`'s "reflect" leaves the edge pixel out of the mirror; an
    // image no larger than a token is padded "symmetric", edge included.
    let symmetric = image.width().min(image.height()) <= TOKEN;
    let source = |position: i64, size: u32| -> u32 {
        let size = i64::from(size);
        if size == 1 {
            return 0;
        }
        let period = if symmetric { 2 * size } else { 2 * size - 2 };
        let position = position.rem_euclid(period);
        (if position < size {
            position
        } else if symmetric {
            period - 1 - position
        } else {
            period - position
        }) as u32
    };
    let canvas = RgbImage::from_fn(width, height, |x, y| {
        *image.get_pixel(
            source(i64::from(x) - i64::from(left), image.width()),
            source(i64::from(y) - i64::from(top), image.height()),
        )
    });
    (
        canvas,
        (left, top, left + image.width(), top + image.height()),
    )
}

/// diffusers' `_resize_to_target_area`: scaled to about `target_area`
/// pixels, keeping the aspect ratio.
fn resize_to_target_area(image: &RgbImage, target_area: u32) -> Result<RgbImage> {
    let scale =
        (f64::from(target_area) / (f64::from(image.width()) * f64::from(image.height()))).sqrt();
    let width = (f64::from(image.width()) * scale).floor().max(1.0) as u32;
    let height = (f64::from(image.height()) * scale).floor().max(1.0) as u32;
    crate::image::resize(image, width, height, LANCZOS)
}

/// diffusers' `get_crop_region`: the mask's bounding box grown by `pad` and
/// widened to `width` x `height`'s aspect ratio, inside the mask.
/// https://github.com/huggingface/diffusers/blob/a37f6f8394ac2a7ee8360c3abea811efe54512b1/src/diffusers/image_processor.py#L288-L375
pub(super) fn crop_region(mask: &GrayImage, width: u32, height: u32, pad: u32) -> Option<Region> {
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
pub(super) fn crop_to_region(
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

/// diffusers' `_resize_and_fill`: resized to cover `width` x `height` on one
/// side, centred, the rest left black.
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

/// diffusers' `_resize_and_crop`, without the resize its callers here never
/// need: the centre `width` x `height` of `image`.
fn resize_and_crop(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
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
pub(super) fn binarize(mask: &mut GrayImage) {
    for pixel in mask.pixels_mut() {
        *pixel = Luma([if pixel.0[0] < 128 { 0 } else { 255 }]);
    }
}

/// diffusers' `apply_overlay`: the generated image placed back into the
/// crop `region` of `init_image` and blended into it by `mask`.
pub(super) fn apply_overlay(
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

#[cfg(test)]
mod tests {
    use image::{GrayImage, Luma, Rgb, RgbImage};

    use super::apply_overlay;

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
