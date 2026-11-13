//! The pages rail's thumbnails.

use anyhow::{Result, ensure};
use koharu_renderer::{Frame, Renderer};

/// How tall a thumbnail is, in pixels.
const HEIGHT: u32 = 256;

/// `frame` rasterized, scaled to `HEIGHT` pixels tall, as WebP.
pub fn webp(renderer: &Renderer, frame: &Frame) -> Result<Vec<u8>> {
    let raster = renderer.rasterize(frame)?;
    let (width, height) = raster.image.dimensions();
    ensure!(width > 0 && height > 0, "the page is empty");
    let scaled = (u64::from(width) * u64::from(HEIGHT) / u64::from(height)).max(1);
    let image = image::imageops::thumbnail(&raster.image, u32::try_from(scaled)?, HEIGHT);
    let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
    Ok(encoder.encode(80.0).to_vec())
}
