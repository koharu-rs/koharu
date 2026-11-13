//! `[pipeline]`: each role's model and that role's options. The app passes
//! the section to `Pipeline::new` and `set_config`, and a request may bring
//! its own. An invalid section fails to parse.

use icu_locale_core::{LanguageIdentifier, langid};
use indoc::indoc;
use koharu_llm::rig::completion::Effort;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use strum::{Display, VariantArray};

/// Translation's system prompt unless the user writes their own:
/// Index-Translate's instTrans template in English, as bilibili/Index-Translate's
/// `docs/prompts.md` gives it (93ac63774d1c6939aec05e6f0f191bf90f225365,
/// Apache-2.0), measured against its Chinese template on Manga109 pages.
pub const DEFAULT_PROMPT: &str = indoc! {"
    Please translate the manga text in each message into {target} and strictly follow all constraints. Each message is a page's texts as a JSON object, numbered in reading order.

    [Constraints]
    1. [Hard Constraint] Preserve the JSON format in the source text unchanged

    Output the translation in the same JSON format, keeping the keys unchanged and each value its translation. Output only the JSON without any explanation."};

/// What the system prompt has replaced by the target's name.
pub const TARGET: &str = "{target}";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    pub detector: DetectorConfig,
    pub recognizer: RecognizerConfig,
    pub inpainter: InpainterConfig,
    pub translator: TranslatorConfig,
}

impl koharu_config::Section for PipelineConfig {
    const NAME: &'static str = "pipeline";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectorConfig {
    pub model: Detector,
    /// Each class's lowest confidence a detection keeps, from 0 to 1, or the
    /// model's own.
    #[serde(deserialize_with = "threshold")]
    pub text: Option<f32>,
    #[serde(deserialize_with = "threshold")]
    pub onomatopoeia: Option<f32>,
    #[serde(deserialize_with = "threshold")]
    pub bubble: Option<f32>,
    #[serde(deserialize_with = "threshold")]
    pub panel: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecognizerConfig {
    pub model: Recognizer,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InpainterConfig {
    pub model: Inpainter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslatorConfig {
    /// A `vendor/model` reference, as koharu-llm connects one.
    pub model: String,
    /// A language CLDR names in English.
    #[serde(deserialize_with = "target")]
    pub target: LanguageIdentifier,
    /// The system prompt, with `{target}` where the target's name goes.
    #[serde(deserialize_with = "prompt")]
    pub prompt: String,
    /// Whether each page's turn shows the model the page itself beside its
    /// texts, as an image goes with a chat message to OpenAI's and Gemini's
    /// models; only for a model that reads images.
    pub vision: bool,
    /// How much the model reasons before it answers, by rig's portable levels
    /// after OpenAI's `reasoning_effort`; the model's own default when unset.
    pub reasoning: Option<Effort>,
}

impl Default for TranslatorConfig {
    fn default() -> Self {
        Self {
            // On 16 Blue Archive pages 9B read names and meaning that 2B
            // missed, such as Shale for シャーレ where 2B wrote "tray", at
            // about twice the decoding time.
            model: "koharu/index-translate-9b".to_owned(),
            target: langid!("en"),
            prompt: DEFAULT_PROMPT.to_owned(),
            vision: false,
            reasoning: None,
        }
    }
}

/// The detectors, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
pub enum Detector {
    #[default]
    #[strum(to_string = "Koharu Layout HighRes")]
    KoharuLayoutHighres,
}

#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
pub enum Recognizer {
    #[default]
    #[strum(to_string = "manga-ocr-ar")]
    MangaOcrAr,
    #[strum(to_string = "Baberu OCR")]
    BaberuOcr,
    #[strum(to_string = "Hayai OCR")]
    HayaiOcr,
}

impl Recognizer {
    /// The language it reads, as a BCP 47 tag: `und` for one that reads
    /// several and doesn't say which.
    pub(crate) fn language(self) -> &'static str {
        match self {
            Self::MangaOcrAr | Self::BaberuOcr => "ja",
            Self::HayaiOcr => "und",
        }
    }
}

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
    #[strum(to_string = "FLUX.3 Image Klein")]
    Flux3Klein,
    #[strum(to_string = "MOSS-Image")]
    MossImage,
}

impl Inpainter {
    /// Whether it takes a prompt.
    pub(crate) fn prompted(self) -> bool {
        matches!(self, Self::Flux3Klein | Self::MossImage)
    }

    /// The longest side of a window it redraws in one pass: past 800
    /// pixels, LaMa crops around each masked region and runs once for each,
    /// as IOPaint does.
    pub(crate) fn side(self) -> u32 {
        match self {
            Self::Lama => 800,
            Self::RoremMixed | Self::Flux3Klein | Self::MossImage => u32::MAX,
        }
    }

    /// Whether it runs on stable-diffusion.cpp, which fits itself to the
    /// device's free memory.
    pub(crate) fn ggml(self) -> bool {
        matches!(self, Self::Flux3Klein | Self::MossImage)
    }
}

fn threshold<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f32>, D::Error> {
    let threshold = Option::<f32>::deserialize(deserializer)?;
    match threshold {
        Some(value) if !(0.0..=1.0).contains(&value) => Err(D::Error::custom(format!(
            "a detection threshold of {value} isn't between 0 and 1"
        ))),
        _ => Ok(threshold),
    }
}

fn target<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LanguageIdentifier, D::Error> {
    let target = LanguageIdentifier::deserialize(deserializer)?;
    crate::steps::translate::name(&target).map_err(D::Error::custom)?;
    Ok(target)
}

fn prompt<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let prompt = String::deserialize(deserializer)?;
    if !prompt.contains(TARGET) {
        return Err(D::Error::custom(format!(
            "the translation prompt has no {TARGET}"
        )));
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_section_fails_to_parse() {
        let parse = |section: serde_json::Value| serde_json::from_value::<PipelineConfig>(section);
        assert!(parse(serde_json::json!({"detector": {"text": 0.3}})).is_ok());
        assert!(parse(serde_json::json!({"detector": {"text": 1.5}})).is_err());
        assert!(parse(serde_json::json!({"translator": {"target": "xx"}})).is_err());
        assert!(parse(serde_json::json!({"translator": {"prompt": "Translate."}})).is_err());
    }
}
