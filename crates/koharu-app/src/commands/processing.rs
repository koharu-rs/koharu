use std::{fmt, sync::Arc};

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
    pub kind: JobKind,
    pub phase: JobPhase,
    pub state: JobState,
    #[specta(type = f64)]
    pub completed: usize,
    #[specta(type = f64)]
    pub total: usize,
    pub page: Option<koharu_scene::EntityId>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Pipeline,
    GlossaryScan,
    GlossaryTranslation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobPhase {
    Pipeline {
        stage: Option<koharu_pipeline::Stage>,
    },
    PreparingOcr,
    ExtractingTerms,
    TranslatingTerms,
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
    registry: Arc<JobRegistry>,
    pub(crate) inpainting_mask: Arc<Mutex<Option<koharu_pipeline::InpaintingMask>>>,
}

#[derive(Default)]
struct JobRegistry {
    state: Mutex<JobRegistryState>,
}

#[derive(Default)]
struct JobRegistryState {
    active: Option<ActiveJob>,
}

struct ActiveJob {
    job: Job,
    stop: StopToken,
}

impl Processing {
    pub(crate) fn start_job(
        &self,
        kind: JobKind,
        phase: JobPhase,
        channel: &JobChannel,
    ) -> Result<(JobId, StopToken)> {
        let id = JobId::new();
        let stop = StopToken::default();
        let job = Job {
            id,
            kind,
            phase,
            state: JobState::Running,
            completed: 0,
            total: 0,
            page: None,
            error: None,
        };
        let mut registry = self.registry.state.lock();
        if registry.active.is_some() {
            anyhow::bail!("another process is already running");
        }
        registry.active = Some(ActiveJob {
            job: job.clone(),
            stop: stop.clone(),
        });
        drop(registry);
        channel.channel.publish(job);
        Ok((id, stop))
    }

    pub(crate) fn update_job(
        &self,
        id: JobId,
        channel: &JobChannel,
        update: impl FnOnce(&mut Job),
    ) {
        let mut registry = self.registry.state.lock();
        if let Some(active) = registry
            .active
            .as_mut()
            .filter(|active| active.job.id == id)
        {
            update(&mut active.job);
            let job = active.job.clone();
            channel.channel.publish(job);
        }
    }

    pub(crate) fn finish_job(
        &self,
        id: JobId,
        state: JobState,
        error: Option<String>,
        channel: &JobChannel,
    ) {
        let mut registry = self.registry.state.lock();
        if registry
            .active
            .as_ref()
            .is_some_and(|active| active.job.id == id)
        {
            let mut active = registry.active.take().expect("active job checked");
            active.job.state = state;
            active.job.error = error;
            let job = active.job;
            channel.channel.publish(job);
        }
    }

    pub(crate) fn stop_all(&self) {
        let registry = self.registry.state.lock();
        if let Some(active) = registry.active.as_ref() {
            active.stop.stop();
        }
    }

    pub(crate) fn stop_job(&self, id: JobId) -> Result<()> {
        let registry = self.registry.state.lock();
        let active = registry
            .active
            .as_ref()
            .filter(|active| active.job.id == id)
            .with_context(|| format!("job {id} is not running"))?;
        active.stop.stop();
        Ok(())
    }

    pub(crate) fn is_running(&self) -> bool {
        self.registry.state.lock().active.is_some()
    }

    pub(crate) fn snapshot(&self) -> Vec<Job> {
        self.registry
            .state
            .lock()
            .active
            .as_ref()
            .map(|active| vec![active.job.clone()])
            .unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub(crate) struct JobChannel {
    pub(crate) channel: Arc<Mutex<Option<Channel<Job>>>>,
}

pub(crate) struct ProjectCommitter {
    pub(crate) project: CurrentProject,
    pub(crate) desktop: Desktop,
    pub(crate) canvas: CanvasChannel,
}

#[async_trait::async_trait]
impl Committer for ProjectCommitter {
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

pub(crate) fn terminology_snapshot(
    snapshot: &Snapshot,
) -> Result<Arc<[koharu_translator::TerminologyEntry]>> {
    let Some(glossary) = snapshot.project_component::<koharu_scene::Glossary>()? else {
        return Ok(Arc::from([]));
    };
    Ok(koharu_pipeline::terminology_from_glossary(&glossary))
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
    let terminology = terminology_snapshot(&snapshot)?;
    let (id, stop) =
        processing.start_job(JobKind::Pipeline, JobPhase::Pipeline { stage: None }, &jobs)?;

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
            koharu_pipeline::Request::new(operation, scope, stop.clone(), terminology);
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
            if let Some((completed, total, page, stage, _model)) = update {
                progress_processing.update_job(id, &progress_jobs, |job| {
                    job.completed = completed;
                    job.total = total;
                    job.page = page;
                    job.phase = JobPhase::Pipeline { stage };
                });
            }
        }));

        let mut committer = ProjectCommitter {
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
        let state = if stopped {
            JobState::Stopped
        } else if error.is_some() {
            JobState::Failed
        } else {
            JobState::Finished
        };
        finish_processing.finish_job(id, state, error, &finish_jobs);
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
    processing.stop_job(job)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Job, JobId, JobKind, JobPhase, JobState};

    #[test]
    fn job_protocol_uses_explicit_kind_and_phase_without_legacy_fields() {
        let job = Job {
            id: JobId::new(),
            kind: JobKind::Pipeline,
            phase: JobPhase::Pipeline {
                stage: Some(koharu_pipeline::Stage::Ocr),
            },
            state: JobState::Running,
            completed: 1,
            total: 2,
            page: None,
            error: None,
        };

        let value = serde_json::to_value(job).unwrap();
        assert_eq!(value["kind"], "pipeline");
        assert_eq!(value["phase"]["kind"], "pipeline");
        assert_eq!(value["phase"]["stage"], "ocr");
        assert!(value.get("stage").is_none());
        assert!(value.get("model").is_none());
    }

    #[test]
    fn glossary_jobs_have_dedicated_protocol_states() {
        assert_eq!(
            serde_json::to_value(JobKind::GlossaryScan).unwrap(),
            "glossary_scan"
        );
        assert_eq!(
            serde_json::to_value(JobKind::GlossaryTranslation).unwrap(),
            "glossary_translation"
        );
        for (phase, expected) in [
            (JobPhase::PreparingOcr, "preparing_ocr"),
            (JobPhase::ExtractingTerms, "extracting_terms"),
            (JobPhase::TranslatingTerms, "translating_terms"),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap()["kind"], expected);
        }
    }

    #[test]
    fn every_job_kind_reserves_the_same_processing_slot() {
        let processing = super::Processing::default();
        let channel = super::JobChannel::default();
        let (first, _) = processing
            .start_job(JobKind::GlossaryScan, JobPhase::PreparingOcr, &channel)
            .unwrap();
        assert!(
            processing
                .start_job(
                    JobKind::Pipeline,
                    JobPhase::Pipeline { stage: None },
                    &channel,
                )
                .is_err()
        );

        processing.finish_job(first, JobState::Stopped, None, &channel);
        assert!(processing.snapshot().is_empty());
    }

    #[test]
    fn stop_all_cannot_interleave_with_job_publication() {
        use std::{sync::mpsc, time::Duration};

        let processing = super::Processing::default();
        let channel = super::JobChannel::default();
        let (published, published_rx) = mpsc::sync_channel(0);
        let (release, release_rx) = mpsc::sync_channel(0);
        let release_rx = std::sync::Mutex::new(release_rx);
        *channel.channel.lock() = Some(crate::commands::Channel::from_sink(move |_| {
            published.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            true
        }));

        let starting = processing.clone();
        let start_channel = channel.clone();
        let started = std::thread::spawn(move || {
            starting.start_job(
                JobKind::GlossaryScan,
                JobPhase::PreparingOcr,
                &start_channel,
            )
        });
        published_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let stopping = processing.clone();
        let (stopped, stopped_rx) = mpsc::sync_channel(0);
        let stop_all = std::thread::spawn(move || {
            stopping.stop_all();
            stopped.send(()).unwrap();
        });
        stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            processing
                .start_job(
                    JobKind::Pipeline,
                    JobPhase::Pipeline { stage: None },
                    &super::JobChannel::default(),
                )
                .is_err()
        );
        release.send(()).unwrap();
        let (job, token) = started.join().unwrap().unwrap();
        stop_all.join().unwrap();

        assert!(token.stopped());
        processing.finish_job(job, JobState::Stopped, None, &super::JobChannel::default());
        assert!(processing.snapshot().is_empty());
        assert!(
            processing
                .start_job(
                    JobKind::Pipeline,
                    JobPhase::Pipeline { stage: None },
                    &super::JobChannel::default(),
                )
                .is_ok()
        );
    }

    #[tokio::test]
    async fn process_terminology_is_taken_once_from_the_starting_snapshot() {
        use koharu_scene::{
            Glossary, GlossaryEntry, GlossaryEntryId, GlossaryKind, GlossaryValueOrigin, Session,
        };

        let mut session = Session::memory().await.unwrap();
        let glossary = Glossary {
            enabled: true,
            source_language: None,
            target_language: None,
            source_fingerprint: None,
            entries: vec![GlossaryEntry {
                id: GlossaryEntryId::new(),
                source: "アリス".to_owned(),
                translation: Some("Alice".to_owned()),
                kind: GlossaryKind::Person,
                enabled: true,
                confidence: None,
                occurrence_count: 0,
                examples: Vec::new(),
                source_origin: GlossaryValueOrigin::User,
                translation_origin: Some(GlossaryValueOrigin::User),
                present_in_last_scan: true,
            }],
        };
        let patch = session
            .snapshot()
            .patch(|edit| edit.set_project(&glossary))
            .unwrap();
        session.commit(patch).await.unwrap();
        let snapshot = session.snapshot();

        let terminology = super::terminology_snapshot(&snapshot).unwrap();
        assert_eq!(terminology.len(), 1);
        assert_eq!(terminology[0].source, "アリス");
        assert_eq!(terminology[0].translation, "Alice");
    }
}
