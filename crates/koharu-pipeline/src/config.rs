use serde::{Deserialize, Serialize};

use crate::steps::{Flux2KleinConfig, KoharuLayoutRFDetrSeg2XLConfig, RoremMixedConfig};

/// `[pipeline]`: the model each step runs, and every model's settings, which
/// stay while another model is selected.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    pub detection: Detection,
    pub ocr: OcrModel,
    pub translation: TranslationConfig,
    pub inpainting: Inpainting,
    pub processor: ProcessorConfig,
}

impl koharu_config::Section for PipelineConfig {
    const NAME: &'static str = "pipeline";
}

impl PipelineConfig {
    /// The selected detection model, with its settings.
    pub fn detection(&self) -> DetectionModel {
        match self.detection {
            Detection::KoharuLayoutRFDetrSeg2XL => DetectionModel::KoharuLayoutRFDetrSeg2XL(
                self.processor.koharu_layout_rfdetr_seg_2xl.clone(),
            ),
        }
    }

    /// The selected inpainting model, with its settings.
    pub fn inpainting(&self) -> InpaintingModel {
        match self.inpainting {
            Inpainting::LaMa => InpaintingModel::LaMa {},
            Inpainting::AotInpainting => InpaintingModel::AotInpainting {},
            Inpainting::Flux2Klein => {
                InpaintingModel::Flux2Klein(self.processor.flux2_klein.clone())
            }
            Inpainting::RoremMixed => {
                InpaintingModel::RoremMixed(self.processor.rorem_mixed.clone())
            }
        }
    }
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
    #[serde(rename = "koharu-layout-rfdetr-seg-2xl")]
    pub koharu_layout_rfdetr_seg_2xl: KoharuLayoutRFDetrSeg2XLConfig,
    #[serde(rename = "flux2-klein")]
    pub flux2_klein: Flux2KleinConfig,
    #[serde(rename = "rorem-mixed")]
    pub rorem_mixed: RoremMixedConfig,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "model")]
pub enum Detection {
    #[default]
    #[serde(rename = "koharu-layout-rfdetr-seg-2xl")]
    KoharuLayoutRFDetrSeg2XL,
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
pub enum Inpainting {
    #[default]
    #[serde(rename = "lama")]
    LaMa,
    #[serde(rename = "aot-inpainting")]
    AotInpainting,
    #[serde(rename = "flux2-klein")]
    Flux2Klein,
    #[serde(rename = "rorem-mixed")]
    RoremMixed,
}

/// The selected detection model with its settings, as a stage runs it.
#[derive(Clone, Debug, PartialEq)]
pub enum DetectionModel {
    KoharuLayoutRFDetrSeg2XL(KoharuLayoutRFDetrSeg2XLConfig),
}

/// The selected inpainting model with its settings, as a stage runs it.
#[derive(Clone, Debug, PartialEq)]
pub enum InpaintingModel {
    LaMa {},
    AotInpainting {},
    Flux2Klein(Flux2KleinConfig),
    RoremMixed(RoremMixedConfig),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_slots_use_defaults() {
        let config = toml::from_str::<PipelineConfig>("").unwrap();

        assert_eq!(config, PipelineConfig::default());
        assert_eq!(config.detection, Detection::KoharuLayoutRFDetrSeg2XL);
        assert_eq!(config.ocr, OcrModel::PaddleOcrVl1_6);
        assert_eq!(config.inpainting(), InpaintingModel::LaMa {});
    }

    #[test]
    fn a_selected_model_takes_its_settings_from_processor() {
        let config: PipelineConfig = toml::from_str(
            r#"
                [ocr]
                model = "baberu-ocr"

                [inpainting]
                model = "rorem-mixed"

                [processor."koharu-layout-rfdetr-seg-2xl"]
                text_threshold = 0.25

                [processor."rorem-mixed"]
                prompt = "Remove the lettering."
                negative_prompt = "letters, words"

                [processor."flux2-klein"]
                prompt = "kept while RORem is selected"
            "#,
        )
        .unwrap();

        assert_eq!(config.ocr, OcrModel::BaberuOcr);
        assert!(matches!(
            config.detection(),
            DetectionModel::KoharuLayoutRFDetrSeg2XL(settings)
                if settings.text_threshold == Some(0.25)
        ));
        assert!(matches!(
            config.inpainting(),
            InpaintingModel::RoremMixed(settings)
                if settings.prompt == "Remove the lettering."
                    && settings.negative_prompt == "letters, words"
        ));
        assert_eq!(
            config.processor.flux2_klein.prompt,
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
            inpainting: Inpainting::Flux2Klein,
            ..PipelineConfig::default()
        };
        config.processor.flux2_klein.prompt = "Keep the line art.".to_owned();

        let document = toml::to_string(&config).unwrap();
        assert!(document.contains("[inpainting]\nmodel = \"flux2-klein\""));
        assert!(document.contains("[processor.flux2-klein]"));
        assert_eq!(toml::from_str::<PipelineConfig>(&document).unwrap(), config);
    }
}
