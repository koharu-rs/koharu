//! Pixels as 64×64 tiles keyed by tile position, each compressed with zlib
//! on its own, as GIMP and Krita store them. Only tiles holding pixels are
//! stored.

use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use loro::LoroBinaryValue;

use crate::{Error, Hash, PixelFormat, Rect, Result, Tile};

const SIZE: i32 = 64;

/// Compressed tiles by tile position. The bytes are shared, so copying a
/// map copies no pixels.
pub(crate) type TileMap = BTreeMap<(i32, i32), LoroBinaryValue>;

/// Tiles that changed: `None` for one that became empty.
pub(crate) type Changes = BTreeMap<(i32, i32), Option<LoroBinaryValue>>;

pub(crate) fn bytes_per_pixel(format: PixelFormat) -> usize {
    match format {
        PixelFormat::Rgba8 => 4,
        PixelFormat::Gray8 => 1,
    }
}

pub(crate) fn key((x, y): (i32, i32)) -> String {
    format!("{x},{y}")
}

pub(crate) fn parse_key(key: &str) -> Option<(i32, i32)> {
    let (x, y) = key.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn tile_len(format: PixelFormat) -> usize {
    (SIZE * SIZE) as usize * bytes_per_pixel(format)
}

fn decompress(bytes: &[u8], format: PixelFormat) -> Result<Vec<u8>> {
    let mut raw = Vec::with_capacity(tile_len(format));
    ZlibDecoder::new(bytes)
        .read_to_end(&mut raw)
        .map_err(Error::corrupt)?;
    if raw.len() != tile_len(format) {
        return Err(Error::corrupt("a pixel tile has the wrong size"));
    }
    Ok(raw)
}

/// Writes `pixels` over `rect`, replacing what was there, and returns the
/// tiles that changed.
pub(crate) fn write(
    tiles: &mut TileMap,
    format: PixelFormat,
    rect: Rect,
    pixels: &[u8],
) -> Result<Changes> {
    let bpp = bytes_per_pixel(format);
    let size = i64::from(SIZE);
    let (left, top) = (i64::from(rect.x), i64::from(rect.y));
    let (right, bottom) = (left + i64::from(rect.width), top + i64::from(rect.height));
    let mut changed = Changes::new();
    if rect.width == 0 || rect.height == 0 {
        return Ok(changed);
    }
    for ty in top.div_euclid(size)..=(bottom - 1).div_euclid(size) {
        for tx in left.div_euclid(size)..=(right - 1).div_euclid(size) {
            let position = (tx as i32, ty as i32);
            let mut raw = match tiles.get(&position) {
                Some(bytes) => decompress(bytes, format)?,
                None => vec![0; tile_len(format)],
            };
            let (origin_x, origin_y) = (tx * size, ty * size);
            let (from_x, to_x) = (left.max(origin_x), right.min(origin_x + size));
            let width = (to_x - from_x) as usize * bpp;
            for y in top.max(origin_y)..bottom.min(origin_y + size) {
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
                if tiles.remove(&position).is_some() {
                    changed.insert(position, None);
                }
            } else {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                encoder
                    .write_all(&raw)
                    .expect("writing to a Vec cannot fail");
                let bytes =
                    LoroBinaryValue::from(encoder.finish().expect("writing to a Vec cannot fail"));
                tiles.insert(position, bytes.clone());
                changed.insert(position, Some(bytes));
            }
        }
    }
    Ok(changed)
}

/// The tiles overlapping `rect` (all of them without one), decoded.
pub(crate) fn decode(
    tiles: &TileMap,
    format: PixelFormat,
    rect: Option<Rect>,
) -> Result<Vec<Tile>> {
    tiles
        .iter()
        .filter(|&(&(x, y), _)| {
            rect.is_none_or(|rect| {
                let (left, top) = (x * SIZE, y * SIZE);
                left < rect.x.saturating_add_unsigned(rect.width)
                    && rect.x < left + SIZE
                    && top < rect.y.saturating_add_unsigned(rect.height)
                    && rect.y < top + SIZE
            })
        })
        .map(|(&(x, y), bytes)| {
            Ok(Tile {
                x,
                y,
                pixels: decompress(bytes, format)?,
            })
        })
        .collect()
}

/// The hash of a layer's pixels, over decoded tiles so that it does not
/// depend on how zlib compressed them.
pub(crate) fn hash(tiles: &TileMap, format: PixelFormat) -> Result<Hash> {
    let mut hasher = blake3::Hasher::new();
    for (&(x, y), bytes) in tiles {
        hasher.update(&x.to_le_bytes());
        hasher.update(&y.to_le_bytes());
        hasher.update(&decompress(bytes, format)?);
    }
    Ok(hasher.finalize().into())
}
