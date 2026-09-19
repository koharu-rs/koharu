use std::sync::Arc;

use anyhow::{Context as _, Result};
use arc_swap::ArcSwap;
use koharu_config::Config;
use koharu_scene::Snapshot;

use crate::{
    Committer, PipelineConfig, PipelineError, Report, Request, ResourceSnapshot,
    execution::Execution, resources::ResourceMonitor, stage_runner::StageRunner,
};

#[derive(Clone)]
pub struct Pipeline {
    current: Arc<ArcSwap<StageRunner>>,
    resources: Arc<ResourceMonitor>,
    execution: Arc<tokio::sync::Mutex<()>>,
}

impl Pipeline {
    pub fn load(device: koharu_ml::Device) -> Result<Self> {
        Self::from_config(
            PipelineConfig::load()?,
            koharu_translator::ProvidersConfig::load()?,
            device,
        )
    }

    #[tracing::instrument(skip_all)]
    pub fn from_config(
        config: Config<PipelineConfig>,
        providers: Config<koharu_translator::ProvidersConfig>,
        device: koharu_ml::Device,
    ) -> Result<Self> {
        let translator = koharu_translator::Translator::from_config(device.clone(), providers)?;
        let resources = ResourceMonitor::new(&device);
        let runner = {
            let value = config.read()?;
            StageRunner::new(&value, translator.clone(), &device, resources.clone())?
        };
        let current = Arc::new(ArcSwap::from_pointee(runner));
        let watched = current.clone();
        let watched_resources = resources.clone();
        let _watcher = tokio::runtime::Handle::try_current()
            .context("pipeline requires a Tokio runtime")?
            .spawn(async move {
                let mut changes = config.subscribe();
                while changes.changed().await.is_ok() {
                    let runner = config.read().and_then(|value| {
                        StageRunner::new(
                            &value,
                            translator.clone(),
                            &device,
                            watched_resources.clone(),
                        )
                    });
                    match runner {
                        Ok(runner) => watched.store(Arc::new(runner)),
                        Err(error) => tracing::error!(%error, "failed to reload pipeline"),
                    }
                }
            });
        Ok(Self {
            current,
            resources,
            execution: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub fn subscribe_resources(&self) -> tokio::sync::watch::Receiver<ResourceSnapshot> {
        self.resources.start();
        self.resources.subscribe()
    }

    pub async fn extract_glossary_terms(
        &self,
        text: &str,
        confidence_threshold: f32,
        stop: &crate::StopToken,
    ) -> Result<Option<Vec<koharu_ml::glossary_ner::GlossaryEntity>>> {
        let _execution = self.execution.lock().await;
        self.current
            .load_full()
            .extract_glossary_terms(text, confidence_threshold, stop)
            .await
    }

    pub fn unload_glossary_ner(&self) -> bool {
        self.current.load_full().unload_glossary_ner()
    }

    pub async fn translate_terms(
        &self,
        selection: &koharu_translator::ModelSelection,
        generation: koharu_translator::GenerationConfig,
        request: koharu_translator::TranslationRequest,
    ) -> Result<Vec<String>> {
        let _execution = self.execution.lock().await;
        self.current
            .load_full()
            .translate_terms(selection, generation, request)
            .await
    }

    #[tracing::instrument(skip_all)]
    pub async fn execute(
        &self,
        snapshot: Snapshot,
        request: Request,
        committer: &mut dyn Committer,
    ) -> std::result::Result<Report, PipelineError> {
        let _execution = self.execution.lock().await;
        Execution::new(
            self.current.load_full(),
            self.resources.clone(),
            snapshot,
            request,
            committer,
        )?
        .run()
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::Pipeline;

    #[tokio::test]
    async fn pipeline_clones_share_the_stage_runner_glossary_owner_and_explicit_unload() {
        let pipeline = Pipeline::from_config(
            koharu_config::Config::memory(crate::PipelineConfig::default()),
            koharu_config::Config::memory(koharu_translator::ProvidersConfig::default()),
            koharu_ml::Device::cpu(),
        )
        .unwrap();
        let cloned = pipeline.clone();

        assert!(std::sync::Arc::ptr_eq(
            &pipeline.current.load_full(),
            &cloned.current.load_full(),
        ));
        assert!(!pipeline.unload_glossary_ner());
        assert!(!cloned.unload_glossary_ner());
    }

    #[tokio::test]
    async fn term_translation_uses_the_pipeline_translator_owner_and_capability_gate() {
        let pipeline = Pipeline::from_config(
            koharu_config::Config::memory(crate::PipelineConfig::default()),
            koharu_config::Config::memory(koharu_translator::ProvidersConfig::default()),
            koharu_ml::Device::cpu(),
        )
        .unwrap();
        let selection = koharu_translator::ModelSelection {
            provider: koharu_translator::Provider::DeepL,
            model: None,
            quantization: None,
            vision: false,
            reasoning: false,
        };
        let request = koharu_translator::TranslationRequest::new_term_translation(
            ["アリス"],
            koharu_translator::Language::English,
        );

        let error = pipeline
            .translate_terms(
                &selection,
                koharu_translator::GenerationConfig::default(),
                request,
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("system prompt"), "{error:#}");
    }
}
