//! A ZIP or CBZ archive's images.

use std::{io::Read as _, path::Path};

use anyhow::{Context as _, Result, bail};
use fs_err::File;

use super::{EncodedPage, metadata, raster};

pub(super) fn extract(path: &Path) -> Result<Vec<EncodedPage>> {
    let file = File::open(path)?;
    let mut archive = ::zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read the archive {}", path.display()))?;
    let mut images = Vec::new();
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .with_context(|| format!("failed to read member {index} of {}", path.display()))?;
        if member.is_dir() {
            continue;
        }
        let name = member
            .name()
            .with_context(|| format!("failed to read member {index}'s name in {}", path.display()))?
            .into_owned();
        if !raster(Path::new(&name)) || metadata(Path::new(&name)) {
            continue;
        }
        let mut bytes = Vec::new();
        member
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read {name:?} in {}", path.display()))?;
        images.push(EncodedPage { name, bytes });
    }
    alphanumeric_sort::sort_slice_by_os_str_key(&mut images, |image| {
        std::ffi::OsStr::new(&image.name)
    });
    if images.is_empty() {
        bail!("{} holds no images", path.display());
    }
    Ok(images)
}
