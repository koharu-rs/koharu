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
    translator: koharu_translator::Translator,
    config: Config<PipelineConfig>,
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
        let live_config = config.clone();
        let shared_translator = translator.clone();
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
            translator: shared_translator,
            config: live_config,
        })
    }

    pub async fn suggest_glossary_translations(
        &self,
        sources: Vec<String>,
        glossary: &[koharu_scene::GlossaryEntry],
    ) -> Result<Vec<String>> {
        anyhow::ensure!(
            !sources.is_empty() && sources.len() <= 24,
            "translate between 1 and 24 terms per batch"
        );
        anyhow::ensure!(
            sources
                .iter()
                .all(|source| !source.trim().is_empty() && source.len() <= 1024)
                && sources.iter().map(String::len).sum::<usize>() <= 4096,
            "glossary batch exceeds the source text limit"
        );
        let config = self.config.read()?.translation.clone();
        // Hosted providers answer batches in parallel; a local model must not
        // run beside a pipeline that owns the same weights.
        let _execution = if self.translator.concurrent(&config.model) {
            None
        } else {
            Some(
                self.execution
                    .try_lock()
                    .context("another pipeline operation is running")?,
            )
        };
        let instructions = format!(
            "{}\nTranslate these glossary terms as concise, consistent dictionary entries. Preserve proper-name identity. Use the target language configured in the request. Return only the translated term in each segment, without explanations or alternatives.",
            config.instructions.as_deref().unwrap_or_default()
        );
        let request = koharu_translator::TranslationRequest::new(sources, config.target_language)
            .with_glossary(glossary)
            .with_instructions(instructions);
        let (_, translations) = self
            .translator
            .translate(&config.model, config.generation, request)
            .await?;
        anyhow::ensure!(
            translations
                .iter()
                .all(|target| !target.trim().is_empty() && target.len() <= 4096),
            "translation provider returned an empty or oversized glossary translation"
        );
        Ok(translations)
    }

    pub fn subscribe_resources(&self) -> tokio::sync::watch::Receiver<ResourceSnapshot> {
        self.resources.start();
        self.resources.subscribe()
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
