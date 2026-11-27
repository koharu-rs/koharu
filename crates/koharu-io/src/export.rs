//! Headless export of Koharu project pages to PNG, JPEG, WebP, JPEG XL, PSD,
//! PDF, and multi-codec CBZ archives with single-pass 8-bit grayscale
//! auto-detection.

use std::{io::Write as _, path::Path};

use anyhow::{Context as _, Result, ensure};
use fs_err::File;
use image::{
    ExtendedColorType, ImageEncoder as _, Rgba, RgbaImage,
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
    imageops,
};
use koharu_psd::Labels;
use koharu_renderer::Renderer;
use koharu_scene::{LanguageIdentifier, PageId, Session};
use krilla::{Document, geom::Size, image::Image, page::PageSettings};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{COMIC_INFO_KEY, ComicInfo, ComicPageInfo, import::PDF_SCALE, jxl};

/// What the pages go out as: a folder of images or of PSDs, a CBZ of
/// images, or a PDF.
#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Images(ImageFormat),
    /// With the names of the layers the scene names nothing, in the
    /// interface's language.
    Psd(Labels),
    Cbz(ImageFormat),
    Pdf,
}

/// How a flattened page is written: PNG is lossless; JPEG is lossy at a
/// quality from 1 to 100; WebP and JPEG XL support lossless or lossy at 1 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg {
        #[serde(deserialize_with = "deserialize_quality")]
        #[specta(type = u8)]
        quality: u8,
    },
    Webp {
        #[serde(deserialize_with = "deserialize_quality")]
        #[specta(type = u8)]
        quality: u8,
        #[serde(default)]
        lossless: bool,
    },
    Jxl {
        #[serde(deserialize_with = "deserialize_quality")]
        #[specta(type = u8)]
        quality: u8,
        #[serde(default)]
        lossless: bool,
    },
}

impl ImageFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg { .. } => "jpg",
            Self::Webp { .. } => "webp",
            Self::Jxl { .. } => "jxl",
        }
    }

    fn validate(self) -> Result<()> {
        match self {
            Self::Png => Ok(()),
            Self::Jpeg { quality }
            | Self::Webp { quality, .. }
            | Self::Jxl { quality, .. } => {
                ensure!(
                    (1..=100).contains(&quality),
                    "export quality {quality} must be between 1 and 100"
                );
                Ok(())
            }
        }
    }

    pub fn encode(self, image: &RgbaImage) -> Result<Vec<u8>> {
        self.validate()?;
        let (width, height) = image.dimensions();
        match self {
            Self::Png => encode_png(image),
            // JPEG has no transparency, so a transparent page is drawn over
            // white, as Photoshop's JPEG export does.
            Self::Jpeg { quality } => {
                let mut out = Vec::new();
                let encoder = JpegEncoder::new_with_quality(&mut out, quality);
                match classify(image) {
                    Classified::Gray(l8) => {
                        encoder.write_image(&l8, width, height, ExtendedColorType::L8)?;
                    }
                    Classified::Rgb(rgb) => {
                        encoder.write_image(&rgb, width, height, ExtendedColorType::Rgb8)?;
                    }
                    Classified::Rgba(_) => {
                        let white = composite_white(image);
                        match classify(&white) {
                            Classified::Gray(l8) => {
                                encoder.write_image(&l8, width, height, ExtendedColorType::L8)?;
                            }
                            Classified::Rgb(rgb) => {
                                encoder.write_image(
                                    &rgb,
                                    width,
                                    height,
                                    ExtendedColorType::Rgb8,
                                )?;
                            }
                            Classified::Rgba(_) => unreachable!("white composite is opaque"),
                        }
                    }
                }
                Ok(out)
            }
            Self::Webp { quality, lossless } => {
                // libwebp's `WEBP_MAX_DIMENSION`.
                ensure!(
                    width.max(height) <= 16383,
                    "a WebP image is at most 16383 pixels a side, and the page is {width}×{height}"
                );
                let quality = f32::from(quality);
                let encoded = match classify(image) {
                    Classified::Gray(l8) => {
                        // `webp 0.3.1` only accepts RGB/RGBA inputs.
                        let mut rgb = Vec::with_capacity(l8.len() * 3);
                        for &value in &l8 {
                            rgb.extend_from_slice(&[value, value, value]);
                        }
                        webp::Encoder::from_rgb(&rgb, width, height)
                            .encode_simple(lossless, quality)
                    }
                    Classified::Rgb(rgb) => webp::Encoder::from_rgb(&rgb, width, height)
                        .encode_simple(lossless, quality),
                    Classified::Rgba(rgba) => webp::Encoder::from_rgba(rgba, width, height)
                        .encode_simple(lossless, quality),
                };
                encoded
                    .map(|mem| mem.to_vec())
                    .map_err(|err| anyhow::anyhow!("libwebp failed with {err:?}"))
            }
            Self::Jxl { quality, lossless } => match classify(image) {
                Classified::Gray(l8) => jxl::encode(
                    jxl::JxlPixels::L8(&l8),
                    width,
                    height,
                    lossless,
                    quality,
                ),
                Classified::Rgb(rgb) => jxl::encode(
                    jxl::JxlPixels::Rgb8(&rgb),
                    width,
                    height,
                    lossless,
                    quality,
                ),
                Classified::Rgba(rgba) => jxl::encode(
                    jxl::JxlPixels::Rgba8(rgba),
                    width,
                    height,
                    lossless,
                    quality,
                ),
            },
        }
    }
}

/// Image codec identifier stored in preferences and accepted by the CLI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImageCodec {
    #[default]
    Png,
    Jpeg,
    Webp,
    Jxl,
}

/// Remembered export defaults persisted in `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ExportConfig {
    pub cbz_codec: ImageCodec,
    /// Lossy encoding quality in `1..=100` for JPEG, WebP, and JPEG XL.
    #[serde(deserialize_with = "deserialize_quality")]
    #[specta(type = u8)]
    pub quality: u8,
    /// Lossless encoding for WebP and JPEG XL (PNG is always lossless; JPEG is always lossy).
    pub lossless: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            cbz_codec: ImageCodec::Png,
            quality: 90,
            lossless: false,
        }
    }
}

impl ExportConfig {
    #[must_use]
    pub const fn image_format(&self, codec: ImageCodec) -> ImageFormat {
        match codec {
            ImageCodec::Png => ImageFormat::Png,
            ImageCodec::Jpeg => ImageFormat::Jpeg {
                quality: self.quality,
            },
            ImageCodec::Webp => ImageFormat::Webp {
                quality: self.quality,
                lossless: self.lossless,
            },
            ImageCodec::Jxl => ImageFormat::Jxl {
                quality: self.quality,
                lossless: self.lossless,
            },
        }
    }
}

impl koharu_config::Section for ExportConfig {
    const NAME: &'static str = "export";
}

fn deserialize_quality<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let quality = u8::deserialize(deserializer)?;
    if !(1..=100).contains(&quality) {
        return Err(serde::de::Error::custom(format!(
            "export quality {quality} must be between 1 and 100"
        )));
    }
    Ok(quality)
}

enum Classified<'a> {
    Gray(Vec<u8>),
    Rgb(Vec<u8>),
    Rgba(&'a [u8]),
}

fn classify(image: &RgbaImage) -> Classified<'_> {
    let raw = image.as_raw();
    let mut is_gray = true;
    for pixel in raw.chunks_exact(4) {
        if pixel[3] != 255 {
            return Classified::Rgba(raw);
        }
        is_gray &= pixel[0] == pixel[1] && pixel[1] == pixel[2];
    }
    if is_gray {
        Classified::Gray(raw.chunks_exact(4).map(|pixel| pixel[0]).collect())
    } else {
        let mut rgb = Vec::with_capacity((raw.len() / 4) * 3);
        for pixel in raw.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        Classified::Rgb(rgb)
    }
}

fn composite_white(image: &RgbaImage) -> RgbaImage {
    let mut white = RgbaImage::from_pixel(image.width(), image.height(), Rgba([255; 4]));
    imageops::overlay(&mut white, image, 0, 0);
    // Overlay's rounding can leave white's alpha at 254.
    for pixel in white.pixels_mut() {
        pixel[3] = 255;
    }
    white
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let (width, height) = image.dimensions();
    let mut out = Vec::new();
    // The png crate's own default compression: image's, the fastest, leaves a
    // page in near-grays twice the size.
    let encoder =
        PngEncoder::new_with_quality(&mut out, CompressionType::Default, FilterType::Adaptive);
    match classify(image) {
        Classified::Gray(l8) => {
            encoder.write_image(&l8, width, height, ExtendedColorType::L8)?;
        }
        Classified::Rgb(rgb) => {
            encoder.write_image(&rgb, width, height, ExtendedColorType::Rgb8)?;
        }
        Classified::Rgba(rgba) => {
            encoder.write_image(rgba, width, height, ExtendedColorType::Rgba8)?;
        }
    }
    Ok(out)
}

/// Exports `pages` (or every page when `None`) in project order to `destination`.
pub fn export(
    session: &Mutex<Session>,
    renderer: &Renderer,
    pages: Option<&[PageId]>,
    format: ExportFormat,
    target_language: Option<&LanguageIdentifier>,
    destination: &Path,
) -> Result<()> {
    match &format {
        ExportFormat::Images(image_format) | ExportFormat::Cbz(image_format) => {
            image_format.validate()?;
        }
        ExportFormat::Psd(_) | ExportFormat::Pdf => {}
    }

    let (order, stored_comic_info): (Vec<PageId>, Option<serde_json::Value>) = {
        let locked = session.lock();
        let project = locked.project();
        (
            project.pages().map(|page| page.id).collect(),
            project.metadata(COMIC_INFO_KEY).cloned(),
        )
    };
    let digits = order.len().to_string().len().max(3);
    // A page keeps its number in the project whichever pages go with it, so
    // one exported again replaces its own file.
    let selected_all = pages.is_none();
    let pages = order
        .iter()
        .enumerate()
        .filter(|(_, page)| pages.is_none_or(|pages| pages.contains(page)))
        .map(|(index, &page)| (format!("{:0digits$}", index + 1), page));
    let draw = |page| -> Result<RgbaImage> {
        let frame = renderer.render(&session.lock(), page)?;
        Ok(frame.rasterize()?.image)
    };

    match format {
        ExportFormat::Images(image_format) => {
            let ext = image_format.extension();
            for (number, page) in pages {
                fs_err::write(
                    destination.join(format!("{number}.{ext}")),
                    image_format.encode(&draw(page)?)?,
                )?;
            }
        }
        ExportFormat::Psd(labels) => {
            for (number, page) in pages {
                let psd = koharu_psd::export(&session.lock(), renderer, page, &labels)?;
                fs_err::write(destination.join(format!("{number}.psd")), psd)?;
            }
        }
        ExportFormat::Cbz(image_format) => {
            let mut comic_info: ComicInfo = match stored_comic_info {
                Some(value) => {
                    serde_json::from_value(value).context("invalid stored ComicInfo metadata")?
                }
                None => ComicInfo::default(),
            };
            let previous_pages = if selected_all && comic_info.pages.len() == order.len() {
                std::mem::take(&mut comic_info.pages)
            } else {
                comic_info.pages.clear();
                Vec::new()
            };

            let mut archive = ZipWriter::new(File::create(destination)?);
            // Already-compressed images gain nothing from deflate.
            let stored_options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let ext = image_format.extension();
            let mut exported_pages = Vec::new();
            for (export_index, (number, page)) in pages.enumerate() {
                let image = draw(page)?;
                let (width, height) = image.dimensions();
                let encoded = image_format.encode(&image)?;
                let filename = format!("{number}.{ext}");
                archive.start_file(&filename, stored_options)?;
                archive.write_all(&encoded)?;

                let prev = previous_pages.get(export_index);
                exported_pages.push(ComicPageInfo {
                    image: export_index as i32,
                    page_type: prev.and_then(|p| p.page_type),
                    double_page: (width > height).then_some(true),
                    image_size: Some(encoded.len() as i64),
                    key: Some(filename),
                    bookmark: prev.and_then(|p| p.bookmark.clone()),
                    image_width: Some(width as i32),
                    image_height: Some(height as i32),
                });
            }

            comic_info.page_count = Some(exported_pages.len() as i32);
            comic_info.pages = exported_pages;
            if let Some(lang) = target_language.filter(|lang| **lang != LanguageIdentifier::UNKNOWN)
            {
                comic_info.language_iso = Some(lang.to_string());
            }
            let xml = comic_info.to_xml_string()?;
            archive.start_file(
                "ComicInfo.xml",
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )?;
            archive.write_all(xml.as_bytes())?;
            archive.finish()?;
        }
        ExportFormat::Pdf => {
            let mut document = Document::new();
            for (_, page) in pages {
                let image = draw(page)?;
                let (width, height) = image.dimensions();
                // Import's own scale, so a page read back keeps its pixels.
                let size = Size::from_wh(width as f32 / PDF_SCALE, height as f32 / PDF_SCALE)
                    .context("a page has no pixels")?;
                document
                    .start_page_with(PageSettings::new(size))
                    .surface()
                    .draw_image(Image::from_rgba8(image.into_raw(), width, height), size);
            }
            fs_err::write(destination, document.finish()?)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use image::{ImageDecoder as _, ImageFormat as ImgFmt, Rgba, codecs::png::PngDecoder};

    use super::*;

    fn grayscale_image(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let v = ((x * 7 + y * 13) % 256) as u8;
            Rgba([v, v, v, 255])
        })
    }

    #[test]
    fn export_config_defaults_and_rejects_out_of_range_quality() {
        let cfg = ExportConfig::default();
        assert_eq!(cfg.cbz_codec, ImageCodec::Png);
        assert_eq!(cfg.quality, 90);
        assert!(!cfg.lossless);

        assert!(serde_json::from_str::<ExportConfig>(r#"{"quality":0}"#).is_err());
        assert!(serde_json::from_str::<ExportConfig>(r#"{"quality":101}"#).is_err());
        let valid: ExportConfig =
            serde_json::from_str(r#"{"cbz_codec":"jxl","quality":100,"lossless":true}"#).unwrap();
        assert_eq!(valid.cbz_codec, ImageCodec::Jxl);
        assert_eq!(valid.quality, 100);
        assert!(valid.lossless);

        let rgb = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255]));
        assert!(ImageFormat::Jpeg { quality: 0 }.encode(&rgb).is_err());
        assert!(
            ImageFormat::Webp {
                quality: 101,
                lossless: false,
            }
            .encode(&rgb)
            .is_err()
        );
    }

    #[test]
    fn grayscale_auto_detection_classifies_l8_rgb8_and_rgba8_correctly() {
        let gray = grayscale_image(32, 32);
        assert!(matches!(classify(&gray), Classified::Gray(_)));
        let png_bytes = encode_png(&gray).unwrap();
        let decoder = PngDecoder::new(std::io::Cursor::new(&png_bytes)).unwrap();
        assert_eq!(decoder.color_type(), image::ColorType::L8);

        let mut rgb = gray.clone();
        rgb.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
        assert!(matches!(classify(&rgb), Classified::Rgb(_)));
        let png_rgb = encode_png(&rgb).unwrap();
        let decoder = PngDecoder::new(std::io::Cursor::new(&png_rgb)).unwrap();
        assert_eq!(decoder.color_type(), image::ColorType::Rgb8);

        let mut rgba = rgb;
        rgba.put_pixel(1, 1, Rgba([10, 10, 10, 128]));
        assert!(matches!(classify(&rgba), Classified::Rgba(_)));
        let png_rgba = encode_png(&rgba).unwrap();
        let decoder = PngDecoder::new(std::io::Cursor::new(&png_rgba)).unwrap();
        assert_eq!(decoder.color_type(), image::ColorType::Rgba8);
    }

    #[test]
    fn encodes_pages_in_png_jpeg_webp_and_jxl() {
        let gray = grayscale_image(32, 32);
        let rgba = RgbaImage::from_pixel(16, 16, Rgba([20, 40, 60, 128]));

        for (img, label) in [(&gray, "gray"), (&rgba, "rgba")] {
            let png = ImageFormat::Png.encode(img).unwrap();
            assert_eq!(image::guess_format(&png).unwrap(), ImgFmt::Png, "{label}");

            let jpeg = ImageFormat::Jpeg { quality: 85 }.encode(img).unwrap();
            assert_eq!(image::guess_format(&jpeg).unwrap(), ImgFmt::Jpeg, "{label}");

            let webp_lossy = ImageFormat::Webp {
                quality: 80,
                lossless: false,
            }
            .encode(img)
            .unwrap();
            assert_eq!(
                image::guess_format(&webp_lossy).unwrap(),
                ImgFmt::WebP,
                "{label}"
            );

            let webp_lossless = ImageFormat::Webp {
                quality: 80,
                lossless: true,
            }
            .encode(img)
            .unwrap();
            assert_eq!(
                image::guess_format(&webp_lossless).unwrap(),
                ImgFmt::WebP,
                "{label}"
            );

            let jxl_lossy = ImageFormat::Jxl {
                quality: 85,
                lossless: false,
            }
            .encode(img)
            .unwrap();
            assert!(
                jxl_lossy.starts_with(&[0xFF, 0x0A]),
                "{label} jxl codestream header"
            );

            let jxl_lossless = ImageFormat::Jxl {
                quality: 85,
                lossless: true,
            }
            .encode(img)
            .unwrap();
            assert!(
                jxl_lossless.starts_with(&[0xFF, 0x0A]),
                "{label} jxl lossless header"
            );
        }
    }

    #[test]
    fn corpus_grayscale_cbz_export_reduces_size_across_codecs() {
        let corpus_dir = Path::new("/home/finn/よつばとtesting");
        let sample = corpus_dir.join("よつばと!_1_003.jpg");
        if !sample.exists() {
            return;
        }
        let img = image::open(&sample).unwrap().to_rgba8();
        let gray_scan = RgbaImage::from_fn(img.width(), img.height(), |x, y| {
            let p = img.get_pixel(x, y);
            let lum = ((u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3) as u8;
            Rgba([lum, lum, lum, 255])
        });

        let mut raw_rgba_png = Vec::new();
        PngEncoder::new(&mut raw_rgba_png)
            .write_image(
                gray_scan.as_raw(),
                gray_scan.width(),
                gray_scan.height(),
                ExtendedColorType::Rgba8,
            )
            .unwrap();

        let l8_png = ImageFormat::Png.encode(&gray_scan).unwrap();
        let webp_lossy = ImageFormat::Webp {
            quality: 85,
            lossless: false,
        }
        .encode(&gray_scan)
        .unwrap();
        let jxl_lossless = ImageFormat::Jxl {
            quality: 90,
            lossless: true,
        }
        .encode(&gray_scan)
        .unwrap();
        let jxl_lossy = ImageFormat::Jxl {
            quality: 85,
            lossless: false,
        }
        .encode(&gray_scan)
        .unwrap();

        assert!(
            l8_png.len() < raw_rgba_png.len(),
            "L8 PNG ({}) should be smaller than RGBA8 PNG ({})",
            l8_png.len(),
            raw_rgba_png.len()
        );
        assert!(webp_lossy.len() < raw_rgba_png.len());
        assert!(jxl_lossless.len() < raw_rgba_png.len());
        assert!(jxl_lossy.len() < raw_rgba_png.len());

        let tmp = tempfile::tempdir().unwrap();
        let cbz_path = tmp.path().join("out.cbz");
        {
            let mut archive = ZipWriter::new(File::create(&cbz_path).unwrap());
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            archive.start_file("001.jxl", opts).unwrap();
            archive.write_all(&jxl_lossy).unwrap();
            archive.finish().unwrap();
        }
        let mut reader = zip::ZipArchive::new(File::open(&cbz_path).unwrap()).unwrap();
        assert_eq!(reader.len(), 1);
        let mut entry = reader.by_index(0).unwrap();
        assert_eq!(entry.name().unwrap(), "001.jxl");
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, jxl_lossy);
    }

    #[test]
    fn cbz_export_and_reimport_round_trips_comic_info_with_corpus_pages() {
        let corpus_dir = Path::new("/home/finn/よつばとtesting");
        let p1 = corpus_dir.join("よつばと!_1_003.jpg");
        let p2 = corpus_dir.join("よつばと!_1_004-005.jpg");
        if !p1.exists() || !p2.exists() {
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let session = Mutex::new(
            Session::create(&tmp.path().join("proj.khr"), koharu_scene::Meta::default()).unwrap(),
        );
        crate::import_paths(&session, &[p1, p2], None).unwrap();

        let info = ComicInfo {
            title: Some(crate::LocalizedField {
                original: "あつい夏".to_owned(),
                translated: Some("Hot Summer".to_owned()),
            }),
            series: Some(crate::LocalizedField::new("よつばと!")),
            number: Some("1".to_owned()),
            language_iso: Some("ja".to_owned()),
            manga: Some(crate::Manga::YesAndRightToLeft),
            ..Default::default()
        };
        session
            .lock()
            .apply(koharu_scene::Edit {
                label: "set_metadata".to_owned(),
                ops: vec![koharu_scene::Op::SetProjectMetadata {
                    key: COMIC_INFO_KEY.to_owned(),
                    value: Some(serde_json::to_value(&info).unwrap()),
                }],
            })
            .unwrap();

        let out_cbz = tmp.path().join("exported.cbz");
        let target_lang: LanguageIdentifier = "en".parse().unwrap();
        export(
            &session,
            Renderer::get(),
            None,
            ExportFormat::Cbz(ImageFormat::Png),
            Some(&target_lang),
            &out_cbz,
        )
        .unwrap();

        let mut archive = zip::ZipArchive::new(File::open(&out_cbz).unwrap()).unwrap();
        let xml_bytes = {
            let mut xml_entry = archive.by_name("ComicInfo.xml").unwrap();
            assert_eq!(xml_entry.compression(), CompressionMethod::Deflated);
            let mut buf = Vec::new();
            xml_entry.read_to_end(&mut buf).unwrap();
            buf
        };
        let parsed = ComicInfo::from_xml_bytes(&xml_bytes).unwrap().unwrap();
        assert_eq!(parsed.title.as_ref().unwrap().original, "Hot Summer");
        assert_eq!(parsed.series.as_ref().unwrap().original, "よつばと!");
        assert_eq!(parsed.language_iso.as_deref(), Some("en"));
        assert_eq!(parsed.page_count, Some(2));
        assert_eq!(parsed.pages.len(), 2);
        assert_eq!(parsed.pages[0].key.as_deref(), Some("001.png"));
        assert_eq!(parsed.pages[0].double_page, None);
        assert_eq!(parsed.pages[1].key.as_deref(), Some("002.png"));
        assert_eq!(parsed.pages[1].double_page, Some(true));
        assert!(parsed.pages[0].image_width.unwrap() > 0);
        assert!(parsed.pages[0].image_size.unwrap() > 0);

        let reimported = Mutex::new(
            Session::create(
                &tmp.path().join("reimport.khr"),
                koharu_scene::Meta::default(),
            )
            .unwrap(),
        );
        let report = crate::import_paths(&reimported, &[out_cbz], None).unwrap();
        assert_eq!(report.added_pages, 2);
        assert!(report.skipped_files.is_empty());
        let locked = reimported.lock();
        let stored: ComicInfo =
            serde_json::from_value(locked.project().metadata(COMIC_INFO_KEY).unwrap().clone())
                .unwrap();
        assert_eq!(stored.title.as_ref().unwrap().original, "Hot Summer");
        assert_eq!(stored.page_count, Some(2));
        assert_eq!(stored.language_iso.as_deref(), Some("en"));
    }
}
