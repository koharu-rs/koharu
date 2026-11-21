//! The inpainters, and how each redraws a window: LaMa under the mask, the
//! edit models all of it.

use anyhow::Context as _;
use image::{DynamicImage, GrayImage, RgbImage, imageops};
use koharu_ml::{
    Canceller,
    flux3_klein::{self, Flux3Klein},
    lama::{self, Lama},
    qwen_image::{self, QwenImage},
};
use koharu_runtime::Device;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

use super::tone;

/// The inpainters, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[derive(specta::Type)]
pub enum Inpainter {
    #[default]
    #[strum(to_string = "LaMa")]
    Lama,
    #[strum(to_string = "FLUX.3 Klein")]
    Flux3Klein,
    #[strum(to_string = "Qwen-Image 2.1 Turbo")]
    QwenImage,
}

/// An inpainter, loaded.
pub(super) type Model = Box<dyn Inpaint + Send>;

pub(super) trait Inpaint {
    /// `image` redrawn where `mask` is set, at its own size, after `prompt`,
    /// which may be blank. An edit model redraws all of it.
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage>;

    /// Whether it's a generative edit model, which redraws the whole image
    /// rather than only the mask, so all of what it drew is worth keeping.
    fn generative(&self) -> bool {
        false
    }

    /// The longest side of a window it redraws in one pass.
    fn pass(&self) -> u32 {
        u32::MAX
    }

    /// What stops a redraw under way, for a model that can be stopped.
    fn canceller(&self) -> Option<Canceller> {
        None
    }
}

impl Inpaint for Lama {
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        // It takes no prompt.
        _: &str,
    ) -> anyhow::Result<RgbImage> {
        Lama::inference(self, &DynamicImage::ImageRgb8(image.clone()), mask)
    }

    /// Past it LaMa crops around each masked region and runs once for each.
    fn pass(&self) -> u32 {
        lama::WHOLE
    }
}

impl Inpaint for Flux3Klein {
    /// An edit of the window, its only reference, as Black Forest Labs and
    /// stable-diffusion.cpp publish the model's editing; neither inpaints it.
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let edited = Flux3Klein::generate(
            self,
            &[DynamicImage::ImageRgb8(image.clone())],
            prompt,
            &flux3_klein::GenerateOptions::default(),
        )?
        .into_iter()
        .next()
        .context("FLUX.3 Image Klein returned no image")?;
        Ok(matched(edited, image, mask))
    }

    fn generative(&self) -> bool {
        true
    }

    fn canceller(&self) -> Option<Canceller> {
        Some(Flux3Klein::canceller(self))
    }
}

impl Inpaint for QwenImage {
    /// An edit of the window, its one condition image, on Turbo's 8 sigmas,
    /// as Qwen publishes the model's editing; it has no inpainting pipeline.
    /// It edits the window at its own size, at most a megapixel, as FLUX.3
    /// does and as ComfyUI's Qwen-Image 2.1 edit template keeps a reference.
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let side = (f64::from(image.width()) * f64::from(image.height())).sqrt();
        let options = qwen_image::Options {
            output_resolution: side.min(1024.0).round() as u32,
            ..qwen_image::Options::default()
        };
        let edited = QwenImage::generate(
            self,
            prompt,
            &[DynamicImage::ImageRgb8(image.clone())],
            &options,
        )?
        .into_iter()
        .next()
        .context("Qwen-Image 2.1 Turbo returned no image")?;
        Ok(matched(
            DynamicImage::ImageRgba8(edited).into_rgb8(),
            image,
            mask,
        ))
    }

    fn generative(&self) -> bool {
        true
    }

    fn canceller(&self) -> Option<Canceller> {
        Some(QwenImage::canceller(self))
    }
}

/// An edit of `window` back at its size, since the edit models draw on their
/// own grid, and with its colours matched to the window's away from `mask`.
fn matched(edited: RgbImage, window: &RgbImage, mask: &GrayImage) -> RgbImage {
    let (width, height) = window.dimensions();
    let edited = if edited.dimensions() == (width, height) {
        edited
    } else {
        imageops::resize(&edited, width, height, imageops::FilterType::Lanczos3)
    };
    tone::matched(window, &edited, mask)
}

pub(super) async fn load(model: Inpainter, device: &'static Device) -> anyhow::Result<Model> {
    Ok(match model {
        Inpainter::Lama => Box::new(Lama::load(device).await?),
        Inpainter::Flux3Klein => Box::new(Flux3Klein::load(device).await?),
        Inpainter::QwenImage => Box::new(QwenImage::load(device).await?),
    })
}
