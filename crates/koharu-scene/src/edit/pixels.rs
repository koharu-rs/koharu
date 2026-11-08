//! Pixels and imported files, whose bytes live in the store by content hash,
//! out of undo's reach, so undo steps stay small.

use std::io::Cursor;

use super::Run;
use crate::{Error, Hash, LayerId, Rect, Ref, Result, Size, tiles};

impl Run<'_, '_> {
    pub(super) fn write_pixels(
        &mut self,
        op: usize,
        layer: &Ref,
        rect: Rect,
        pixels: &[u8],
    ) -> Result<()> {
        let id = LayerId(self.resolve(layer)?);
        let Some(picture) = self.project.picture(id) else {
            return Err(match self.project.index.contains_key(&id) {
                true => Error::invalid(op, format!("layer {id} has no pixels")),
                false => Error::NotFound(format!("layer {id}")),
            });
        };
        let format = picture.format;
        let expected = rect.width as usize * rect.height as usize * tiles::bytes_per_pixel(format);
        if pixels.len() != expected {
            return Err(Error::invalid(
                op,
                format!(
                    "{} bytes of pixels, but the rectangle needs {expected}",
                    pixels.len()
                ),
            ));
        }
        let end = |start: i32, length: u32| i64::from(start) + i64::from(length);
        if end(rect.x, rect.width) > 1 << 31 || end(rect.y, rect.height) > 1 << 31 {
            return Err(Error::invalid(op, "the rectangle runs out of range"));
        }
        if picture.locked {
            return Err(Error::Locked { op, layer: id });
        }
        self.unlocked(op, picture.holder)?;
        let Some(((x0, x1), (y0, y1))) = tiles::covered(rect) else {
            return Ok(());
        };
        let mut map = self.project.tiles.get(&id).cloned().unwrap_or_default();
        let mut current = Vec::new();
        for ((x, y), hash) in map.iter() {
            if (x0..=x1).contains(&i64::from(*x)) && (y0..=y1).contains(&i64::from(*y)) {
                current.push((*x, *y, self.tx.blob(hash)?));
            }
        }
        for (x, y, bytes) in tiles::write(&current, format, rect, pixels)? {
            match bytes {
                Some(bytes) => {
                    map.insert((x, y), self.tx.put_blob(&bytes)?);
                }
                None => {
                    map.remove(&(x, y));
                }
            }
        }
        match map.is_empty() {
            true => self.project.tiles.remove(&id),
            false => self.project.tiles.insert(id, map),
        };
        Ok(())
    }

    pub(super) fn add_file(&mut self, bytes: &[u8]) -> Result<()> {
        self.tx.put_blob(bytes)?;
        Ok(())
    }

    /// The size of an added image, which a page made from it takes.
    pub(super) fn image_size(&self, op: usize, file: &Hash) -> Result<Size> {
        let bytes = match self.tx.blob(file) {
            Err(Error::NotFound(_)) => {
                return Err(Error::invalid(op, format!("file {file} was not added")));
            }
            bytes => bytes?,
        };
        let (width, height) = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| Error::invalid(op, error.to_string()))?
            .into_dimensions()
            .map_err(|error| Error::invalid(op, format!("file {file} isn't an image: {error}")))?;
        Ok(Size { width, height })
    }
}
