//! The archives packages come in: Koharu's own `.tar.gz` releases and
//! Python's `.whl` wheels.

use std::{io::copy, path::Path};

use anyhow::{Context, Result};
use fast_glob::glob_match;
use flate2::read::GzDecoder;
use fs_err::{File, create_dir_all};

use crate::transfer;

/// The files an archive of libraries is extracted for.
pub(super) const LIBRARIES: &[&str] = &["**/*.dll", "**/*.dylib", "**/*.so", "**/*.so.*"];

/// Fetches an archive into `stage`, on the store's own volume, and extracts
/// the files matching `patterns` off the async threads.
pub(super) async fn extract(
    url: &str,
    stage: &Path,
    patterns: &'static [&'static str],
) -> Result<()> {
    let name = url
        .rsplit('/')
        .next()
        .context("an archive URL has no name")?;
    let archive = stage.join(name);
    transfer::fetch(url, &archive).await?;
    let stage = stage.to_owned();
    tokio::task::spawn_blocking(move || {
        unpack(&archive, &stage, patterns)?;
        fs_err::remove_file(&archive)?;
        Ok(())
    })
    .await?
}

fn unpack(archive: &Path, destination: &Path, patterns: &[&str]) -> Result<()> {
    let selected = |path: &Path| {
        patterns
            .iter()
            .any(|pattern| glob_match(pattern, path.to_string_lossy().as_bytes()))
    };
    match archive.extension().and_then(|extension| extension.to_str()) {
        Some("whl") => extract_wheel(archive, destination, selected),
        Some("gz") => extract_tarball(archive, destination, selected),
        _ => anyhow::bail!("{} isn't an archive Koharu reads", archive.display()),
    }
}

fn extract_wheel(
    archive: &Path,
    destination: &Path,
    selected: impl Fn(&Path) -> bool,
) -> Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if entry.is_dir() || !selected(&path) {
            continue;
        }
        let output = destination.join(path);
        create_dir_all(output.parent().context("an archive entry has no parent")?)?;
        copy(&mut entry, &mut File::create(output)?)?;
    }
    Ok(())
}

fn extract_tarball(
    archive: &Path,
    destination: &Path,
    selected: impl Fn(&Path) -> bool,
) -> Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(archive)?));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if selected(&entry.path()?) {
            entry.unpack_in(destination)?;
        }
    }
    Ok(())
}
