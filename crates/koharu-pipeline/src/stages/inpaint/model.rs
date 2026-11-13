use image::{DynamicImage, GrayImage, RgbImage, imageops};
use koharu_ml::{
    flux3_klein::{self, Flux3Klein},
    lama::{self, Lama},
    moss_image::{self, MossImage},
    qwen_image::{self, QwenImage},
    rorem_mixed::{self, RoremMixed},
};
use koharu_runtime::Device;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

/// The inpainters, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
pub enum Inpainter {
    #[default]
    #[strum(to_string = "LaMa")]
    Lama,
    #[strum(to_string = "RORem")]
    RoremMixed,
    #[strum(to_string = "FLUX.3 Klein")]
    Flux3Klein,
    #[strum(to_string = "MOSS-Image")]
    MossImage,
    #[strum(to_string = "Qwen-Image 2.1")]
    QwenImage,
}

/// An inpainter, loaded.
pub(super) type Model = Box<dyn Inpaint + Send>;

pub(super) trait Inpaint {
    /// `image` redrawn where `mask` is set, at its own size, after `prompt`,
    /// which may be blank.
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage>;

    /// The longest side of a window it redraws in one pass.
    fn pass(&self) -> u32 {
        u32::MAX
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

impl Inpaint for RoremMixed {
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let options = rorem_mixed::Options::default();
        let redrawn = RoremMixed::inference(
            self,
            prompt,
            &DynamicImage::ImageRgb8(image.clone()),
            mask,
            &options,
        )?;
        Ok(fitted(redrawn, image))
    }
}

impl Inpaint for Flux3Klein {
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let options = flux3_klein::InpaintOptions::default();
        let redrawn = Flux3Klein::inpaint(
            self,
            prompt,
            &DynamicImage::ImageRgb8(image.clone()),
            None,
            &DynamicImage::ImageLuma8(mask.clone()),
            &options,
        )?;
        Ok(fitted(redrawn, image))
    }
}

impl Inpaint for MossImage {
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let options = moss_image::InpaintOptions::default();
        let redrawn = MossImage::inpaint(
            self,
            prompt,
            &DynamicImage::ImageRgb8(image.clone()),
            &DynamicImage::ImageLuma8(mask.clone()),
            &options,
        )?;
        Ok(fitted(redrawn, image))
    }
}

impl Inpaint for QwenImage {
    fn inference(
        &self,
        image: &RgbImage,
        mask: &GrayImage,
        prompt: &str,
    ) -> anyhow::Result<RgbImage> {
        let options = qwen_image::InpaintOptions::default();
        let redrawn = QwenImage::inpaint(
            self,
            prompt,
            &DynamicImage::ImageRgb8(image.clone()),
            &DynamicImage::ImageLuma8(mask.clone()),
            &options,
        )?;
        Ok(fitted(redrawn, image))
    }
}

/// `redrawn` back at `image`'s size: the diffusion models draw on their own
/// grid.
fn fitted(redrawn: RgbImage, image: &RgbImage) -> RgbImage {
    let (width, height) = image.dimensions();
    if redrawn.dimensions() == (width, height) {
        redrawn
    } else {
        imageops::resize(&redrawn, width, height, imageops::FilterType::Lanczos3)
    }
}

pub(super) async fn load(model: Inpainter, device: &'static Device) -> anyhow::Result<Model> {
    Ok(match model {
        Inpainter::Lama => Box::new(Lama::load(device).await?),
        Inpainter::RoremMixed => Box::new(RoremMixed::load(device).await?),
        Inpainter::Flux3Klein => Box::new(Flux3Klein::load(device).await?),
        Inpainter::MossImage => Box::new(MossImage::load(device).await?),
        Inpainter::QwenImage => Box::new(QwenImage::load(device).await?),
    })
}
