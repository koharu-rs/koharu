//! A RAR or CBR archive's images.

use std::{
    cell::RefCell,
    io::{self, Write},
    path::Path,
    rc::Rc,
};

use anyhow::{Context as _, Result, bail};
use rars::ArchiveReader;

use super::{EncodedPage, metadata, raster};

/// Collects one member's bytes, and hands them over when rars drops it.
struct PageWriter {
    name: String,
    bytes: Vec<u8>,
    images: Rc<RefCell<Vec<EncodedPage>>>,
}

impl Write for PageWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for PageWriter {
    fn drop(&mut self) {
        self.images.borrow_mut().push(EncodedPage {
            name: std::mem::take(&mut self.name),
            bytes: std::mem::take(&mut self.bytes),
        });
    }
}

pub(super) fn extract(path: &Path) -> Result<Vec<EncodedPage>> {
    let archive = ArchiveReader::read_path(path)
        .with_context(|| format!("failed to open the archive {}", path.display()))?;
    let images = Rc::new(RefCell::new(Vec::new()));
    archive
        .extract_to(None, |entry| {
            let name = entry.name_lossy();
            let member = Path::new(&name);
            if entry.is_directory || !raster(member) || metadata(member) {
                Ok(Box::new(io::sink()))
            } else {
                Ok(Box::new(PageWriter {
                    name,
                    bytes: Vec::new(),
                    images: Rc::clone(&images),
                }))
            }
        })
        .with_context(|| format!("failed to extract {}", path.display()))?;
    let mut images = std::mem::take(&mut *images.borrow_mut());
    alphanumeric_sort::sort_slice_by_os_str_key(&mut images, |image| {
        std::ffi::OsStr::new(&image.name)
    });
    if images.is_empty() {
        bail!("{} holds no images", path.display());
    }
    Ok(images)
}
