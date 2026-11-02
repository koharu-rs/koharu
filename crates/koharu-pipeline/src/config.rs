use serde::{Deserialize, Serialize};

use crate::steps::{Flux3KleinConfig, KoharuLayoutHighresConfig, RoremMixedConfig};

/// `[pipeline]`: the model each step runs, and every model's settings, which
/// stay while another model is selected.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    pub detection: DetectionModel,
    pub ocr: OcrModel,
    pub translation: TranslationConfig,
    pub inpainting: InpaintingModel,
    pub processor: ProcessorConfig,
}

impl koharu_config::Section for PipelineConfig {
    const NAME: &'static str = "pipeline";
}

/// `[pipeline.translation]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationConfig {
    pub model: koharu_llm::Selection,
    /// A BCP 47 tag.
    pub target_language: String,
    pub instructions: Option<String>,
    /// Sends each page's scan with its text.
    pub vision: bool,
    pub sampling: koharu_llm::Sampling,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            model: koharu_llm::Selection::default(),
            target_language: "en".to_owned(),
            instructions: None,
            vision: true,
            sampling: koharu_llm::Sampling::default(),
        }
    }
}

/// Each model's settings, under its name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessorConfig {
    #[serde(rename = "koharu-layout-highres")]
    pub koharu_layout_highres: KoharuLayoutHighresConfig,
    #[serde(rename = "flux3-klein")]
    pub flux3_klein: Flux3KleinConfig,
    #[serde(rename = "rorem-mixed")]
    pub rorem_mixed: RoremMixedConfig,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "model")]
pub enum DetectionModel {
    #[default]
    #[serde(rename = "koharu-layout-highres")]
    KoharuLayoutHighres,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "model")]
pub enum OcrModel {
    #[default]
    #[serde(rename = "paddleocr-vl-1.6")]
    PaddleOcrVl1_6,
    #[serde(rename = "manga-ocr")]
    MangaOcr,
    #[serde(rename = "baberu-ocr")]
    BaberuOcr,
    #[serde(rename = "hayai-ocr")]
    HayaiOcr,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "model")]
pub enum InpaintingModel {
    #[default]
    #[serde(rename = "lama")]
    Lama,
    #[serde(rename = "aot-inpainting")]
    AotInpainting,
    #[serde(rename = "flux3-klein")]
    Flux3Klein,
    #[serde(rename = "rorem-mixed")]
    RoremMixed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_slots_use_defaults() {
        let config = toml::from_str::<PipelineConfig>("").unwrap();

        assert_eq!(config, PipelineConfig::default());
        assert_eq!(config.detection, DetectionModel::KoharuLayoutHighres);
        assert_eq!(config.ocr, OcrModel::PaddleOcrVl1_6);
        assert_eq!(config.inpainting, InpaintingModel::Lama);
    }

    #[test]
    fn every_model_keeps_its_settings_in_processor() {
        let config: PipelineConfig = toml::from_str(
            r#"
                [ocr]
                model = "baberu-ocr"

                [inpainting]
                model = "rorem-mixed"

                [processor."koharu-layout-highres"]
                text_threshold = 0.25

                [processor."rorem-mixed"]
                prompt = "Remove the lettering."
                negative_prompt = "letters, words"

                [processor."flux3-klein"]
                prompt = "kept while RORem is selected"
            "#,
        )
        .unwrap();

        assert_eq!(config.ocr, OcrModel::BaberuOcr);
        assert_eq!(config.inpainting, InpaintingModel::RoremMixed);
        assert_eq!(
            config.processor.koharu_layout_highres.text_threshold,
            Some(0.25)
        );
        assert_eq!(config.processor.rorem_mixed.prompt, "Remove the lettering.");
        assert_eq!(
            config.processor.rorem_mixed.negative_prompt,
            "letters, words"
        );
        assert_eq!(
            config.processor.flux3_klein.prompt,
            "kept while RORem is selected"
        );
    }

    #[test]
    fn an_unknown_model_fails() {
        assert!(toml::from_str::<PipelineConfig>("[ocr]\nmodel = \"tesseract\"").is_err());
    }

    #[test]
    fn round_trips_through_toml() {
        let mut config = PipelineConfig {
            inpainting: InpaintingModel::Flux3Klein,
            ..PipelineConfig::default()
        };
        config.processor.flux3_klein.prompt = "Keep the line art.".to_owned();

        let document = toml::to_string(&config).unwrap();
        assert!(document.contains("[inpainting]\nmodel = \"flux3-klein\""));
        assert!(document.contains("[processor.flux3-klein]"));
        assert_eq!(toml::from_str::<PipelineConfig>(&document).unwrap(), config);
    }
}
