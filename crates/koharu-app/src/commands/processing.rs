use std::{collections::HashMap, fmt, sync::Arc};

use anyhow::{Context as _, Result};
use koharu_pipeline::{Committer, Progress, RunStatus, StageOutput, StopToken};
use koharu_scene::Snapshot;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

use super::{Channel, ChannelExt as _, Error, canvas::CanvasChannel, project::CurrentProject};
use koharu_desktop::Desktop;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, Type)]
#[serde(transparent)]
pub struct JobId(Uuid);

impl JobId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct Job {
    pub id: JobId,
    pub state: JobState,
    #[specta(type = f64)]
    pub completed: usize,
    #[specta(type = f64)]
    pub total: usize,
    pub page: Option<koharu_scene::EntityId>,
    pub stage: Option<koharu_pipeline::Stage>,
    pub model: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Finished,
    Failed,
    Stopped,
}

#[derive(Clone, Default)]
pub(crate) struct Processing {
    pub(crate) stops: Arc<Mutex<HashMap<JobId, StopToken>>>,
    pub(crate) jobs: Arc<Mutex<HashMap<JobId, Job>>>,
    pub(crate) inpainting_mask: Arc<Mutex<Option<koharu_pipeline::InpaintingMask>>>,
}

impl Processing {
    pub(crate) fn stop_all(&self) {
        let mut stops = self.stops.lock();
        for stop in stops.values() {
            stop.stop();
        }
        stops.clear();
        drop(stops);
        self.jobs.lock().clear();
    }
}

#[derive(Clone, Default)]
pub(crate) struct JobChannel {
    pub(crate) channel: Arc<Mutex<Option<Channel<Job>>>>,
}

#[koharu_macros::command]
pub(crate) async fn process(
    pipeline: crate::host::Pipeline,
    scope: koharu_pipeline::Scope,
    operation: koharu_pipeline::Operation,
    project: CurrentProject,
    processing: Processing,
    jobs: JobChannel,
    desktop: Desktop,
    canvas: CanvasChannel,
) -> std::result::Result<JobId, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    let id = JobId::new();
    let stop = StopToken::default();
    {
        let mut stops = processing.stops.lock();
        if !stops.is_empty() {
            return Err(anyhow::anyhow!("another process is already running").into());
        }
        stops.insert(id, stop.clone());
    }
    let job = Job {
        id,
        state: JobState::Running,
        completed: 0,
        total: 0,
        page: None,
        stage: None,
        model: None,
        error: None,
    };
    processing.jobs.lock().insert(id, job.clone());
    jobs.channel.publish(job);

    let inpainting_mask = processing.inpainting_mask.lock().take();
    let progress_processing = processing.clone();
    let progress_jobs = jobs.clone();
    let commit_project = project.clone();
    let commit_desktop = desktop.clone();
    let commit_canvas = canvas.clone();
    let finish_processing = processing.clone();
    let finish_jobs = jobs.clone();
    drop(tokio::spawn(async move {
        let progress = Arc::new(Mutex::new((0_usize, 0_usize)));
        let progress_processing = progress_processing.clone();
        let progress_jobs = progress_jobs.clone();
        let mut request =
            koharu_pipeline::Request::new(operation, scope, stop.clone(), Arc::from([]));
        if let Some(inpainting_mask) = inpainting_mask {
            request = request.with_inpainting_mask(inpainting_mask);
        }
        request = request.with_progress(Arc::new(move |event| {
            let update = match event {
                Progress::Started { pages, stages } => {
                    tracing::info!(
                        target: "koharu_metrics",
                        metric = "pipeline_start",
                        page_count = pages.len(),
                        stage_count = stages.len(),
                    );
                    let mut progress = progress.lock();
                    *progress = (0, pages.len().saturating_mul(stages.len()));
                    Some((0, progress.1, None, None, None))
                }
                Progress::Loading { page, stage, model } => {
                    tracing::info!(
                        target: "koharu_metrics",
                        metric = "stage_loading",
                        stage = %stage,
                        model,
                    );
                    let progress = progress.lock();
                    Some((progress.0, progress.1, Some(page), Some(stage), Some(model)))
                }
                Progress::Finished {
                    page,
                    stage,
                    model,
                    elapsed,
                } => {
                    if stage != koharu_pipeline::Stage::Translation {
                        tracing::info!(
                            target: "koharu_metrics",
                            metric = "model_run",
                            stage = %stage,
                            model,
                            duration_ms = elapsed.as_secs_f64() * 1000.0,
                        );
                    }
                    let mut progress = progress.lock();
                    progress.0 = progress.0.saturating_add(1).min(progress.1);
                    Some((progress.0, progress.1, Some(page), Some(stage), Some(model)))
                }
                Progress::Skipped { page, stage } => {
                    tracing::info!(
                        target: "koharu_metrics",
                        metric = "stage_skip",
                        stage = %stage,
                    );
                    let mut progress = progress.lock();
                    progress.0 = progress.0.saturating_add(1).min(progress.1);
                    Some((progress.0, progress.1, Some(page), Some(stage), None))
                }
                Progress::Running { stage, model, .. } => {
                    tracing::info!(
                        target: "koharu_metrics",
                        metric = "stage_running",
                        stage = %stage,
                        model,
                    );
                    None
                }
            };
            if let Some((completed, total, page, stage, model)) = update {
                let job = {
                    let mut jobs = progress_processing.jobs.lock();
                    jobs.get_mut(&id).map(|job| {
                        job.completed = completed;
                        job.total = total;
                        job.page = page;
                        job.stage = stage;
                        job.model = model;
                        job.clone()
                    })
                };
                if let Some(job) = job {
                    progress_jobs.channel.publish(job);
                }
            }
        }));

        struct PipelineCommitter {
            project: CurrentProject,
            desktop: Desktop,
            canvas: CanvasChannel,
        }

        #[async_trait::async_trait]
        impl Committer for PipelineCommitter {
            async fn commit(&mut self, output: StageOutput) -> Result<Snapshot> {
                let (commit, page) = {
                    let mut projects = self.project.project.lock().await;
                    let project = projects.as_mut().context("no project is open")?;
                    let Some(commit) = project.commit_rebased(output.patch).await? else {
                        return Ok(project.snapshot());
                    };
                    project.record_commit(&commit);
                    let page = project.active_page();
                    (commit, page)
                };
                let snapshot = commit.snapshot.clone();
                self.desktop
                    .synchronize(&commit.snapshot, page, &commit)
                    .await?;
                let canvas_state = self.desktop.canvas_state();
                self.canvas.channel.publish(canvas_state);
                Ok(snapshot)
            }
        }

        let mut committer = PipelineCommitter {
            project: commit_project,
            desktop: commit_desktop,
            canvas: commit_canvas,
        };
        let result = pipeline.execute(snapshot, request, &mut committer).await;
        let (stopped, error) = match result {
            Ok(report) => (report.status == RunStatus::Stopped, None),
            Err(error) => {
                tracing::error!(stage = ?error.stage, %error, "processing failed");
                (false, Some(format!("{error:#}")))
            }
        };
        tracing::info!(
            target: "koharu_metrics",
            metric = "pipeline_result",
            outcome = if stopped {
                "stopped"
            } else if error.is_some() {
                "failed"
            } else {
                "completed"
            },
        );
        finish_processing.stops.lock().remove(&id);
        let job = finish_processing.jobs.lock().remove(&id).map(|mut job| {
            job.state = if stopped {
                JobState::Stopped
            } else if error.is_some() {
                JobState::Failed
            } else {
                JobState::Finished
            };
            job.error = error;
            job
        });
        if let Some(job) = job {
            finish_jobs.channel.publish(job);
        }
    }));
    Ok(id)
}

#[tracing::instrument(
    target = "koharu_metrics",
    name = "pipeline_stop",
    skip_all,
    fields(state = "requested")
)]
#[koharu_macros::command]
pub(crate) async fn stop_job(job: JobId, processing: Processing) -> std::result::Result<(), Error> {
    let stops = processing.stops.lock();
    let stop = stops
        .get(&job)
        .with_context(|| format!("job {job} is not running"))?;
    stop.stop();
    Ok(())
}
