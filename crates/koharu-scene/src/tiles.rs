//! Pixels as 64×64 tiles keyed by tile position, each compressed with zlib on
//! its own, as GIMP stores them. Only tiles holding pixels are stored.

use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use rayon::prelude::*;

use crate::{Error, Offset, PixelFormat, Rect, Result};

pub(crate) const SIZE: i64 = Tile::SIZE as i64;
/// zlib level 3 through `zlib-rs` encoded Manga109 tiles three times faster
/// than level 6 through `miniz_oxide`, for 4% more bytes.
const LEVEL: u32 = 3;

/// Decoded tiles at tile coordinates, on a grid whose origin sits at `offset`
/// on the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Tiles {
    pub offset: Offset,
    pub format: PixelFormat,
    pub tiles: Vec<Tile>,
}

/// A tile at tile coordinates, its pixels in rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub pixels: Vec<u8>,
}

impl Tile {
    /// The pixels on a tile's side.
    pub const SIZE: u32 = 64;
}

impl Tiles {
    /// What 8-bit tiles cover of a `width` by `height` page: the rectangle
    /// around their set pixels, and those pixels over it; `None` when they
    /// cover none of it.
    #[must_use]
    pub fn coverage(&self, width: u32, height: u32) -> Option<(Rect, image::GrayImage)> {
        let (width, height) = (i64::from(width), i64::from(height));
        let mut on = Vec::new();
        for tile in &self.tiles {
            let left = i64::from(tile.x) * SIZE + i64::from(self.offset.x);
            let top = i64::from(tile.y) * SIZE + i64::from(self.offset.y);
            for (at, &value) in tile.pixels.iter().enumerate() {
                let (x, y) = (left + at as i64 % SIZE, top + at as i64 / SIZE);
                if value != 0 && (0..width).contains(&x) && (0..height).contains(&y) {
                    on.push((x, y));
                }
            }
        }
        let x = on.iter().map(|&(x, _)| x).min()?;
        let y = on.iter().map(|&(_, y)| y).min()?;
        let right = on.iter().map(|&(x, _)| x).max()? + 1;
        let bottom = on.iter().map(|&(_, y)| y).max()? + 1;
        let rect = Rect {
            x: x as i32,
            y: y as i32,
            width: (right.min(width) - x) as u32,
            height: (bottom.min(height) - y) as u32,
        };
        let mut mask = image::GrayImage::new(rect.width, rect.height);
        for (px, py) in on {
            mask.put_pixel((px - x) as u32, (py - y) as u32, image::Luma([255]));
        }
        Some((rect, mask))
    }
}

/// A stored tile: its position and compressed bytes.
pub(crate) type Stored = (i32, i32, Vec<u8>);
/// A written tile: its position, and its compressed bytes unless it's now empty.
pub(crate) type Written = (i32, i32, Option<Vec<u8>>);

pub(crate) fn bytes_per_pixel(format: PixelFormat) -> usize {
    match format {
        PixelFormat::Rgba8 => 4,
        PixelFormat::Gray8 => 1,
    }
}

fn tile_len(format: PixelFormat) -> usize {
    (SIZE * SIZE) as usize * bytes_per_pixel(format)
}

/// The tile columns and rows `rect` touches, or `None` when it's empty.
pub(crate) fn covered(rect: Rect) -> Option<((i64, i64), (i64, i64))> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let (left, top) = (i64::from(rect.x), i64::from(rect.y));
    let (right, bottom) = (left + i64::from(rect.width), top + i64::from(rect.height));
    Some((
        (left.div_euclid(SIZE), (right - 1).div_euclid(SIZE)),
        (top.div_euclid(SIZE), (bottom - 1).div_euclid(SIZE)),
    ))
}

/// Inflates a tile, reading no more than a tile can hold, so a crafted one
/// can't exhaust memory before it is found wrong.
fn decompress(bytes: &[u8], format: PixelFormat) -> Result<Vec<u8>> {
    let mut raw = Vec::with_capacity(tile_len(format));
    ZlibDecoder::new(bytes)
        .take(tile_len(format) as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(Error::corrupt)?;
    if raw.len() != tile_len(format) {
        return Err(Error::corrupt("a pixel tile has the wrong size"));
    }
    Ok(raw)
}

/// Writes `pixels` over `rect` on the `current` compressed tiles, and returns
/// each tile `rect` touches with its new bytes, or `None` for one now empty.
/// Each tile is decoded, written, and encoded on its own, so tiles run in
/// parallel.
pub(crate) fn write(
    current: &[Stored],
    format: PixelFormat,
    rect: Rect,
    pixels: &[u8],
) -> Result<Vec<Written>> {
    let Some(((first_x, last_x), (first_y, last_y))) = covered(rect) else {
        return Ok(Vec::new());
    };
    let bpp = bytes_per_pixel(format);
    let (left, top) = (i64::from(rect.x), i64::from(rect.y));
    let (right, bottom) = (left + i64::from(rect.width), top + i64::from(rect.height));
    let current: BTreeMap<(i32, i32), &[u8]> = current
        .iter()
        .map(|(x, y, bytes)| ((*x, *y), bytes.as_slice()))
        .collect();
    let positions: Vec<(i64, i64)> = (first_y..=last_y)
        .flat_map(|y| (first_x..=last_x).map(move |x| (x, y)))
        .collect();
    positions
        .into_par_iter()
        .map(|(tx, ty)| {
            let position = (tx as i32, ty as i32);
            let mut raw = match current.get(&position) {
                Some(bytes) => decompress(bytes, format)?,
                None => vec![0; tile_len(format)],
            };
            let (origin_x, origin_y) = (tx * SIZE, ty * SIZE);
            let (from_x, to_x) = (left.max(origin_x), right.min(origin_x + SIZE));
            let width = (to_x - from_x) as usize * bpp;
            for y in top.max(origin_y)..bottom.min(origin_y + SIZE) {
                let source =
                    ((y - top) as usize * rect.width as usize + (from_x - left) as usize) * bpp;
                let target =
                    ((y - origin_y) as usize * SIZE as usize + (from_x - origin_x) as usize) * bpp;
                raw[target..target + width].copy_from_slice(&pixels[source..source + width]);
            }
            let empty = match format {
                PixelFormat::Rgba8 => raw.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0),
                PixelFormat::Gray8 => raw.iter().all(|&value| value == 0),
            };
            if empty {
                return Ok((position.0, position.1, None));
            }
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(LEVEL));
            encoder
                .write_all(&raw)
                .expect("writing to a Vec cannot fail");
            let bytes = encoder.finish().expect("writing to a Vec cannot fail");
            Ok((position.0, position.1, Some(bytes)))
        })
        .collect()
}

/// Decodes compressed tiles in parallel.
pub(crate) fn decode(tiles: &[Stored], format: PixelFormat) -> Result<Vec<Tile>> {
    tiles
        .par_iter()
        .map(|(x, y, bytes)| {
            Ok(Tile {
                x: *x,
                y: *y,
                pixels: decompress(bytes, format)?,
            })
        })
        .collect()
}
