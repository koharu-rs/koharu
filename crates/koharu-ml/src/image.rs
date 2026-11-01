//! The resize every processor goes through, each naming the filter its
//! upstream resizes with, and reflect padding's mirrored index.

use anyhow::{Context, Result};
use fast_image_resize::{IntoImageView, IntoImageViewMut, ResizeAlg, ResizeOptions, Resizer};
use image::{ImageBuffer, Pixel};

/// `image` resized to `width` by `height` with `algorithm`; one already that
/// size is copied as it is.
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
    let mut output = ImageBuffer::new(width, height);
    Resizer::new()
        .resize(
            image,
            &mut output,
            &ResizeOptions::new().resize_alg(algorithm).use_alpha(false),
        )
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
