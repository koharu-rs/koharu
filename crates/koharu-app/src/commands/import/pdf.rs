use std::{fs, path::Path};

use anyhow::{Context as _, Result, bail};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render as render_page};

use super::EncodedPage;

// PDF coordinates are points at 72 per inch. Four and one sixth points-to-pixels
// therefore gives 300 DPI, retaining fine manga lettering and line art. A typical
// A4 page is about 33 MiB as RGBA pixels at this scale, and rendering one page at
// a time releases that working buffer before the next page is processed.
const PDF_SCALE: f32 = 300.0 / 72.0;

pub(super) fn render(path: &Path) -> Result<Vec<EncodedPage>> {
    let data = fs::read(path).with_context(|| format!("failed to read PDF {}", path.display()))?;
    let pdf = Pdf::new(data)
        .map_err(|error| anyhow::anyhow!("failed to parse PDF {}: {error:?}", path.display()))?;
    let pages = pdf.pages();
    let page_count = pages.len();

    if page_count == 0 {
        bail!("PDF {} contains no pages", path.display());
    }

    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "page".to_owned());
    let page_number_width = page_count.to_string().len().max(3);
    let interpreter_settings = InterpreterSettings::default();
    let render_settings = RenderSettings::default();
    let pixmap_settings = PixmapSettings {
        x_scale: PDF_SCALE,
        y_scale: PDF_SCALE,
        bg_color: WHITE,
    };
    let cache = RenderCache::new();

    pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let page_number = index + 1;
            let bytes = render_page(
                page,
                &cache,
                &interpreter_settings,
                &render_settings,
                &pixmap_settings,
            )
            .into_png()
            .with_context(|| {
                format!(
                    "failed to encode PDF page {page_number} of {} as PNG",
                    path.display()
                )
            })?;

            Ok(EncodedPage {
                name: format!(
                    "{stem}-{page_number:0page_number_width$}.png",
                    page_number_width = page_number_width
                ),
                bytes,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        time::{SystemTime, UNIX_EPOCH},
    };

    use image::GenericImageView as _;

    use super::*;

    fn single_page_pdf() -> Vec<u8> {
        let content = b"0 0 0 rg\n0 0 36 72 re f\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> /Contents 4 0 R >>"
                .to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{}endstream",
                content.len(),
                String::from_utf8_lossy(content)
            ),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        let mut trailer = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
        for offset in offsets {
            writeln!(&mut trailer, "{offset:010} 00000 n ").expect("write xref");
        }
        write!(
            &mut trailer,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .expect("write trailer");
        pdf.extend_from_slice(trailer.as_bytes());
        pdf
    }

    #[test]
    fn renders_pdf_pages_as_300_dpi_pngs() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "koharu-import-pdf-{}-{timestamp}.pdf",
            std::process::id()
        ));
        fs::write(&path, single_page_pdf()).expect("write PDF fixture");
        let pages = render(&path).expect("render PDF");
        fs::remove_file(&path).expect("remove PDF fixture");

        assert_eq!(pages.len(), 1);
        assert!(pages[0].name.ends_with("-001.png"));
        let image = image::load_from_memory_with_format(&pages[0].bytes, image::ImageFormat::Png)
            .expect("decode rendered page");
        assert_eq!(image.dimensions(), (300, 300));
        assert_eq!(image.get_pixel(10, 10), image::Rgba([0, 0, 0, 255]));
        assert_eq!(image.get_pixel(290, 10), image::Rgba([255, 255, 255, 255]));
    }
}
