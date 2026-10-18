use std::{fs::File, io::Read, path::Path};

use anyhow::{Context as _, Result, bail};

use super::{EncodedPage, Format};

pub(super) fn extract(path: &Path) -> Result<Vec<EncodedPage>> {
    let file = File::open(path)
        .with_context(|| format!("failed to open ZIP/CBZ container {}", path.display()))?;
    let mut archive = ::zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read ZIP/CBZ container {}", path.display()))?;
    let mut images = Vec::new();

    for index in 0..archive.len() {
        let mut member = archive.by_index(index).with_context(|| {
            format!(
                "failed to read member {index} from ZIP/CBZ container {}",
                path.display()
            )
        })?;
        if member.is_dir() {
            continue;
        }
        let name = member
            .name()
            .with_context(|| {
                format!(
                    "failed to decode member {index} name in ZIP/CBZ container {}",
                    path.display()
                )
            })?
            .into_owned();
        let member_path = Path::new(&name);
        let supported = member_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension.parse(), Ok(Format::Raster)));
        let metadata = member_path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .is_none_or(|file_name| {
                file_name.starts_with("._")
                    || [".ds_store", "thumbs.db", "desktop.ini"]
                        .iter()
                        .any(|candidate| file_name.eq_ignore_ascii_case(candidate))
            })
            || member_path.components().any(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .is_some_and(|component| component.eq_ignore_ascii_case("__macosx"))
            });
        if !supported || metadata {
            continue;
        }
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).with_context(|| {
            format!(
                "failed to read image member {name:?} from ZIP/CBZ container {}",
                path.display()
            )
        })?;
        images.push(EncodedPage { name, bytes });
    }

    alphanumeric_sort::sort_slice_by_os_str_key(&mut images, |image| {
        std::ffi::OsStr::new(&image.name)
    });
    if images.is_empty() {
        bail!(
            "ZIP/CBZ container {} contains no supported raster images",
            path.display()
        );
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageFormat;
    use std::{
        io::{Cursor, Write},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn sample_png() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode PNG");
        bytes.into_inner()
    }

    fn archive_path(suffix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "koharu-import-{}-{timestamp}.{suffix}",
            std::process::id()
        ))
    }

    fn write_archive(entries: &[(&str, &[u8])]) -> PathBuf {
        let path = archive_path("cbz");
        let file = File::create(&path).expect("create archive");
        let mut archive = ::zip::ZipWriter::new(file);
        let options = ::zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            archive.start_file(*name, options).expect("start member");
            archive.write_all(bytes).expect("write member");
        }
        archive.finish().expect("finish archive");
        path
    }

    #[test]
    fn reads_images_in_natural_member_order_and_skips_metadata() {
        let png = sample_png();
        let path = write_archive(&[
            ("ComicInfo.xml", b"<ComicInfo />"),
            ("pages/page10.png", &png),
            ("pages/page2.png", &png),
            ("__MACOSX/._page3.png", &png),
        ]);
        let images = extract(&path).expect("read archive");
        std::fs::remove_file(&path).expect("remove archive");
        assert_eq!(
            images
                .iter()
                .map(|image| image.name.as_str())
                .collect::<Vec<_>>(),
            ["pages/page2.png", "pages/page10.png"]
        );
    }

    #[test]
    fn preserves_unicode_member_names() {
        let png = sample_png();
        let path = write_archive(&[("漫画/ページ.png", &png)]);
        let images = extract(&path).expect("read archive");
        std::fs::remove_file(&path).expect("remove archive");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "漫画/ページ.png");
        assert_eq!(images[0].bytes, png);
    }

    #[test]
    fn rejects_archive_without_supported_images() {
        let path = write_archive(&[("ComicInfo.xml", b"<ComicInfo />")]);
        let error = extract(&path).expect_err("archive should be rejected");
        std::fs::remove_file(&path).expect("remove archive");
        assert!(error.to_string().contains("no supported raster images"));
    }
}
