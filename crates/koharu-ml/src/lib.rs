mod image;
mod nn;
mod torch;

pub mod baberu_ocr;
pub mod flux3_klein;
pub mod hayai_ocr;
pub mod koharu_layout_highres;
pub mod lama;
pub mod manga_ocr_ar;
pub mod qwen_image;
pub mod yuzumarker_font_detection;

pub use koharu_diffusion::Canceller;
pub use koharu_runtime::{Device, PinnedFile};

/// Every ML model owned by `koharu-ml`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Model {
    KoharuLayoutHighres,
    YuzuMarkerFontDetection,
    MangaOcrAr,
    BaberuOcr,
    HayaiOcr,
    Lama,
    Flux3Klein,
    QwenImage,
}

/// All pinned Hugging Face files required by `model`.
#[must_use]
pub fn files_for(model: Model) -> Vec<PinnedFile> {
    match model {
        Model::KoharuLayoutHighres => koharu_layout_highres::FILES,
        Model::YuzuMarkerFontDetection => yuzumarker_font_detection::FILES,
        Model::MangaOcrAr => manga_ocr_ar::FILES,
        Model::BaberuOcr => baberu_ocr::FILES,
        Model::HayaiOcr => hayai_ocr::FILES,
        Model::Lama => lama::FILES,
        Model::Flux3Klein => flux3_klein::FILES,
        Model::QwenImage => qwen_image::FILES,
    }
    .to_vec()
}

/// Returns `true` when `error` or any cause in its chain is a typed OOM
/// failure from `koharu-diffusion`.
#[must_use]
pub fn is_oom(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<koharu_diffusion::LoadError>(),
            Some(koharu_diffusion::LoadError::Status(
                koharu_diffusion::StatusError::OutOfMemory(_)
            ))
        ) || matches!(
            cause.downcast_ref::<koharu_diffusion::GenerateError>(),
            Some(koharu_diffusion::GenerateError::Status(
                koharu_diffusion::StatusError::OutOfMemory(_)
            ))
        ) || matches!(
            cause.downcast_ref::<koharu_diffusion::StatusError>(),
            Some(koharu_diffusion::StatusError::OutOfMemory(_))
        )
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_model_declares_nonempty_unique_pinned_files_with_sizes() {
        let models = [
            Model::KoharuLayoutHighres,
            Model::YuzuMarkerFontDetection,
            Model::MangaOcrAr,
            Model::BaberuOcr,
            Model::HayaiOcr,
            Model::Lama,
            Model::Flux3Klein,
            Model::QwenImage,
        ];
        for model in models {
            let files = files_for(model);
            assert!(!files.is_empty(), "{model:?} must declare files");
            let mut seen = HashSet::new();
            for file in files {
                assert!(!file.filename.is_empty(), "{model:?} has empty filename");
                assert!(file.bytes > 0, "{model:?} {} has zero bytes", file.filename);
                assert!(
                    seen.insert((file.repo, file.filename)),
                    "{model:?} has duplicate file {}",
                    file.filename
                );
            }
        }
    }
}
