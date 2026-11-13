//! Pixels as 512×512 tiles at full size and at halved levels, so a pass draws
//! only the tiles it meets at the level its scale needs, and vello's image
//! atlas holds them.

use std::sync::{Arc, OnceLock};

use std::collections::HashMap;

use image::RgbaImage;
use koharu_scene::{PixelFormat, Tile, Tiles};
use rayon::prelude::*;

use crate::{Result, images};
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
    /// Level 0 holds the pixels as given; the others are built when a view
    /// first needs them.
    levels: Vec<OnceLock<Arc<Level>>>,
}

struct Level {
    image: RgbaImage,
    /// Tiles in rows, each with its rectangle in this level's pixels.
    tiles: OnceLock<Vec<(Rect, ImageData)>>,
}

impl Level {
    fn new(image: RgbaImage) -> Arc<Self> {
        Arc::new(Self {
            image,
            tiles: OnceLock::new(),
        })
    }

    fn tiles(&self) -> &[(Rect, ImageData)] {
        self.tiles.get_or_init(|| cut(&self.image))
    }
}

impl Tiled {
    pub fn new(origin: (i32, i32), image: RgbaImage) -> Self {
        let (width, height) = image.dimensions();
        let count = (width.max(height).max(1) as f64 / f64::from(TILE))
            .log2()
            .ceil()
            .max(0.0) as usize
            + 1;
        let levels: Vec<_> = (0..count).map(|_| OnceLock::new()).collect();
        let _ = levels[0].set(Level::new(image));
        Self {
            origin,
            width,
            height,
            levels,
        }
    }

    /// Where the pixels land on the page through `transform`.
    pub fn bounds(&self, transform: Affine) -> Rect {
        transform.transform_rect_bbox(Rect::new(
            f64::from(self.origin.0),
            f64::from(self.origin.1),
            f64::from(self.origin.0) + f64::from(self.width),
            f64::from(self.origin.1) + f64::from(self.height),
        ))
    }

    /// What the full-size pixels and their tiles take, for the cache.
    pub fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4 * 2
    }

    fn level(&self, index: usize) -> Arc<Level> {
        self.levels[index]
            .get_or_init(|| Level::new(half(&self.level(index - 1).image)))
            .clone()
    }

    /// Draws the tiles that meet `visible`, a rectangle of output pixels,
    /// through `transform` from layer pixels to output pixels.
    pub fn draw(&self, scene: &mut Scene, transform: Affine, visible: Rect) {
        let (index, tiles, to_layer) = self.select(transform, visible);
        let level = self.level(index);
        let to_output = transform * to_layer;
        let size = (
            f64::from(level.image.width()),
            f64::from(level.image.height()),
        );
        for at in tiles {
            let (rect, image) = &level.tiles()[at];
            let brush = ImageBrush::new(image.clone())
                .with_extend(Extend::Pad)
                .with_quality(ImageQuality::Medium);
            let brush_transform =
                Affine::translate((rect.x0 - f64::from(GUTTER), rect.y0 - f64::from(GUTTER)));
            scene.fill(
                Fill::NonZero,
                to_output,
                &brush,
                Some(brush_transform),
                &snapped(*rect, to_output, size),
            );
        }
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

/// `rect`, a tile in level pixels, with each edge it shares with another
/// tile moved to the nearest output pixel through an axis-aligned
/// `to_output`. Two tiles meeting inside a pixel would each cover part of it,
/// and compositing one over the other lets what's under show through the
/// line, so they meet on a pixel's edge instead. The move is at most half an
/// output pixel, under a level pixel, which the gutter covers; the image's
/// own edges stay where they are. A turned tile keeps its edges.
fn snapped(rect: Rect, to_output: Affine, (width, height): (f64, f64)) -> Rect {
    let [a, b, c, d, e, f] = to_output.as_coeffs();
    if b != 0.0 || c != 0.0 || a == 0.0 || d == 0.0 {
        return rect;
    }
    let snap = |at: f64, scale: f64, shift: f64, end: f64| match at > 0.0 && at < end {
        true => ((at * scale + shift).round() - shift) / scale,
        false => at,
    };
    Rect::new(
        snap(rect.x0, a, e, width),
        snap(rect.y0, d, f, height),
        snap(rect.x1, a, e, width),
        snap(rect.y1, d, f, height),
    )
}

/// Halves an image, averaging each 2×2 block in premultiplied alpha. An odd
/// side rounds up, and the block's pixels past it count as transparent: a
/// level draws at exactly half the scale of the one before, so its last
/// column or row reaches past the image and covers only half its area.
// fast_image_resize keeps this grid only on a padded copy, which on an odd
// side costs as much as its SIMD saves.
fn half(image: &RgbaImage) -> RgbaImage {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let (half_width, half_height) = (width.div_ceil(2).max(1), height.div_ceil(2).max(1));
    let source = image.as_raw();
    let stride = width * 4;
    let mut half = RgbaImage::new(half_width as u32, half_height as u32);
    half.par_chunks_exact_mut(half_width * 4)
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
                    for &[red, green, blue, alpha] in
                        [2 * x, 2 * x + 1].iter().filter_map(|sx| source.get(*sx))
                    {
                        let alpha = u32::from(alpha);
                        sum[0] += u32::from(red) * alpha;
                        sum[1] += u32::from(green) * alpha;
                        sum[2] += u32::from(blue) * alpha;
                        sum[3] += alpha;
                    }
                }
                let color = |channel: usize| match sum[3] {
                    0 => 0,
                    total => (sum[channel] / total) as u8,
                };
                *pixel = [color(0), color(1), color(2), (sum[3] / 4) as u8];
            }
        });
    half
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

/// The scene's tiles as one image, cropped to its painted pixels and placed
/// relative to the plane's offset; a grey plane becomes alpha under white.
/// Tiles spread wider than a page could hold fail.
pub(crate) fn assemble(tiles: Tiles) -> Result<Option<Tiled>> {
    const TILE: i64 = Tile::SIZE as i64;
    let Some(x0) = tiles.tiles.iter().map(|tile| tile.x).min() else {
        return Ok(None);
    };
    let y0 = tiles.tiles.iter().map(|tile| tile.y).min().unwrap_or(0);
    let x1 = tiles.tiles.iter().map(|tile| tile.x).max().unwrap_or(0) + 1;
    let y1 = tiles.tiles.iter().map(|tile| tile.y).max().unwrap_or(0) + 1;
    let extent =
        |from: i32, to: i32| u32::try_from(i64::from(to - from) * TILE).unwrap_or(u32::MAX);
    images::check_size(extent(x0, x1), extent(y0, y1))?;
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
    Ok(Some(Tiled::new((left as i32, top as i32), image)))
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
        let half = half(&image);
        assert_eq!(half.dimensions(), (2, 1));
        assert_eq!(half.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert_eq!(half.get_pixel(1, 0).0, [200, 100, 50, 127]);
    }
}
