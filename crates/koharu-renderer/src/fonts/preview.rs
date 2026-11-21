//! A family's preview, which the font list shows: its name set in it, as a
//! WebP.

use anyhow::{Context as _, Result};
use koharu_scene::{CharStyle, LanguageIdentifier, RichText, Text, TextStyle};
use kurbo::Affine;

use super::{catalog, library};
use crate::{Gpu, text};

/// How tall a preview is, in pixels.
const HEIGHT: u16 = 96;

/// A WebP of `family`'s name set in it. A bundled family has its own; an
/// installed family that can't draw its name sets it in Arial.
pub async fn preview(family: &str) -> Result<Vec<u8>> {
    let family = family.to_owned();
    tokio::task::spawn_blocking(move || {
        if let Some(bundled) = catalog::family(&family)? {
            return catalog::preview(bundled);
        }
        let font = library::with(|library| library.family(&family))?;
        let fonts = match font.renders(&family) {
            true => vec![font],
            false => {
                let arial = CharStyle {
                    family: Some("Arial".to_owned()),
                    ..CharStyle::default()
                };
                super::chain(&arial, &[], &family, &LanguageIdentifier::UNKNOWN)?
            }
        };
        let name = Text {
            content: RichText::from(family.as_str()),
            style: TextStyle {
                chars: CharStyle {
                    size: Some(24.0),
                    ..CharStyle::default()
                },
                ..TextStyle::default()
            },
            ..Text::default()
        };
        let layout = text::layout(&name, |_, _| Ok(fonts.clone()))?
            .context("a font family with no name has no preview")?;
        let frame = layout.resolved.frame;
        let scale = f64::from(HEIGHT) / frame.height().max(1.0);
        let width = (frame.width() * scale).ceil().max(1.0) as u16;
        let gpu = Gpu::get()?;
        let mut drawing = gpu.drawing(width, HEIGHT);
        text::draw(
            &mut drawing,
            &layout,
            Affine::scale(scale) * Affine::translate((-frame.x0, -frame.y0)),
            false,
        )?;
        let image = drawing.read()?;
        Ok(
            webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                .encode(85.0)
                .to_vec(),
        )
    })
    .await?
}
