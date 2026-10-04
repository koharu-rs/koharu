use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use image::{DynamicImage, ImageFormat, ImageReader};
use koharu_ml::webtoon::{SliceParams, plan_slices, row_profile};
use rayon::prelude::*;
use strum::{EnumIter, EnumMessage, EnumString};

mod pdf;
mod rar;
mod zip;

/// JPEG quality for a band re-encoded from a lossy source.
///
/// A band is re-encoded rather than losslessly cropped because the source pixels cannot be
/// copied without re-running a lossless codec over them anyway; 92 keeps grain and thin
/// strokes, which is what the detector reads, while staying far smaller than PNG.
const BAND_JPEG_QUALITY: u8 = 92;

#[derive(Clone, Copy, EnumIter, EnumMessage, EnumString)]
#[strum(ascii_case_insensitive)]
pub(super) enum Format {
    #[strum(
        serialize = "png",
        serialize = "jpg",
        serialize = "jpeg",
        serialize = "webp"
    )]
    Raster,
    #[strum(serialize = "cbz", serialize = "zip")]
    Zip,
    #[strum(serialize = "rar")]
    Rar,
    #[strum(serialize = "pdf")]
    Pdf,
}

/// Whether tall images are cut into pages on the way in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slicing {
    /// Cut an image only when its geometry says it is a webtoon.
    Auto,
    /// Cut every image that is taller than a single page, whatever its aspect ratio.
    Forced,
}

impl Slicing {
    /// Planner parameters for this mode. Forcing relaxes the geometry gate rather than
    /// bypassing the planner, so a forced import of an ordinary page still produces exactly
    /// one page instead of a hand-rolled second cutting path.
    fn params(self) -> SliceParams {
        match self {
            Self::Auto => SliceParams::default(),
            Self::Forced => SliceParams {
                trigger_aspect: 0.0,
                min_sliceable_height: 0,
                ..SliceParams::default()
            },
        }
    }
}

#[derive(Debug)]
pub(super) struct EncodedPage {
    pub(super) name: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) struct Page {
    pub(super) name: String,
    pub(super) bytes: Arc<[u8]>,
    pub(super) format: ImageFormat,
    pub(super) width: u32,
    pub(super) height: u32,
}

/// One imported source image, already reduced to the pages the project should hold.
pub(super) enum Imported {
    /// An image that is already one readable page.
    Page(Page),
    /// A tall image divided into pages at import time.
    Strip {
        /// The uncut image, retained so the strip can be cut again with other boundaries.
        source: Arc<[u8]>,
        width: u32,
        height: u32,
        bands: Vec<Band>,
    },
}

/// One page cut out of a tall image.
pub(super) struct Band {
    /// Distance from the top of the uncut image, recorded as the page's provenance.
    pub(super) y_offset: u32,
    pub(super) page: Page,
}

fn decode(path: &Path, source: EncodedPage) -> Result<Page> {
    let EncodedPage { name, bytes } = source;
    let format = image::guess_format(&bytes).with_context(|| {
        format!(
            "failed to identify imported image {} ({name})",
            path.display()
        )
    })?;
    let (width, height) = ImageReader::with_format(Cursor::new(bytes.as_slice()), format)
        .into_dimensions()
        .with_context(|| {
            format!(
                "failed to read dimensions of imported image {} ({name})",
                path.display()
            )
        })?;
    Ok(Page {
        name,
        bytes: Arc::<[u8]>::from(bytes),
        format,
        width,
        height,
    })
}

pub(super) fn import(paths: Vec<PathBuf>, slicing: Slicing) -> Result<Vec<Imported>> {
    let pages = read(paths)?;
    Ok(cut(pages, slicing))
}

/// Reads and sorts every supported page source without deciding how tall images are divided.
fn read(mut paths: Vec<PathBuf>) -> Result<Vec<Page>> {
    alphanumeric_sort::sort_slice_by_os_str_key(&mut paths, |path| {
        path.file_name().unwrap_or_else(|| path.as_os_str())
    });
    let mut groups = paths
        .into_par_iter()
        .map(|path| -> Result<Vec<Page>> {
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(|extension| extension.parse::<Format>().ok());
            let encoded = match extension {
                Some(Format::Raster) => vec![EncodedPage {
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "page".to_owned()),
                    bytes: fs::read(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?,
                }],
                Some(Format::Zip) => zip::extract(&path)?,
                Some(Format::Rar) => rar::extract(&path)?,
                Some(Format::Pdf) => pdf::render(&path)?,
                None => bail!("unsupported page import path {}", path.display()),
            };
            encoded
                .into_iter()
                .map(|source| decode(&path, source))
                .collect()
        })
        .collect::<Result<Vec<_>>>()?;
    let page_count = groups.iter().map(Vec::len).sum();
    let mut pages = Vec::with_capacity(page_count);
    for group in &mut groups {
        pages.append(group);
    }
    Ok(pages)
}

/// Divides webtoon strips into pages and leaves every other image whole.
///
/// Whether a strip is cut is the planner's decision: `plan_slices` returns `None` for
/// anything it considers readable as one page, so the geometry rule lives in exactly one
/// place. The local pre-check exists only to skip the full decode of images that cannot
/// possibly be cut, and it reads the planner's own thresholds instead of restating them.
fn cut(pages: Vec<Page>, slicing: Slicing) -> Vec<Imported> {
    let params = slicing.params();
    pages
        .into_par_iter()
        .map(|page| cut_one(page, &params))
        .collect()
}

fn cut_one(page: Page, params: &SliceParams) -> Imported {
    if !may_be_a_webtoon(page.width, page.height, params) {
        return Imported::Page(page);
    }
    let image = match image::load_from_memory_with_format(&page.bytes, page.format) {
        Ok(image) => image,
        Err(error) => {
            // Dimensions were already read from the header, so a decode failure here means a
            // truncated or corrupt body. Importing the original keeps the user's file visible
            // instead of dropping it over a failed optimisation.
            tracing::warn!(%error, page = %page.name, "could not decode an imported image for slicing");
            return Imported::Page(page);
        }
    };
    let profile = row_profile(&image.to_luma8());
    let Some(plan) = plan_slices(page.width, page.height, &profile, params) else {
        return Imported::Page(page);
    };
    // A plan that cuts nothing describes the image that is already there. Reporting it as a
    // strip would give an ordinary page a band record claiming it was cut out of itself.
    if plan.page_count() <= 1 {
        return Imported::Page(page);
    }
    match band(&page, &image, &plan) {
        Ok(bands) => Imported::Strip {
            source: page.bytes,
            width: plan.width,
            height: plan.height,
            bands,
        },
        Err(error) => {
            tracing::warn!(%error, page = %page.name, "could not cut an imported webtoon strip");
            Imported::Page(page)
        }
    }
}

fn may_be_a_webtoon(width: u32, height: u32, params: &SliceParams) -> bool {
    height > params.min_sliceable_height && height as f32 > params.trigger_aspect * width as f32
}

fn band(
    page: &Page,
    image: &DynamicImage,
    plan: &koharu_ml::webtoon::SlicePlan,
) -> Result<Vec<Band>> {
    let (format, encoder): (ImageFormat, BandEncoder) = match page.format {
        ImageFormat::Png => (ImageFormat::Png, BandEncoder::Lossless),
        _ => (ImageFormat::Jpeg, BandEncoder::Jpeg),
    };
    let mut bands = Vec::with_capacity(plan.page_count());
    for (index, (y_offset, height)) in plan.page_ranges().into_iter().enumerate() {
        let cropped = image.crop_imm(0, y_offset, plan.width, height);
        bands.push(Band {
            y_offset,
            page: Page {
                // Strip pages are unnamed parts of one file, so a positional name is the only
                // label that stays meaningful without inventing chapter numbering.
                name: format!("{} {:02}", strip_stem(&page.name), index + 1),
                bytes: encode(&cropped, encoder)?,
                format,
                width: plan.width,
                height,
            },
        });
    }
    Ok(bands)
}

fn strip_stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

#[derive(Clone, Copy)]
enum BandEncoder {
    /// A PNG source is cut without a second lossy generation.
    Lossless,
    Jpeg,
}

fn encode(image: &DynamicImage, encoder: BandEncoder) -> Result<Arc<[u8]>> {
    let mut bytes = Cursor::new(Vec::new());
    match encoder {
        BandEncoder::Lossless => image.write_to(&mut bytes, ImageFormat::Png)?,
        BandEncoder::Jpeg => {
            let mut jpeg =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, BAND_JPEG_QUALITY);
            jpeg.encode_image(&image.to_rgb8())?;
        }
    }
    Ok(Arc::from(bytes.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str, image: &DynamicImage) -> PathBuf {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("koharu-import-{}-{timestamp}", std::process::id()));
        fs::create_dir_all(&directory).expect("create fixture directory");
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("encode fixture");
        let path = directory.join(name);
        fs::write(&path, encoded.get_ref()).expect("write fixture");
        path
    }

    /// A strip of `height` rows separated by white gutters, so the planner has real cut
    /// candidates rather than having to fall back to the flattest row.
    fn strip_image(width: u32, height: u32) -> DynamicImage {
        let mut image =
            image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
        for top in (0..height).step_by(1600) {
            for y in top..(top + 1200).min(height) {
                for x in 0..width {
                    image.put_pixel(x, y, image::Rgba([20, 20, 20, 255]));
                }
            }
        }
        DynamicImage::ImageRgba8(image)
    }

    #[test]
    fn top_level_paths_are_naturally_sorted() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "koharu-import-order-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create fixture directory");
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("encode fixture");
        let paths = ["page10.PNG", "page2.png", "page1.png"].map(|name| directory.join(name));
        for path in &paths {
            fs::write(path, encoded.get_ref()).expect("write fixture");
        }

        let pages = read(paths.into()).expect("read fixtures");
        fs::remove_dir_all(&directory).expect("remove fixture directory");
        assert_eq!(
            pages
                .iter()
                .map(|page| page.name.as_str())
                .collect::<Vec<_>>(),
            ["page1.png", "page2.png", "page10.PNG"]
        );
    }

    #[test]
    fn ordinary_pages_are_never_cut_and_webtoons_always_are() {
        let page = fixture("page.png", &strip_image(800, 1600));
        let strip = fixture("chapter.png", &strip_image(360, 6000));
        let paths = vec![page.clone(), strip.clone()];

        // Imports are ordered by file name, so entries are looked up rather than indexed.
        fn split<'a>(imported: &'a [Imported], stem: &str) -> &'a Imported {
            imported
                .iter()
                .find(|entry| match entry {
                    Imported::Page(page) => page.name.starts_with(stem),
                    Imported::Strip { bands, .. } => bands
                        .first()
                        .is_some_and(|band| band.page.name.starts_with(stem)),
                })
                .unwrap_or_else(|| panic!("{stem} is missing from the import"))
        }

        let imported = import(paths.clone(), Slicing::Auto).expect("import fixtures");
        assert!(matches!(split(&imported, "page"), Imported::Page(page) if page.height == 1600));
        let Imported::Strip {
            width,
            height,
            bands,
            ..
        } = split(&imported, "chapter")
        else {
            panic!("a 1:16 image must be cut on import");
        };
        assert_eq!((*width, *height), (360, 6000));
        assert!(bands.len() > 1);
        // Bands tile the strip exactly, in reading order, and stay full width.
        let mut cursor = 0;
        for band in bands {
            assert_eq!(band.y_offset, cursor);
            assert_eq!(band.page.width, 360);
            assert_eq!(
                image::load_from_memory(&band.page.bytes).unwrap().height(),
                band.page.height
            );
            cursor += band.page.height;
        }
        assert_eq!(cursor, 6000);

        // Forcing relaxes the aspect gate without producing a second cutting path: an ordinary
        // page still arrives whole, because the planner finds no legal cut for it.
        let forced = import(paths, Slicing::Forced).expect("import fixtures");
        assert!(matches!(split(&forced, "page"), Imported::Page(page) if page.height == 1600));
        assert!(
            matches!(split(&forced, "chapter"), Imported::Strip { bands, .. } if bands.len() > 1)
        );

        fs::remove_dir_all(page.parent().unwrap()).expect("remove fixture directory");
    }
}
