//! The resize most processors go through, each naming the filter its
//! upstream resizes with, and reflect padding's mirrored index.

use anyhow::{Context, Result};
use fast_image_resize::{IntoImageView, IntoImageViewMut, ResizeAlg, ResizeOptions, Resizer};
use image::{ImageBuffer, Pixel};

/// `image` resized to `width` by `height` with `algorithm`; one already that
/// size is copied as it is.
///
/// Pillow and torchvision go horizontally first and clip to `u8` between their
/// passes. Asked for both at once, `fast_image_resize` goes vertically first on
/// `u8`, which differs by tens of levels where a filter overshoots at a text
/// edge, so each pass is asked for on its own.
pub(crate) fn resize<P>(
    image: &ImageBuffer<P, Vec<u8>>,
    width: u32,
    height: u32,
    algorithm: ResizeAlg,
) -> Result<ImageBuffer<P, Vec<u8>>>
where
    P: Pixel<Subpixel = u8>,
    ImageBuffer<P, Vec<u8>>: IntoImageView + IntoImageViewMut,
{
    let options = ResizeOptions::new().resize_alg(algorithm).use_alpha(false);
    let mut resizer = Resizer::new();
    let mut horizontal = ImageBuffer::new(width, image.height());
    resizer
        .resize(image, &mut horizontal, &options)
        .context("failed to resize an image")?;
    let mut output = ImageBuffer::new(width, height);
    resizer
        .resize(&horizontal, &mut output, &options)
        .context("failed to resize an image")?;
    Ok(output)
}

/// The index `index` reads from `length` values padded symmetrically, numpy's
/// `symmetric` mode: mirrored at each edge with the edge repeated.
pub(crate) fn symmetric_index(index: u32, length: u32) -> u32 {
    let index = index % (length * 2);
    if index < length {
        index
    } else {
        length * 2 - index - 1
    }
}
