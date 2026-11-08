//! A font's preview: its family's name set in it, as a WebP.

use koharu_scene::{CharStyle, LanguageIdentifier, RichText, Text, TextStyle};
use vello::{Scene, kurbo::Affine};

use super::Renderer;
use crate::{Error, Result, fonts::FontPreview, text};

impl Renderer {
    /// A WebP of `family`'s name set in it, 96 pixels tall. A bundled family
    /// has its own; a system family that can't draw its name sets it in
    /// Arial.
    pub async fn font_preview(&self, family: &str) -> Result<Vec<u8>> {
        const HEIGHT: u32 = 96;
        let font = match self.0.fonts.preview(family).await.map_err(Error::Fonts)? {
            FontPreview::Webp(bytes) => return Ok(bytes),
            FontPreview::System(font) => *font,
        };
        let label = family.to_owned();
        let fonts = if font.renders(&label) {
            vec![font]
        } else {
            let arial = CharStyle {
                family: Some("Arial".to_owned()),
                ..CharStyle::default()
            };
            self.0
                .fonts
                .resolve(&arial, &[], &label, &LanguageIdentifier::UNKNOWN)
                .map_err(Error::Fonts)?
                .0
        };
        let renderer = self.clone();
        tokio::task::spawn_blocking(move || {
            let text = Text {
                content: RichText::from(label.as_str()),
                style: TextStyle {
                    chars: CharStyle {
                        size: Some(24.0),
                        ..CharStyle::default()
                    },
                    ..TextStyle::default()
                },
                ..Text::default()
            };
            let layout = text::layout(&text, |_, _| Ok(fonts.clone()))
                .map_err(Error::Fonts)?
                .ok_or_else(|| Error::invalid("a font family with no name has no preview"))?;
            let frame = layout.resolved.frame;
            let scale = f64::from(HEIGHT) / frame.height().max(1.0);
            let width = (frame.width() * scale).ceil().max(1.0) as u32;
            let mut scene = Scene::new();
            scene.append(
                &text::paint(&layout).scene,
                Some(Affine::scale(scale) * Affine::translate((-frame.x0, -frame.y0))),
            );
            let image = renderer
                .gpu()?
                .read(&scene, width, HEIGHT)
                .map_err(Error::Backend)?;
            Ok(
                webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                    .encode(85.0)
                    .to_vec(),
            )
        })
        .await
        .map_err(|error| Error::Backend(error.into()))?
    }
}
