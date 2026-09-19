use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use koharu_scene::{EntityId, Patch};

use crate::{
    ErrorKind, PipelineConfig, PipelineError, Progress, ProgressSink, Stage, StopToken,
    accelerator::AcceleratorGate,
    progress,
    resources::ResourceMonitor,
    stages::{StageDispatch, Stages},
};

pub(crate) struct StageRunner {
    stages: Stages,
    accelerator: AcceleratorGate,
    device: koharu_ml::Device,
    glossary_ner: crate::ModelCell<koharu_ml::glossary_ner::GlossaryNer>,
}

impl StageRunner {
    pub(crate) fn new(
        config: &PipelineConfig,
        translator: koharu_translator::Translator,
        device: &koharu_ml::Device,
        resources: Arc<ResourceMonitor>,
    ) -> Result<Self> {
        Ok(Self {
            stages: Stages::new(config, translator, device)?,
            accelerator: AcceleratorGate::new(device, resources),
            device: device.clone(),
            glossary_ner: crate::ModelCell::new(),
        })
    }

    #[tracing::instrument(skip_all)]
    pub(crate) async fn run(&self, job: StageJob) -> StageCompletion {
        let started = Instant::now();
        let page = job.input.page();
        let model = self.stages.model(job.stage).to_owned();
        let outcome = self.run_with_recovery(&job, &model).await;
        StageCompletion {
            page,
            stage: job.stage,
            model,
            elapsed: started.elapsed(),
            outcome,
        }
    }

    pub(crate) async fn translate_terms(
        &self,
        selection: &koharu_translator::ModelSelection,
        generation: koharu_translator::GenerationConfig,
        request: koharu_translator::TranslationRequest,
    ) -> Result<Vec<String>> {
        self.stages
            .translate_terms(selection, generation, request)
            .await
    }

    pub(crate) async fn extract_glossary_terms(
        &self,
        text: &str,
        confidence_threshold: f32,
        stop: &StopToken,
    ) -> Result<Option<Vec<koharu_ml::glossary_ner::GlossaryEntity>>> {
        if stop.stopped() {
            return Ok(None);
        }
        let permit = self.accelerator.acquire().await;
        if stop.stopped() {
            return Ok(None);
        }
        let first = self
            .load_and_extract_glossary_terms(text, confidence_threshold, stop)
            .await;
        let failure = match first {
            Ok(entities) => return Ok(entities),
            Err(error) if is_out_of_memory(&error) && !stop.stopped() => error,
            Err(error) => {
                self.glossary_ner.unload();
                return Err(error);
            }
        };

        drop(permit);
        self.glossary_ner.unload();
        tracing::warn!(error = %failure, "retrying glossary NER after memory pressure");
        let _permit = self
            .accelerator
            .recover(|| {
                unload_for_recovery(
                    None,
                    |stage| self.stages.unload(stage),
                    || self.glossary_ner.unload(),
                )
            })
            .await;
        if stop.stopped() {
            return Ok(None);
        }
        let result = self
            .load_and_extract_glossary_terms(text, confidence_threshold, stop)
            .await;
        if result.is_err() {
            self.glossary_ner.unload();
        }
        result
    }

    async fn load_and_extract_glossary_terms(
        &self,
        text: &str,
        confidence_threshold: f32,
        stop: &StopToken,
    ) -> Result<Option<Vec<koharu_ml::glossary_ner::GlossaryEntity>>> {
        if !ensure_model_with_cancellation(&self.glossary_ner, stop, || {
            koharu_ml::glossary_ner::GlossaryNer::load(self.device.clone())
        })
        .await
        .context("failed to load glossary NER model gliner_multi-v2.1")?
        {
            return Ok(None);
        }
        let cancelled = || stop.stopped();
        let cancellation = koharu_ml::glossary_ner::Cancellation::new(&cancelled);
        self.glossary_ner
            .lock()
            .await
            .as_ref()
            .expect("glossary NER initialized")
            .extract(text, confidence_threshold, &cancellation)
    }

    pub(crate) fn unload_glossary_ner(&self) -> bool {
        self.glossary_ner.unload()
    }

    async fn run_with_recovery(
        &self,
        job: &StageJob,
        model: &str,
    ) -> std::result::Result<StageOutcome, PipelineError> {
        if job.stop.stopped() {
            return Ok(StageOutcome::Stopped);
        }
        let skip = self.stages.skip(&job.input).map_err(|error| {
            self.stage_error(
                job.stage,
                model,
                AttemptFailure {
                    kind: ErrorKind::Processing,
                    error,
                },
            )
        })?;
        if skip {
            return Ok(StageOutcome::Skipped);
        }
        let permit = self.accelerator.acquire().await;
        if job.stop.stopped() {
            return Ok(StageOutcome::Stopped);
        }
        let first = self.load_and_process(job, model).await;
        let failure = match first {
            Ok(outcome) => return Ok(outcome),
            Err(failure) if is_out_of_memory(&failure.error) && !job.stop.stopped() => failure,
            Err(failure) => return Err(self.stage_error(job.stage, model, failure)),
        };

        drop(permit);
        tracing::warn!(stage = %job.stage, page = %job.input.page(), error = %failure.error, "retrying stage after memory pressure");
        let _metric =
            tracing::info_span!(target: "koharu_metrics", "stage_retry", stage = %job.stage, model);
        let _permit = self
            .accelerator
            .recover(|| {
                unload_for_recovery(
                    Some(job.stage),
                    |stage| self.stages.unload(stage),
                    || self.glossary_ner.unload(),
                )
            })
            .await;
        if job.stop.stopped() {
            return Ok(StageOutcome::Stopped);
        }
        match self.load_and_process(job, model).await {
            Ok(outcome) => Ok(outcome),
            Err(failure) => Err(self.stage_error(job.stage, model, failure)),
        }
    }

    async fn load_and_process(
        &self,
        job: &StageJob,
        model: &str,
    ) -> std::result::Result<StageOutcome, AttemptFailure> {
        progress::emit(
            job.progress.as_ref(),
            Progress::Loading {
                page: job.input.page(),
                stage: job.stage,
                model: model.to_owned(),
            },
        );
        self.stages
            .load(job.stage)
            .await
            .map_err(|error| AttemptFailure {
                kind: ErrorKind::ModelLoad,
                error,
            })?;
        if job.stop.stopped() {
            return Ok(StageOutcome::Stopped);
        }
        progress::emit(
            job.progress.as_ref(),
            Progress::Running {
                page: job.input.page(),
                stage: job.stage,
                model: model.to_owned(),
            },
        );
        self.stages
            .process(job.input.clone())
            .await
            .map(|patch| {
                if patch.is_empty() {
                    StageOutcome::Skipped
                } else {
                    StageOutcome::Patch(patch)
                }
            })
            .map_err(|error| AttemptFailure {
                kind: ErrorKind::Processing,
                error,
            })
    }

    fn stage_error(&self, stage: Stage, model: &str, failure: AttemptFailure) -> PipelineError {
        self.stages.unload(stage);
        let message = match failure.kind {
            ErrorKind::ModelLoad => format!("failed to load {model}"),
            _ => format!("{model} failed"),
        };
        PipelineError::new(failure.kind, Some(stage), failure.error.context(message))
    }
}

struct AttemptFailure {
    kind: ErrorKind,
    error: anyhow::Error,
}

fn is_out_of_memory(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        let message = source.to_string().to_ascii_lowercase();
        message.contains("out of memory")
            || message.contains("cuda_error_out_of_memory")
            || message.contains("not enough memory")
    })
}

async fn ensure_model_with_cancellation<M, F, Fut>(
    model: &crate::ModelCell<M>,
    stop: &StopToken,
    load: F,
) -> Result<bool>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<M>>,
{
    if stop.stopped() {
        return Ok(false);
    }
    model.ensure(load).await?;
    if stop.stopped() {
        model.unload();
        return Ok(false);
    }
    Ok(true)
}

fn unload_for_recovery(
    keep: Option<Stage>,
    mut unload_stage: impl FnMut(Stage) -> bool,
    unload_glossary: impl FnOnce() -> bool,
) -> bool {
    let mut unloaded = false;
    for stage in Stage::ALL {
        if Some(stage) != keep && unload_stage(stage) {
            unloaded = true;
            tracing::info!(target: "koharu_metrics", metric = "model_unload", stage = %stage);
            tracing::debug!(%stage, "unloaded model while recovering from memory pressure");
        }
    }
    if unload_glossary() {
        unloaded = true;
        tracing::info!(target: "koharu_metrics", metric = "model_unload", resource = "glossary_ner");
    }
    unloaded
}

#[cfg(test)]
mod lifecycle_tests {
    use std::cell::{Cell, RefCell};

    use super::{ensure_model_with_cancellation, unload_for_recovery};
    use crate::{ModelCell, Stage, StopToken};

    #[tokio::test]
    async fn cancellation_skips_or_discards_lazy_model_loading() {
        let model = ModelCell::new();
        let stop = StopToken::default();
        stop.stop();
        let loads = Cell::new(0_u8);
        assert!(
            !ensure_model_with_cancellation(&model, &stop, || async {
                loads.set(loads.get() + 1);
                Ok(1_u8)
            })
            .await
            .unwrap()
        );
        assert_eq!(loads.get(), 0);

        let stop = StopToken::default();
        assert!(
            !ensure_model_with_cancellation(&model, &stop, || async {
                loads.set(loads.get() + 1);
                stop.stop();
                Ok(2_u8)
            })
            .await
            .unwrap()
        );
        assert_eq!(loads.get(), 1);
        assert!(model.lock().await.is_none());
    }

    #[test]
    fn memory_recovery_includes_glossary_and_all_other_stage_models() {
        let unloaded = RefCell::new(Vec::new());
        let changed = unload_for_recovery(
            Some(Stage::Ocr),
            |stage| {
                unloaded.borrow_mut().push(stage.to_string());
                true
            },
            || {
                unloaded.borrow_mut().push("glossary_ner".to_owned());
                true
            },
        );

        assert!(changed);
        assert_eq!(
            unloaded.into_inner(),
            ["detection", "translation", "inpainting", "glossary_ner"]
        );
    }
}

pub(crate) struct StageJob {
    stage: Stage,
    input: StageDispatch,
    stop: StopToken,
    progress: Option<ProgressSink>,
}

impl StageJob {
    pub(crate) fn new(
        input: StageDispatch,
        stop: StopToken,
        progress: Option<ProgressSink>,
    ) -> Self {
        let stage = input.stage();
        Self {
            stage,
            input,
            stop,
            progress,
        }
    }

    #[cfg(test)]
    pub(crate) fn translation_input(&self) -> Option<&crate::stages::TranslationInput> {
        self.input.translation_input()
    }
}

pub(crate) enum StageOutcome {
    Patch(Patch),
    Skipped,
    Stopped,
}

pub(crate) struct StageCompletion {
    pub(crate) page: EntityId,
    pub(crate) stage: Stage,
    pub(crate) model: String,
    pub(crate) elapsed: Duration,
    pub(crate) outcome: std::result::Result<StageOutcome, PipelineError>,
}
