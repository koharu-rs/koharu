//! Translation through local and hosted providers.

mod backend;
mod error;
mod language;
mod local;
mod model;
mod prompt;
mod provider;
mod remote;
mod repair;

use std::{sync::Arc, time::Duration};

use koharu_ml::Device;

use error::{Error, Result};
use local::LocalTranslator;

pub use backend::{TranslationContext, TranslationRequest};
pub use language::Language;
pub use model::{GenerationConfig, Model, ModelSelection, Quantization};
pub(crate) use model::{ModelGeneration, QuantizationDefinition, display_name};
pub use provider::{Provider, ProviderConfig, ProvidersConfig};

/// How many times misaligned segments are asked for again.
const REPAIR_ATTEMPTS: u32 = 2;

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

        let mut translated = self.dispatch(selection, generation, &request).await?;
        if matches!(
            provider,
            Provider::DeepL | Provider::GoogleCloudTranslation | Provider::Caiyun
        ) {
            // Machine translation returns one result per segment; it cannot
            // misalign them.
            tracing::Span::current().record("outcome", "completed");
            return Ok((provider_id, translated));
        }

        let suspects = repair::suspects(&request.segments, &translated);
        if !suspects.is_empty() {
            tracing::warn!(
                provider = provider_id,
                segments = suspects.len(),
                "translation looks cut off or shifted; asking again for each of those segments"
            );
        }
        // Each suspect goes alone: a model that split a sentence across two ids
        // does it again when handed the same batch, but with one segment there
        // is no neighbour to push the rest into. The segments that came back
        // whole, and those already repaired, travel as context to keep the
        // scene.
        let mut repaired = vec![false; translated.len()];
        let mut retry_requests = 0_u32;
        for &index in &suspects {
            let mut retry = request.clone();
            retry.segments = vec![request.segments[index].clone()];
            retry.context.extend(
                (0..request.segments.len())
                    .filter(|&other| {
                        other != index && (repaired[other] || !suspects.contains(&other))
                    })
                    .map(|other| TranslationContext {
                        source: request.segments[other].clone(),
                        translation: translated[other].clone(),
                    }),
            );
            for _ in 0..REPAIR_ATTEMPTS {
                retry_requests += 1;
                match self.dispatch(selection, generation, &retry).await {
                    Ok(mut retried) => {
                        let text = retried.remove(0);
                        if repair::acceptable(&request.segments[index], &text) {
                            translated[index] = text;
                            repaired[index] = true;
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(provider = provider_id, "retry failed: {error:#}");
                        break;
                    }
                }
            }
        }
        // One line per request (a page), so the cost of the repair can be
        // measured over a batch: retry_requests is the number of extra calls.
        tracing::info!(
            provider = provider_id,
            segments = request.segments.len(),
            suspects = suspects.len(),
            retry_requests,
            repaired = repaired.iter().filter(|&&done| done).count(),
            "translation repair"
        );
        tracing::Span::current().record("outcome", "completed");
        Ok((provider_id, translated))
    }

    /// One request to the selected provider, checked for a reply per segment.
    async fn dispatch(
        &self,
        selection: &ModelSelection,
        generation: GenerationConfig,
        request: &TranslationRequest,
    ) -> anyhow::Result<Vec<String>> {
        let provider_id: &'static str = selection.provider.into();
        let expected = request.segments.len();
        let translated = if selection.provider == Provider::Local {
            self.local(selection)
                .await?
                .translate(request.clone(), generation)
                .await?
        } else {
            let providers = self.providers.read()?.clone();
            remote::translate(&self.client, &providers, selection, &generation, request).await?
        };
        if translated.len() != expected {
            return Err(Error::SegmentCount {
                provider: provider_id,
                expected,
                actual: translated.len(),
            }
            .into());
        }
        Ok(translated)
    }

    #[tracing::instrument(skip_all)]
    pub async fn models() -> anyhow::Result<Vec<Model>> {
        static CLIENT: tokio::sync::OnceCell<reqwest::Client> = tokio::sync::OnceCell::const_new();

        let providers = ProvidersConfig::load()?;
        let providers = providers.read()?.clone();
        let client = CLIENT
            .get_or_try_init(|| async {
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
            })
            .await?;
        let mut models = local::models();
        models.extend(remote::models(client, &providers).await);
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
}
