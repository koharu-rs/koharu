//! Translation through local and hosted providers.

mod backend;
mod error;
mod glossary;
mod language;
mod local;
mod model;
mod prompt;
mod provider;
mod remote;

use std::sync::Arc;

use koharu_ml::Device;

use error::{Error, Result};
use local::LocalTranslator;

pub use backend::{TranslationContext, TranslationRequest};
pub use language::Language;
pub use model::{GenerationConfig, Model, ModelSelection, Quantization};
pub(crate) use model::{ModelGeneration, QuantizationDefinition, display_name};
pub use provider::{Provider, ProviderConfig, ProvidersConfig};

/// Upper bound on the attempts one batch is translated with: the first covers
/// the whole batch, the remaining ones re-request only what the model dropped.
const MAX_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct Translator {
    providers: koharu_config::Config<ProvidersConfig>,
    local: Arc<tokio::sync::Mutex<Option<LoadedLocal>>>,
    client: reqwest::Client,
    device: Device,
}

struct LoadedLocal {
    model: Option<String>,
    quantization: Option<String>,
    translator: Arc<LocalTranslator>,
}

impl LoadedLocal {
    fn matches(&self, selection: &ModelSelection) -> bool {
        self.model == selection.model && self.quantization == selection.quantization
    }
}

impl Translator {
    pub fn from_config(
        device: Device,
        providers: koharu_config::Config<ProvidersConfig>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            providers,
            local: Arc::new(tokio::sync::Mutex::new(None)),
            client: koharu_runtime::http_client()?,
            device,
        })
    }

    #[must_use]
    pub fn model(selection: &ModelSelection) -> &'static str {
        selection.provider.into()
    }

    #[must_use]
    pub fn supports_vision(selection: &ModelSelection, generation: &GenerationConfig) -> bool {
        generation.vision.unwrap_or(false)
            && (selection.provider != Provider::Local || local::supports_vision(selection))
    }

    /// Whether the provider answers overlapping requests independently.
    #[must_use]
    pub fn concurrent(&self, selection: &ModelSelection) -> bool {
        match selection.provider {
            Provider::Local | Provider::LmStudio => false,
            Provider::OpenAiCompatible => self
                .providers
                .read()
                .map(|providers| {
                    providers
                        .openai_compatible
                        .base_url
                        .as_ref()
                        .is_none_or(|url| !is_loopback(url))
                })
                .unwrap_or(false),
            _ => true,
        }
    }

    #[must_use]
    pub fn loaded(&self, selection: &ModelSelection) -> bool {
        if selection.provider != Provider::Local {
            return true;
        }
        self.local
            .try_lock()
            .map(|loaded| {
                loaded
                    .as_ref()
                    .is_some_and(|loaded| loaded.matches(selection))
            })
            .unwrap_or(true)
    }

    pub fn unload(&self) -> bool {
        self.local
            .try_lock()
            .map(|mut loaded| loaded.take().is_some())
            .unwrap_or(false)
    }

    #[tracing::instrument(skip_all)]
    pub async fn load_model(&self, selection: &ModelSelection) -> anyhow::Result<()> {
        if selection.provider == Provider::Local {
            self.local(selection).await?;
        }
        Ok(())
    }

    #[tracing::instrument(
        target = "koharu_metrics",
        name = "model_run",
        skip_all,
        fields(
            stage = "translation",
            provider = %selection.provider,
            model = selection.model.as_deref().unwrap_or("provider_default"),
            target_language = request.target_language.tag(),
            outcome = tracing::field::Empty,
        ),
    )]
    pub async fn translate(
        &self,
        selection: &ModelSelection,
        generation: GenerationConfig,
        mut request: TranslationRequest,
    ) -> anyhow::Result<(&'static str, Vec<String>)> {
        let _metric = tracing::info_span!(
            target: "koharu_metrics",
            "translation_request",
            provider = %selection.provider,
            model = selection.model.as_deref().unwrap_or("provider_default"),
            target_language = request.target_language.tag(),
        );
        let provider = selection.provider;
        let provider_id: &'static str = provider.into();
        if request.segments.is_empty() {
            tracing::Span::current().record("outcome", "skipped");
            return Ok((provider_id, request.segments));
        }

        let generation = generation.for_model(selection);

        if Self::supports_vision(selection, &generation) {
            request.prepare_image()?;
        } else {
            request.remove_image();
        }

        request.glossary = koharu_scene::relevant_glossary(&request.segments, &request.glossary);
        let protected = if matches!(
            provider,
            Provider::DeepL | Provider::GoogleCloudTranslation | Provider::Caiyun
        ) && !request.glossary.is_empty()
        {
            Some(glossary::ProtectedSegments::prepare(&mut request))
        } else {
            None
        };
        let expected = request.segments.len();
        let mut outcome = self
            .translate_once(selection, &generation, &request)
            .await?;

        // Models truncate or corrupt the tail of a long batch. Re-request only
        // the segments the provider never returned before failing the batch.
        let mut attempt = 1;
        let mut failure: Option<anyhow::Error> = None;
        while !outcome.missing.is_empty() && attempt < MAX_ATTEMPTS {
            attempt += 1;
            failure = failure.or(outcome.error.take());
            let missing = std::mem::take(&mut outcome.missing);
            tracing::debug!(
                attempt,
                segments = missing.len(),
                "re-requesting segments the provider did not return",
            );
            let repair = repair_request(&request, &missing);
            let repaired = self
                .translate_once(selection, &repair_generation(&generation), &repair)
                .await?;
            merge_missing(&mut outcome, &missing, repaired);
        }
        if !outcome.missing.is_empty() {
            let error = Error::SegmentCount {
                provider: provider_id,
                expected,
                actual: expected - outcome.missing.len(),
            };
            return Err(match failure.or(outcome.error) {
                Some(failure) => failure.context(error.to_string()),
                None => error.into(),
            });
        }
        let translated = outcome.translations;
        if translated.len() != expected {
            return Err(Error::SegmentCount {
                provider: provider_id,
                expected,
                actual: translated.len(),
            }
            .into());
        }
        tracing::Span::current().record("outcome", "completed");
        let translated = match protected {
            Some(protected) => protected.restore(&translated)?,
            None => translated,
        };
        Ok((provider_id, translated))
    }

    async fn translate_once(
        &self,
        selection: &ModelSelection,
        generation: &GenerationConfig,
        request: &TranslationRequest,
    ) -> Result<prompt::TranslationOutcome> {
        if selection.provider != Provider::Local {
            let providers = self.providers.read()?.clone();
            return remote::translate(&self.client, &providers, selection, generation, request)
                .await;
        }
        self.local(selection)
            .await?
            .translate(request.clone(), *generation)
            .await
    }

    #[tracing::instrument(skip_all)]
    pub async fn models() -> anyhow::Result<Vec<Model>> {
        let providers = ProvidersConfig::load()?;
        let providers = providers.read()?.clone();
        let client = koharu_runtime::http_client()?;
        let mut models = local::models();
        models.extend(remote::models(&client, &providers).await);
        Ok(models)
    }

    async fn local(&self, selection: &ModelSelection) -> Result<Arc<LocalTranslator>> {
        let mut loaded = self.local.lock().await;
        if loaded
            .as_ref()
            .is_none_or(|loaded| !loaded.matches(selection))
        {
            *loaded = Some(LoadedLocal {
                model: selection.model.clone(),
                quantization: selection.quantization.clone(),
                translator: Arc::new(LocalTranslator::load(self.device.clone(), selection).await?),
            });
        }
        Ok(Arc::clone(
            &loaded
                .as_ref()
                .expect("local translator was loaded")
                .translator,
        ))
    }
}

/// Narrows a batch to the segments a provider did not return, keeping context,
/// glossary, and image intact.
fn repair_request(request: &TranslationRequest, missing: &[usize]) -> TranslationRequest {
    let mut repair = request.clone();
    repair.segments = missing
        .iter()
        .filter_map(|&index| request.segments.get(index).cloned())
        .collect();
    repair
}

/// Repair attempts trade variety for reliability: a dropped or corrupted tail
/// is usually sampling noise, so a retry runs deterministically.
fn repair_generation(generation: &GenerationConfig) -> GenerationConfig {
    GenerationConfig {
        temperature: Some(0.0),
        top_p: Some(1.0),
        reasoning: generation.reasoning.map(|_| false),
        ..*generation
    }
}

/// Folds a repaired response back into the batch. Repair positions map onto the
/// original missing indices rather than the batch.
fn merge_missing(
    outcome: &mut prompt::TranslationOutcome,
    missing: &[usize],
    repaired: prompt::TranslationOutcome,
) {
    let prompt::TranslationOutcome {
        translations,
        missing: dropped,
        error,
    } = repaired;
    let mut remaining = Vec::new();
    for (position, &index) in missing.iter().enumerate() {
        if dropped.contains(&position) {
            remaining.push(index);
        } else if let Some(text) = translations.get(position) {
            outcome.translations[index] = text.clone();
        }
    }
    outcome.missing = remaining;
    outcome.error = error;
}

// Hosted endpoints scale with parallel requests; local servers queue them.
fn is_loopback(url: &url::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host == "::1"
            || host
                .parse::<std::net::Ipv4Addr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_selection(model: &str) -> ModelSelection {
        ModelSelection {
            provider: Provider::Local,
            model: Some(model.to_owned()),
            quantization: None,
            vision: true,
            reasoning: true,
        }
    }

    #[test]
    fn local_vision_requires_capability_and_generation_setting() {
        assert!(Translator::supports_vision(
            &local_selection("gemma4-e2b-it"),
            &GenerationConfig {
                vision: Some(true),
                ..GenerationConfig::default()
            }
        ));
        assert!(!Translator::supports_vision(
            &local_selection("gemma4-e2b-it"),
            &GenerationConfig {
                vision: Some(false),
                ..GenerationConfig::default()
            }
        ));
        assert!(!Translator::supports_vision(
            &local_selection("lfm2.5-1.2b-instruct"),
            &GenerationConfig {
                vision: Some(true),
                ..GenerationConfig::default()
            }
        ));
    }

    fn selection(provider: Provider) -> ModelSelection {
        ModelSelection {
            provider,
            model: None,
            quantization: None,
            vision: true,
            reasoning: true,
        }
    }

    fn translator(providers: ProvidersConfig) -> Translator {
        Translator::from_config(Device::cpu(), koharu_config::Config::memory(providers)).unwrap()
    }

    #[test]
    fn hosted_providers_allow_overlapping_requests() {
        let translator = translator(ProvidersConfig::default());
        assert!(translator.concurrent(&selection(Provider::OpenAi)));
        assert!(translator.concurrent(&selection(Provider::OpenRouter)));
        assert!(!translator.concurrent(&selection(Provider::Local)));
        assert!(!translator.concurrent(&selection(Provider::LmStudio)));
        // The default OpenAI-compatible endpoint is a local server.
        assert!(!translator.concurrent(&selection(Provider::OpenAiCompatible)));
    }

    #[test]
    fn remote_openai_compatible_endpoints_allow_overlapping_requests() {
        let mut providers = ProvidersConfig::default();
        providers.openai_compatible.base_url =
            Some(url::Url::parse("https://api.example.com/v1").unwrap());
        assert!(translator(providers).concurrent(&selection(Provider::OpenAiCompatible)));
    }

    fn outcome(translations: &[&str], missing: &[usize]) -> prompt::TranslationOutcome {
        prompt::TranslationOutcome {
            translations: translations.iter().map(|text| (*text).to_owned()).collect(),
            missing: missing.to_vec(),
            error: None,
        }
    }

    #[test]
    fn repair_request_narrows_segments_and_keeps_the_rest() {
        let request = TranslationRequest::new(["one", "two", "three"], Language::English)
            .with_instructions("keep honorifics");
        let repair = repair_request(&request, &[0, 2]);

        assert_eq!(repair.segments, vec!["one", "three"]);
        assert_eq!(repair.instructions.as_deref(), Some("keep honorifics"));
        assert_eq!(repair.target_language, request.target_language);
    }

    #[test]
    fn repair_generation_is_deterministic() {
        let repaired = repair_generation(&GenerationConfig {
            temperature: Some(1.3),
            top_p: Some(0.9),
            reasoning: Some(true),
            ..GenerationConfig::default()
        });

        assert_eq!(repaired.temperature, Some(0.0));
        assert_eq!(repaired.top_p, Some(1.0));
        assert_eq!(repaired.reasoning, Some(false));
    }

    #[test]
    fn merge_missing_folds_repairs_back_by_original_index() {
        let mut batch = outcome(&["a", "b", "c", "d"], &[1, 3]);
        merge_missing(&mut batch, &[1, 3], outcome(&["B", "d"], &[]));

        assert_eq!(batch.translations, vec!["a", "B", "c", "d"]);
        assert!(batch.missing.is_empty());

        let mut partial = outcome(&["a", "b", "c"], &[0, 1, 2]);
        merge_missing(&mut partial, &[0, 1, 2], outcome(&["x", "b", "c"], &[2]));

        assert_eq!(partial.translations, vec!["x", "b", "c"]);
        assert_eq!(partial.missing, vec![2]);
    }
}
