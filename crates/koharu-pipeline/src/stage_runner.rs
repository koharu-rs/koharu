use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use koharu_scene::{EntityId, Patch};

use crate::{
    ErrorKind, PipelineConfig, PipelineError, Progress, ProgressSink, Stage, StopToken,
    accelerator::AcceleratorGate,
    progress,
    resources::ResourceMonitor,
    stages::{StageInput, Stages},
};

pub(crate) struct StageRunner {
    stages: Stages,
    accelerator: AcceleratorGate,
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

    async fn run_with_recovery(
        &self,
        job: &StageJob,
        model: &str,
    ) -> std::result::Result<StageOutcome, PipelineError> {
        if job.stop.stopped() {
            return Ok(StageOutcome::Stopped);
        }
        let skip = self.stages.skip(job.stage, &job.input).map_err(|error| {
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
        let _permit = self.accelerator.recover(job.stage, &self.stages).await;
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
            .process(job.stage, job.input.clone())
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
    // llama.cpp reports an exhausted device budget as a null model or context
    // handle, which names no memory, so it is matched by type rather than text.
    koharu_ml::llm::is_allocation_failure(error)
        || error.chain().any(|source| {
            let message = source.to_string().to_ascii_lowercase();
            message.contains("out of memory")
                || message.contains("cuda_error_out_of_memory")
                || message.contains("not enough memory")
                || message.contains("failed to allocate")
                || message.contains("unable to allocate")
        })
}

pub(crate) struct StageJob {
    stage: Stage,
    input: StageInput,
    stop: StopToken,
    progress: Option<ProgressSink>,
}

impl StageJob {
    pub(crate) fn new(
        stage: Stage,
        input: StageInput,
        stop: StopToken,
        progress: Option<ProgressSink>,
    ) -> Self {
        Self {
            stage,
            input,
            stop,
            progress,
        }
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

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use koharu_ml::llama::{
        ApplyChatTemplateError, LlamaContextLoadError, LlamaCppError, LlamaModelLoadError,
    };

    use super::is_out_of_memory;

    #[test]
    fn model_load_null_result_is_out_of_memory() {
        let error = anyhow::Error::new(LlamaModelLoadError::NullResult)
            .context("failed to load GGUF model model.gguf")
            .context("failed to load local translation model");
        assert!(is_out_of_memory(&error));
    }

    #[test]
    fn context_null_return_is_out_of_memory() {
        let error: anyhow::Error = Err::<(), _>(LlamaContextLoadError::NullReturn)
            .context("failed to create llama.cpp context")
            .unwrap_err();
        assert!(is_out_of_memory(&error));
    }

    #[test]
    fn wrapped_llama_errors_are_out_of_memory() {
        let error = anyhow::Error::new(LlamaCppError::from(LlamaContextLoadError::NullReturn));
        assert!(is_out_of_memory(&error));
        let error = anyhow::Error::new(LlamaCppError::from(LlamaModelLoadError::NullResult));
        assert!(is_out_of_memory(&error));
    }

    #[test]
    fn chat_template_null_result_is_not_out_of_memory() {
        let error = anyhow::Error::new(ApplyChatTemplateError::NullResult)
            .context("failed to render GGUF chat template");
        assert!(!is_out_of_memory(&error));
    }

    #[test]
    fn other_model_load_errors_are_not_out_of_memory() {
        let error = anyhow::Error::new(LlamaModelLoadError::PathToStrError("model.gguf".into()));
        assert!(!is_out_of_memory(&error));
    }

    #[test]
    fn bounds_violation_is_not_out_of_memory() {
        let error = anyhow::anyhow!("invalid vector subscript");
        assert!(!is_out_of_memory(&error));
    }

    #[test]
    fn messages_naming_memory_are_out_of_memory() {
        for message in [
            "CUDA error: out of memory",
            "CUDA_ERROR_OUT_OF_MEMORY",
            "not enough memory to complete the operation",
            "ggml_backend: failed to allocate buffer",
        ] {
            assert!(
                is_out_of_memory(&anyhow::anyhow!(message.to_owned())),
                "{message}"
            );
        }
    }
}
