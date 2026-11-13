//! A PDF's pages, rendered with hayro.

use std::{fs, path::Path};

use anyhow::{Context as _, Result, bail};
use hayro::{
    PixmapSettings, RenderCache, RenderSettings, hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf, render as render_page, vello_cpu::color::palette::css::WHITE,
};

use super::EncodedPage;

/// PDF points to pixels at 300 DPI, which keeps fine lettering and line art.
/// An A4 page is about 33 MiB of RGBA at this scale; one page renders at a
/// time, so its buffer goes before the next.
const SCALE: f32 = 300.0 / 72.0;

pub(super) fn render(path: &Path) -> Result<Vec<EncodedPage>> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let pdf = Pdf::new(data)
        .map_err(|error| anyhow::anyhow!("failed to parse {}: {error:?}", path.display()))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        bail!("{} has no pages", path.display());
    }
    let digits = pages.len().to_string().len().max(3);
    let (interpreter, settings, cache) = (
        InterpreterSettings::default(),
        RenderSettings::default(),
        RenderCache::new(),
    );
    let pixmap = PixmapSettings {
        x_scale: SCALE,
        y_scale: SCALE,
        bg_color: WHITE,
    };
    pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let number = index + 1;
            let bytes = render_page(page, &cache, &interpreter, &settings, &pixmap)
                .into_png()
                .with_context(|| format!("failed to encode page {number} of {}", path.display()))?;
            Ok(EncodedPage {
                name: format!("{}-{number:0digits$}.png", path.display()),
                bytes,
            })
        })
        .collect()
}
