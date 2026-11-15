//! A layer's pixels as premultiplied pixmaps, at full size and at halved
//! levels, so a view samples the level its scale needs; and the size every
//! surface the renderer draws or decodes is held to.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use anyhow::{Result, bail};
use image::RgbaImage;
use koharu_scene::{Offset, PixelFormat, Tile, Tiles};
use kurbo::{Affine, Rect, Vec2};
use rayon::prelude::*;
use vello_gpu::{PixelMetadata, Pixmap, peniko::ImageAlphaType};

/// The longest side of a surface the renderer draws or decodes.
const MAX_SIDE: u32 = 32_768;

/// The smallest level kept: a view zoomed out further samples it.
const SMALLEST: u32 = 512;

/// Fails for a surface past the renderer's limits: an empty one, one longer
/// than `MAX_SIDE`, or one of more than 2^28 pixels, a gigabyte of RGBA.
pub(crate) fn check_size(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > 1 << 28
    {
        bail!("a {width}×{height} surface is past the renderer's limits");
    }
    Ok(())
}

/// One layer's pixels, placed at `origin` in the layer's own pixel space.
pub(crate) struct Pixels {
    pub origin: Offset,
    pub width: u32,
    pub height: u32,
    /// Level 0 holds the pixels as given; the others are built when a view
    /// first needs them.
    levels: Vec<OnceLock<Arc<Pixmap>>>,
}

impl Pixels {
    /// `image`, straight-alpha RGBA, placed at `origin`.
    pub fn new(origin: Offset, image: RgbaImage) -> Self {
        let (width, height) = image.dimensions();
        let count = (width.max(height).max(1) as f64 / f64::from(SMALLEST))
            .log2()
            .ceil()
            .max(0.0) as usize
            + 1;
        let levels: Vec<_> = (0..count).map(|_| OnceLock::new()).collect();
        let pixmap = Pixmap::from_parts(
            image.into_raw(),
            width as u16,
            height as u16,
            PixelMetadata {
                may_have_transparency: true,
                alpha_type: ImageAlphaType::Alpha,
            },
        );
        let _ = levels[0].set(Arc::new(pixmap));
        Self {
            origin,
            width,
            height,
            levels,
        }
    }

    /// Where the pixels land on the page through `transform`.
    pub fn bounds(&self, transform: Affine) -> Rect {
        let origin = Vec2::from(self.origin).to_point();
        transform.transform_rect_bbox(Rect::from_points(
            origin,
            origin + Vec2::new(f64::from(self.width), f64::from(self.height)),
        ))
    }

    /// What the full-size pixels and their levels take, for the cache.
    pub fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4 * 4 / 3
    }

    /// The level whose scale is the smallest at or above `scale`, output
    /// pixels per layer pixel, with no side past `largest`, and the map from
    /// its pixels to the layer's.
    pub fn level(&self, scale: f64, largest: u32) -> (Arc<Pixmap>, Affine) {
        let fits = (0..self.levels.len())
            .find(|index| self.width.max(self.height).div_ceil(1 << index) <= largest)
            .unwrap_or(self.levels.len() - 1);
        let index = if scale > 0.0 {
            ((1.0 / scale).log2().floor().max(0.0) as usize).min(self.levels.len() - 1)
        } else {
            self.levels.len() - 1
        }
        .max(fits);
        let to_layer = Affine::translate(self.origin) * Affine::scale(f64::from(1u32 << index));
        (self.nth(index), to_layer)
    }

    fn nth(&self, index: usize) -> Arc<Pixmap> {
        self.levels[index]
            .get_or_init(|| Arc::new(half(&self.nth(index - 1))))
            .clone()
    }
}

/// Halves a level, averaging each 2×2 block of premultiplied pixels. An odd
/// side rounds up, and the block's pixels past it count as transparent: a
/// level draws at exactly half the scale of the one before, so its last
/// column or row reaches past the image and covers only half its area.
// fast_image_resize keeps this grid only on a padded copy, which on an odd
// side costs as much as its SIMD saves.
fn half(pixmap: &Pixmap) -> Pixmap {
    let (width, height) = (usize::from(pixmap.width()), usize::from(pixmap.height()));
    let (half_width, half_height) = (width.div_ceil(2).max(1), height.div_ceil(2).max(1));
    let source = pixmap.data_as_u8_slice();
    let stride = width * 4;
    let mut data = vec![0u8; half_width * half_height * 4];
    data.par_chunks_exact_mut(half_width * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let rows = [2 * y, 2 * y + 1].map(|sy| {
                source
                    .get(sy * stride..(sy + 1) * stride)
                    .map(|row| row.as_chunks::<4>().0)
            });
            for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let mut sum = [0u32; 4];
                for source in rows.iter().flatten() {
                    for value in [2 * x, 2 * x + 1].iter().filter_map(|sx| source.get(*sx)) {
                        for (total, channel) in sum.iter_mut().zip(value) {
                            *total += u32::from(*channel);
                        }
                    }
                }
                *pixel = sum.map(|total| (total / 4) as u8);
            }
        });
    Pixmap::from_parts(
        data,
        half_width as u16,
        half_height as u16,
        PixelMetadata {
            may_have_transparency: true,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
        },
    )
}

/// The scene's tiles as one image, cropped to its painted pixels and
/// placed relative to the plane's offset; a grey plane becomes alpha
/// under white. `None` when they paint nothing; tiles spread wider than a
/// page could hold fail.
pub(crate) fn assemble(tiles: Tiles) -> Result<Option<Pixels>> {
    const TILE: i64 = Tile::SIZE as i64;
    let Some(x0) = tiles.tiles.iter().map(|tile| tile.x).min() else {
        return Ok(None);
    };
    let y0 = tiles.tiles.iter().map(|tile| tile.y).min().unwrap_or(0);
    let x1 = tiles.tiles.iter().map(|tile| tile.x).max().unwrap_or(0) + 1;
    let y1 = tiles.tiles.iter().map(|tile| tile.y).max().unwrap_or(0) + 1;
    let extent =
        |from: i32, to: i32| u32::try_from(i64::from(to - from) * TILE).unwrap_or(u32::MAX);
    check_size(extent(x0, x1), extent(y0, y1))?;
    let format = tiles.format;
    let Some([left, top, right, bottom]) = edges(&tiles) else {
        return Ok(None);
    };
    let width = (right - left) as usize;
    let mut image = RgbaImage::new(width as u32, (bottom - top) as u32);
    let mut rows: HashMap<i32, Vec<&Tile>> = HashMap::new();
    for tile in &tiles.tiles {
        rows.entry(tile.y).or_default().push(tile);
    }
    let side = TILE as usize;
    // Row by row across threads, each from the tiles holding it.
    image
        .par_chunks_exact_mut(width * 4)
        .enumerate()
        .for_each(|(row, target)| {
            let y = top + row as i64;
            let line = y.rem_euclid(TILE) as usize;
            let Some(tiles) = i32::try_from(y.div_euclid(TILE))
                .ok()
                .and_then(|at| rows.get(&at))
            else {
                return;
            };
            for tile in tiles {
                let tx = i64::from(tile.x) * TILE;
                let (from_x, to_x) = (tx.max(left), (tx + TILE).min(right));
                if from_x >= to_x {
                    continue;
                }
                let columns = (from_x - tx) as usize..(to_x - tx) as usize;
                let at = (from_x - left) as usize * 4;
                let target = &mut target[at..at + columns.len() * 4];
                match format {
                    PixelFormat::Rgba8 => target.copy_from_slice(
                        &tile.pixels
                            [(line * side + columns.start) * 4..(line * side + columns.end) * 4],
                    ),
                    PixelFormat::Gray8 => {
                        let source =
                            &tile.pixels[line * side + columns.start..line * side + columns.end];
                        for (pixel, grey) in target.as_chunks_mut::<4>().0.iter_mut().zip(source) {
                            *pixel = [255, 255, 255, *grey];
                        }
                    }
                }
            }
        });
    Ok(Some(Pixels::new(
        Offset {
            x: left as i32,
            y: top as i32,
        },
        image,
    )))
}

/// What `tiles` paint, as left, top, right and bottom in their plane's
/// pixels; `None` when they paint nothing. Each edge comes from the tiles
/// nearest it.
fn edges(tiles: &Tiles) -> Option<[i64; 4]> {
    const TILE: i64 = Tile::SIZE as i64;
    let (all, format) = (&tiles.tiles, tiles.format);
    let corner = |tile: &Tile| (i64::from(tile.x) * TILE, i64::from(tile.y) * TILE);
    let edge = |tile: &Tile, at: usize| painted(tile, format).map(|bounds| bounds[at]);
    let left = least(
        all,
        |tile| corner(tile).0,
        |tile| Some(corner(tile).0 + edge(tile, 0)?),
    )?;
    let top = least(
        all,
        |tile| corner(tile).1,
        |tile| Some(corner(tile).1 + edge(tile, 1)?),
    )?;
    // The greatest right and bottom edges, as the least of their negations.
    let right = -least(
        all,
        |tile| -corner(tile).0 - TILE,
        |tile| Some(-corner(tile).0 - edge(tile, 2)?),
    )?;
    let bottom = -least(
        all,
        |tile| -corner(tile).1 - TILE,
        |tile| Some(-corner(tile).1 - edge(tile, 3)?),
    )?;
    Some([left, top, right, bottom])
}

/// The least `edge` of the tiles that paint, where a tile's edge is never
/// below its `floor`: the tiles are visited by floor, stopping once no tile
/// left can be less.
fn least(
    tiles: &[Tile],
    floor: impl Fn(&Tile) -> i64,
    edge: impl Fn(&Tile) -> Option<i64>,
) -> Option<i64> {
    let mut order: Vec<&Tile> = tiles.iter().collect();
    order.sort_unstable_by_key(|tile| floor(tile));
    let mut least: Option<i64> = None;
    for tile in order {
        if least.is_some_and(|least| least <= floor(tile)) {
            break;
        }
        if let Some(edge) = edge(tile) {
            least = Some(least.map_or(edge, |least| least.min(edge)));
        }
    }
    least
}

/// The pixels of `tile` with alpha, as left, top, right and bottom within
/// it; `None` when it has none.
fn painted(tile: &Tile, format: PixelFormat) -> Option<[i64; 4]> {
    let side = Tile::SIZE as usize;
    let mut bounds: Option<[i64; 4]> = None;
    for y in 0..side {
        // The first and last pixels of the row with alpha.
        let ends = match format {
            PixelFormat::Rgba8 => {
                let row = tile.pixels[y * side * 4..(y + 1) * side * 4]
                    .as_chunks::<4>()
                    .0;
                row.iter()
                    .position(|pixel| pixel[3] > 0)
                    .zip(row.iter().rposition(|pixel| pixel[3] > 0))
            }
            PixelFormat::Gray8 => {
                let row = &tile.pixels[y * side..(y + 1) * side];
                row.iter()
                    .position(|alpha| *alpha > 0)
                    .zip(row.iter().rposition(|alpha| *alpha > 0))
            }
        };
        let Some((first, last)) = ends else {
            continue;
        };
        let (x0, x1, y) = (first as i64, last as i64 + 1, y as i64);
        bounds = Some(bounds.map_or([x0, y, x1, y + 1], |[left, top, right, _]| {
            [left.min(x0), top, right.max(x1), y + 1]
        }));
    }
    bounds
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_halved_odd_side_covers_only_the_image() {
        let image = RgbaImage::from_pixel(3, 2, image::Rgba([200, 100, 50, 255]));
        let half = half(&Pixels::new(Offset::default(), image).nth(0));
        assert_eq!((half.width(), half.height()), (2, 1));
        let pixels = half.data_as_u8_slice();
        assert_eq!(pixels[..4], [200, 100, 50, 255]);
        // Premultiplied: half the area, so half of each channel.
        assert_eq!(pixels[4..], [100, 50, 25, 127]);
    }
}
