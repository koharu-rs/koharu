//! Test images built with integer arithmetic only, so they're the same on
//! every platform and Leptonica's outputs for them stay valid.

use image::{GrayImage, Luma, Rgb, RgbImage};

/// A linear congruential sequence.
pub struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn below(&mut self, range: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(range)) as u32
    }
}

/// Lines of glyph-like blocks, ink 255, each line falling `rise` pixels per
/// 1000 across.
pub fn text(width: u32, height: u32, rise: i64, seed: u64) -> GrayImage {
    let mut random = Random::new(seed);
    let mut image = GrayImage::new(width, height);
    let mut top = 12;
    while top + 20 < height {
        let mut left = 10 + random.below(20);
        while left + 20 < width {
            let (glyph_width, glyph_height) = (4 + random.below(14), 8 + random.below(8));
            for x in left..left + glyph_width {
                let fall = i64::from(x) * rise / 1000;
                for y in top + 16 - glyph_height..top + 16 {
                    let y = i64::from(y) + fall;
                    if (0..i64::from(height)).contains(&y) {
                        image.put_pixel(x, y as u32, Luma([255]));
                    }
                }
            }
            left += glyph_width + 2 + random.below(10);
        }
        top += 22 + random.below(6);
    }
    image
}

/// A grey page: text in dark ink over a light background that darkens down
/// and to the left, with noise.
pub fn page(width: u32, height: u32, seed: u64) -> GrayImage {
    let ink = text(width, height, 15, seed);
    let mut random = Random::new(seed ^ 0x5eed);
    GrayImage::from_fn(width, height, |x, y| {
        let noise = random.below(17) as i32 - 8;
        let value = if ink.get_pixel(x, y).0[0] != 0 {
            30 + noise
        } else {
            190 + (30 * x / width) as i32 - (40 * y / height) as i32 + noise
        };
        Luma([value.clamp(0, 255) as u8])
    })
}

/// Ink 255 in a grid of `width`-pixel strokes: rows, columns, and diagonals.
pub fn strokes(image_width: u32, image_height: u32, width: u32) -> GrayImage {
    GrayImage::from_fn(image_width, image_height, |x, y| {
        let on = x % 20 < width || y % 17 < width || (x + y) % 23 < width;
        Luma([if on { 255 } else { 0 }])
    })
}

/// Noisy blocks of six colours, from near black to near white, over a
/// gradient strip.
pub fn colours(width: u32, height: u32, seed: u64) -> RgbImage {
    const BLOCKS: [[i32; 3]; 6] = [
        [240, 238, 230],
        [20, 18, 25],
        [200, 40, 50],
        [40, 90, 200],
        [230, 200, 60],
        [120, 120, 120],
    ];
    let mut random = Random::new(seed);
    RgbImage::from_fn(width, height, |x, y| {
        let noise = |random: &mut Random| random.below(25) as i32 - 12;
        let base = if y >= height - height / 5 {
            [(255 * x / width) as i32, (200 * y / height) as i32, 128]
        } else {
            BLOCKS[((x * 3 / width) + 3 * (y * 2 / height)) as usize % BLOCKS.len()]
        };
        Rgb(base.map(|channel| (channel + noise(&mut random)).clamp(0, 255) as u8))
    })
}

/// FNV-1a, to compare whole images against a recorded value.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
