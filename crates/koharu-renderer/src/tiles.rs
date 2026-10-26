//! Pixels as 512×512 tiles at full size and at halved levels, so a pass draws
//! only the tiles it meets at the level its scale needs, and vello's image
//! atlas holds them.

use std::sync::{Arc, OnceLock};

use image::RgbaImage;
use vello::{
    Scene,
    kurbo::{Affine, Rect},
    peniko::{
        Blob, Extend, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality,
    },
};

const TILE: u32 = 512;
/// Each tile repeats one pixel of its neighbours, so filtered sampling at its
/// edge reads real pixels and leaves no seam.
const GUTTER: u32 = 1;

/// One layer's pixels in straight-alpha RGBA, placed at `origin` in the
/// layer's own pixel space.
pub(crate) struct Tiled {
    pub origin: (i32, i32),
    pub width: u32,
    pub height: u32,
    levels: Vec<OnceLock<Arc<Level>>>,
    full: RgbaImage,
}

struct Level {
    image: RgbaImage,
    /// Tiles in rows, each with its rectangle in this level's pixels.
    tiles: Vec<(Rect, ImageData)>,
}

impl Tiled {
    pub fn new(origin: (i32, i32), image: RgbaImage) -> Self {
        let (width, height) = image.dimensions();
        let count = (width.max(height).max(1) as f64 / f64::from(TILE))
            .log2()
            .ceil()
            .max(0.0) as usize
            + 1;
        Self {
            origin,
            width,
            height,
            levels: (0..count).map(|_| OnceLock::new()).collect(),
            full: image,
        }
    }

    fn level(&self, index: usize) -> Arc<Level> {
        self.levels[index]
            .get_or_init(|| {
                let image = match index {
                    0 => self.full.clone(),
                    _ => half(&self.level(index - 1).image),
                };
                Arc::new(Level {
                    tiles: cut(&image),
                    image,
                })
            })
            .clone()
    }

    /// Draws the tiles that meet `visible`, a rectangle of output pixels,
    /// through `transform` from layer pixels to output pixels. Returns the
    /// image pixels drawn.
    pub fn draw(&self, scene: &mut Scene, transform: Affine, visible: Rect) -> u64 {
        let (index, tiles, to_layer) = self.select(transform, visible);
        let level = self.level(index);
        let mut pixels = 0;
        for at in tiles {
            let (rect, image) = &level.tiles[at];
            let brush = ImageBrush::new(image.clone())
                .with_extend(Extend::Pad)
                .with_quality(ImageQuality::Medium);
            let brush_transform =
                Affine::translate((rect.x0 - f64::from(GUTTER), rect.y0 - f64::from(GUTTER)));
            scene.fill(
                Fill::NonZero,
                transform * to_layer,
                &brush,
                Some(brush_transform),
                rect,
            );
            pixels += u64::from(image.width) * u64::from(image.height);
        }
        pixels
    }

    /// The image pixels `draw` would draw.
    pub fn cost(&self, transform: Affine, visible: Rect) -> u64 {
        let (_, tiles, _) = self.select(transform, visible);
        let tile = u64::from(TILE + 2 * GUTTER);
        tiles.len() as u64 * tile * tile
    }

    /// The level whose scale is the smallest at or above the output's, the
    /// tiles of it that `visible` meets, and the map from that level's pixels
    /// to the layer's.
    fn select(&self, transform: Affine, visible: Rect) -> (usize, Vec<usize>, Affine) {
        let output = transform.determinant().abs().sqrt();
        let index = if output > 0.0 {
            ((1.0 / output).log2().floor().max(0.0) as usize).min(self.levels.len() - 1)
        } else {
            self.levels.len() - 1
        };
        let factor = f64::from(1u32 << index);
        let to_layer = Affine::translate((f64::from(self.origin.0), f64::from(self.origin.1)))
            * Affine::scale(factor);
        let inverse = (transform * to_layer).inverse();
        let area = inverse.transform_rect_bbox(visible);
        let (width, height) = (
            self.width.div_ceil(1 << index),
            self.height.div_ceil(1 << index),
        );
        let columns = width.div_ceil(TILE);
        let rows = height.div_ceil(TILE);
        let tile = f64::from(TILE);
        let x0 = (area.x0 / tile).floor().max(0.0) as u32;
        let y0 = (area.y0 / tile).floor().max(0.0) as u32;
        let x1 = ((area.x1 / tile).ceil().max(0.0) as u32).min(columns);
        let y1 = ((area.y1 / tile).ceil().max(0.0) as u32).min(rows);
        let tiles = (y0..y1)
            .flat_map(|row| (x0..x1).map(move |column| (row * columns + column) as usize))
            .collect();
        (index, tiles, to_layer)
    }
}

/// Halves an image, averaging each 2×2 block in premultiplied alpha.
fn half(image: &RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    let (half_width, half_height) = (width.div_ceil(2).max(1), height.div_ceil(2).max(1));
    RgbaImage::from_fn(half_width, half_height, |x, y| {
        let mut sum = [0u32; 4];
        let mut count = 0;
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (sx, sy) = (x * 2 + dx, y * 2 + dy);
            if sx < width && sy < height {
                let pixel = image.get_pixel(sx, sy).0;
                let alpha = u32::from(pixel[3]);
                for channel in 0..3 {
                    sum[channel] += u32::from(pixel[channel]) * alpha;
                }
                sum[3] += alpha;
                count += 1;
            }
        }
        let alpha = sum[3] / count;
        let color = |channel: usize| match sum[3] {
            0 => 0,
            total => (sum[channel] / total) as u8,
        };
        image::Rgba([color(0), color(1), color(2), alpha as u8])
    })
}

/// Cuts an image into tiles in rows, each with a gutter from its neighbours.
fn cut(image: &RgbaImage) -> Vec<(Rect, ImageData)> {
    let (width, height) = image.dimensions();
    let mut tiles = Vec::new();
    for row in 0..height.div_ceil(TILE) {
        for column in 0..width.div_ceil(TILE) {
            let (x, y) = (column * TILE, row * TILE);
            let (w, h) = (TILE.min(width - x), TILE.min(height - y));
            let size = TILE + 2 * GUTTER;
            let mut tile = vec![0u8; (size * size * 4) as usize];
            // A gutter pixel repeats its nearest image pixel.
            let (left, right) = (x.saturating_sub(GUTTER), (x + w + GUTTER).min(width));
            let skip = GUTTER - (x - left);
            for ty in 0..h + 2 * GUTTER {
                let sy = (y + ty).saturating_sub(GUTTER).min(height - 1);
                let source = &image.as_raw()
                    [((sy * width + left) * 4) as usize..((sy * width + right) * 4) as usize];
                let row = &mut tile[(ty * size * 4) as usize..((ty + 1) * size * 4) as usize];
                let at = (skip * 4) as usize;
                row[at..at + source.len()].copy_from_slice(source);
                if skip > 0 {
                    let first: [u8; 4] = source[..4].try_into().expect("a pixel");
                    row[..4].copy_from_slice(&first);
                }
                let end = at + source.len();
                if end < ((w + 2 * GUTTER) * 4) as usize {
                    let last: [u8; 4] = source[source.len() - 4..].try_into().expect("a pixel");
                    row[end..end + 4].copy_from_slice(&last);
                }
            }
            let data = ImageData {
                data: Blob::from(tile),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: size,
                height: size,
            };
            tiles.push((
                Rect::new(
                    f64::from(x),
                    f64::from(y),
                    f64::from(x + w),
                    f64::from(y + h),
                ),
                data,
            ));
        }
    }
    tiles
}
