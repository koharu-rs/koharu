//! Decoding imported files, with their sizes checked before any pixel is.

use std::io::Cursor;

use image::{ImageError, ImageReader, RgbaImage};
use koharu_scene::Hash;

use crate::{Error, Result};

/// The largest surface the renderer draws or decodes.
const MAX_SIDE: u32 = 32_768;
const MAX_PIXELS: u64 = 268_435_456;

pub(crate) fn check_size(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(Error::invalid(format!(
            "a {width}×{height} surface is past the renderer's limits"
        )));
    }
    Ok(())
}

pub(crate) fn decode(file: &Hash, bytes: &[u8]) -> Result<RgbaImage> {
    let image = |source: ImageError| Error::Image {
        file: file.clone(),
        source,
    };
    let reader = || {
        ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| image(ImageError::IoError(error)))
    };
    let (width, height) = reader()?.into_dimensions().map_err(image)?;
    check_size(width, height)?;
    Ok(reader()?.decode().map_err(image)?.into_rgba8())
}
