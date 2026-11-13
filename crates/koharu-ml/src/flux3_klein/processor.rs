//! FLUX.3 Image Klein image preprocessing.
//!
//! References follow diffusers' `Flux3ImageProcessor`, which follows Black
//! Forest Labs' reference implementation:
//! https://github.com/huggingface/diffusers/blob/86ef420b17363c28e36ebad6b58555e9aa8feda2/src/diffusers/modular_pipelines/flux3/image_processor.py
//! https://github.com/black-forest-labs/flux-3-image-klein/blob/62e90b9278b73ecc52927770c6965dc6b1966498/src/flux3/pipeline.py#L129-L171
//! Neither inpaints, so inpainting keeps diffusers' FLUX.2 Klein helpers, in
//! `crate::diffusers`.

use anyhow::{Result, ensure};
use image::{DynamicImage, RgbImage};

use crate::diffusers::{LANCZOS, Region, TOKEN, resize_and_crop, resize_to_target_area};

const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 4096;
/// The area FLUX.2 Klein's pipeline fits several references under.
const REFERENCE_AREA: u32 = 1024 * 1024;

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
