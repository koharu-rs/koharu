//! Rust ports of the Leptonica algorithms no Rust crate covers, from
//! Leptonica 1.87.0 (commit 13275a2). Each keeps Leptonica's integer and
//! float arithmetic, so it gives Leptonica's result on the same input.
//!
//! A binary input is a `GrayImage` whose nonzero pixels are foreground; a
//! binary result marks foreground 255.

mod binarize;
mod colorseg;
mod skew;
mod strokes;

use image::{GrayImage, Luma};

pub use binarize::{otsu_adaptive_threshold, sauvola_binarize};
pub use colorseg::{Segments, color_segment};
pub use skew::{Skew, find_skew, find_skew_sweep_and_search};
pub use strokes::find_stroke_width;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Leptonica's 1 bpp image, a byte a pixel: 1 is foreground.
#[derive(Clone)]
struct Binary {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Binary {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height],
        }
    }

    fn from_image(image: &GrayImage) -> Self {
        Self {
            width: image.width() as usize,
            height: image.height() as usize,
            pixels: image
                .as_raw()
                .iter()
                .map(|&value| u8::from(value != 0))
                .collect(),
        }
    }

    fn to_image(&self) -> GrayImage {
        GrayImage::from_fn(self.width as u32, self.height as u32, |x, y| {
            Luma([self.get(x as usize, y as usize) * u8::MAX])
        })
    }

    fn get(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * self.width + x]
    }

    fn count(&self) -> u32 {
        self.pixels.iter().map(|&pixel| u32::from(pixel)).sum()
    }
}
