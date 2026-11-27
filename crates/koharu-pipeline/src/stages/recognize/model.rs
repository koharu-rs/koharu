use image::DynamicImage;
use koharu_ml::{baberu_ocr::BaberuOcr, hayai_ocr::HayaiOcr, manga_ocr_ar::MangaOcrAr};
use koharu_runtime::{Device, PinnedFile};
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

/// The recognizers, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[derive(specta::Type)]
pub enum Recognizer {
    #[strum(to_string = "Manga OCR AR")]
    MangaOcrAr,
    #[strum(to_string = "Baberu OCR")]
    BaberuOcr,
    #[default]
    #[strum(to_string = "Hayai OCR")]
    HayaiOcr,
}

impl Recognizer {
    pub const ALL: &'static [Self] = Self::VARIANTS;

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::MangaOcrAr => "manga-ocr-ar",
            Self::BaberuOcr => "baberu-ocr",
            Self::HayaiOcr => "hayai-ocr",
        }
    }

    #[must_use]
    pub fn files(self) -> Vec<PinnedFile> {
        let model = match self {
            Self::MangaOcrAr => koharu_ml::Model::MangaOcrAr,
            Self::BaberuOcr => koharu_ml::Model::BaberuOcr,
            Self::HayaiOcr => koharu_ml::Model::HayaiOcr,
        };
        koharu_ml::files_for(model)
    }
}

/// A recognizer, loaded.
pub(super) type Model = Box<dyn Recognize + Send>;

pub(super) trait Recognize {
    /// Each image's text, in order.
    fn inference(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>>;

    /// The language it reads, as a BCP 47 tag: `und` for one that reads
    /// several and doesn't say which.
    fn language(&self) -> &'static str;
}

impl Recognize for MangaOcrAr {
    fn inference(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>> {
        MangaOcrAr::inference(self, images)
    }

    fn language(&self) -> &'static str {
        "ja"
    }
}

impl Recognize for BaberuOcr {
    fn inference(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>> {
        BaberuOcr::inference(self, images)
    }

    fn language(&self) -> &'static str {
        "ja"
    }
}

impl Recognize for HayaiOcr {
    fn inference(&self, images: &[DynamicImage]) -> anyhow::Result<Vec<String>> {
        HayaiOcr::inference(self, images)
    }

    fn language(&self) -> &'static str {
        "und"
    }
}

pub(super) async fn load(model: Recognizer, device: &'static Device) -> anyhow::Result<Model> {
    Ok(match model {
        Recognizer::MangaOcrAr => Box::new(MangaOcrAr::load(device).await?),
        Recognizer::BaberuOcr => Box::new(BaberuOcr::load(device).await?),
        Recognizer::HayaiOcr => Box::new(HayaiOcr::load(device).await?),
    })
}
