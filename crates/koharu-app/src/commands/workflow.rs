use std::{collections::BTreeSet, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use koharu_pipeline::{Operation, Progress, Request, RunStatus, Scope, Stage, StopToken};
use koharu_scene::{EntityId, ProjectGlossary, ProjectId, Snapshot};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager as _};
use tauri_runtime_cef::CefRuntime;

use super::{
    ChannelExt as _, Error,
    processing::{Job, JobChannel, JobId, JobKind, JobState, PipelineCommitter, Processing},
    project::CurrentProject,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Scheduling {
    #[default]
    PageMajor,
    StageMajor,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStep {
    Detection,
    Ocr,
    Terminology,
    Translation,
    Inpainting,
}

impl WorkflowStep {
    fn stage(self) -> Option<Stage> {
        match self {
            Self::Detection => Some(Stage::Detection),
            Self::Ocr => Some(Stage::Ocr),
            Self::Terminology => None,
            Self::Translation => Some(Stage::Translation),
            Self::Inpainting => Some(Stage::Inpainting),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScope {
    #[default]
    Project,
    SelectedPages,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Type)]
#[serde(default)]
pub struct WorkflowPreset {
    pub name: String,
    pub scheduling: Scheduling,
    pub scope: WorkflowScope,
    pub stages: Vec<WorkflowStep>,
    pub review_glossary: bool,
}

impl Default for WorkflowPreset {
    fn default() -> Self {
        Self {
            name: "Standard".into(),
            scheduling: Scheduling::PageMajor,
            scope: WorkflowScope::Project,
            stages: vec![
                WorkflowStep::Detection,
                WorkflowStep::Ocr,
                WorkflowStep::Translation,
                WorkflowStep::Inpainting,
            ],
            review_glossary: true,
        }
    }
}

impl WorkflowPreset {
    fn steps(&self) -> Result<Vec<WorkflowStep>> {
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 128,
            "preset name must contain 1–128 bytes"
        );
        ensure!(
            !self.stages.is_empty() && self.stages.windows(2).all(|pair| pair[0] < pair[1]),
            "choose unique workflow stages in pipeline order"
        );
        ensure!(
            self.scheduling == Scheduling::StageMajor
                || !self.stages.contains(&WorkflowStep::Terminology),
            "terminology requires stage-major scheduling"
        );
        let stages = self
            .stages
            .iter()
            .copied()
            .filter(|stage| *stage != WorkflowStep::Terminology || self.review_glossary)
            .collect::<Vec<_>>();
        ensure!(
            !stages.is_empty(),
            "at least one workflow stage must be enabled"
        );
        Ok(stages)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
#[serde(default)]
pub struct WorkflowSettings {
    pub enabled: bool,
    pub active_preset: String,
    pub presets: Vec<WorkflowPreset>,
}

impl Default for WorkflowSettings {
    fn default() -> Self {
        let mut consistent = WorkflowPreset {
            name: "Consistent Project Translation".into(),
            scheduling: Scheduling::StageMajor,
            ..Default::default()
        };
        consistent.stages.insert(2, WorkflowStep::Terminology);
        Self {
            enabled: false,
            active_preset: consistent.name.clone(),
            presets: vec![WorkflowPreset::default(), consistent],
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStageState {
    Pending,
    Running,
    AwaitingReview,
    Complete,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct WorkflowStageProgress {
    pub stage: WorkflowStep,
    pub state: WorkflowStageState,
    pub completed: u32,
    pub total: u32,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct WorkflowProgress {
    pub name: String,
    pub stages: Vec<WorkflowStageProgress>,
}

#[tauri::command]
#[specta::specta]
pub(crate) fn get_workflow_settings() -> std::result::Result<WorkflowSettings, Error> {
    Ok(koharu_config::load::<WorkflowSettings>("workflows")?
        .read()?
        .clone())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn configure_project_workflow(
    enabled: bool,
    active_preset: String,
) -> std::result::Result<WorkflowSettings, Error> {
    let config = koharu_config::load::<WorkflowSettings>("workflows")?;
    let mut current = config.write()?;
    current
        .presets
        .iter()
        .find(|preset| preset.name == active_preset)
        .context("workflow preset no longer exists")?;
    current.enabled = enabled;
    current.active_preset = active_preset;
    let saved = current.clone();
    current.save()?;
    Ok(saved)
}

impl WorkflowSettings {
    fn for_run(&self, scope: &Scope, operation: &Operation) -> Result<Option<WorkflowPreset>> {
        let multiple = matches!(scope, Scope::Project)
            || matches!(scope, Scope::Pages(pages) if pages.len() > 1);
        if !self.enabled || !multiple || !matches!(operation, Operation::Full) {
            return Ok(None);
        }
        let mut preset = self
            .presets
            .iter()
            .find(|preset| preset.name == self.active_preset)
            .context("selected project workflow preset no longer exists")?
            .clone();
        preset.scope = if matches!(scope, Scope::Project) {
            WorkflowScope::Project
        } else {
            WorkflowScope::SelectedPages
        };
        Ok(Some(preset))
    }
}

pub(crate) fn configured_preset(
    scope: &Scope,
    operation: &Operation,
) -> Result<Option<WorkflowPreset>> {
    koharu_config::load::<WorkflowSettings>("workflows")?
        .read()?
        .for_run(scope, operation)
}
#[tauri::command]
#[specta::specta]
pub(crate) fn get_workflow_presets() -> std::result::Result<Vec<WorkflowPreset>, Error> {
    Ok(koharu_config::load::<WorkflowSettings>("workflows")?
        .read()?
        .presets
        .clone())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn save_workflow_presets(
    presets: Vec<WorkflowPreset>,
) -> std::result::Result<Vec<WorkflowPreset>, Error> {
    if presets.is_empty() || presets.len() > 50 {
        return Err(anyhow::anyhow!("save between 1 and 50 presets").into());
    }
    let mut names = BTreeSet::new();
    for preset in &presets {
        preset.steps()?;
        if !names.insert(preset.name.trim()) {
            return Err(anyhow::anyhow!("preset names must be unique").into());
        }
    }
    let config = koharu_config::load::<WorkflowSettings>("workflows")?;
    let mut current = config.write()?;
    if !presets
        .iter()
        .any(|preset| preset.name == current.active_preset)
    {
        current.active_preset = presets[0].name.clone();
        current.enabled = false;
    }
    current.presets = presets.clone();
    current.save()?;
    Ok(presets)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn start_workflow(
    handle: AppHandle<CefRuntime>,
    scope: Scope,
    preset: WorkflowPreset,
) -> std::result::Result<JobId, Error> {
    let steps = preset.steps()?;
    let snapshot = handle
        .state::<CurrentProject>()
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    let pages = scoped_pages(&snapshot, &scope)?;
    let project = snapshot.project_id();
    let total = u32::try_from(pages.len())?;
    let id = JobId::new();
    let stop = StopToken::default();
    {
        let processing = handle.state::<Processing>();
        let mut stops = processing.stops.lock();
        ensure_idle(&stops)?;
        stops.insert(id, stop.clone());
    }
    let job = Job {
        id,
        kind: JobKind::Workflow,
        state: JobState::Running,
        completed: 0,
        total: pages.len() * steps.len(),
        page: None,
        stage: None,
        model: None,
        error: None,
        workflow: Some(WorkflowProgress {
            name: preset.name.clone(),
            stages: steps
                .iter()
                .map(|stage| WorkflowStageProgress {
                    stage: *stage,
                    state: WorkflowStageState::Pending,
                    completed: 0,
                    total,
                })
                .collect(),
        }),
    };
    handle
        .state::<Processing>()
        .jobs
        .lock()
        .insert(id, job.clone());
    handle.state::<JobChannel>().channel.publish(job);
    drop(tokio::spawn(async move {
        let result = execute(&handle, id, project, pages, preset, stop.clone()).await;
        let (state, error) = match result {
            Ok(true) => (JobState::Finished, None),
            Ok(false) => (JobState::Stopped, None),
            Err(error) => (
                if stop.stopped() {
                    JobState::Stopped
                } else {
                    JobState::Failed
                },
                Some(format!("{error:#}")),
            ),
        };
        update(&handle, id, |job| {
            job.state = state;
            job.error = error;
            if let Some(workflow) = &mut job.workflow {
                for stage in &mut workflow.stages {
                    if matches!(
                        stage.state,
                        WorkflowStageState::Running | WorkflowStageState::AwaitingReview
                    ) {
                        stage.state = if state == JobState::Failed {
                            WorkflowStageState::Failed
                        } else {
                            WorkflowStageState::Stopped
                        };
                    }
                }
            }
        });
        handle.state::<Processing>().reviews.lock().remove(&id);
        handle.state::<Processing>().stops.lock().remove(&id);
        handle.state::<Processing>().jobs.lock().remove(&id);
    }));
    Ok(id)
}

fn ensure_idle(stops: &std::collections::HashMap<JobId, StopToken>) -> Result<()> {
    ensure!(stops.is_empty(), "another process is already running");
    Ok(())
}

fn scoped_pages(snapshot: &Snapshot, scope: &Scope) -> Result<Vec<EntityId>> {
    let requested = match scope {
        Scope::Project => None,
        Scope::Pages(pages) => {
            for page in pages {
                snapshot.page(*page)?;
            }
            Some(pages.iter().copied().collect::<BTreeSet<_>>())
        }
        _ => anyhow::bail!("project workflows support entire project or selected pages"),
    };
    let pages = snapshot
        .pages()
        .map(|page| page.id())
        .filter(|page| {
            requested
                .as_ref()
                .is_none_or(|requested| requested.contains(page))
        })
        .collect::<Vec<_>>();
    ensure!(!pages.is_empty(), "there are no pages to process");
    Ok(pages)
}

async fn snapshot(handle: &AppHandle<CefRuntime>, project: ProjectId) -> Result<Snapshot> {
    let snapshot = handle
        .state::<CurrentProject>()
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    ensure!(
        snapshot.project_id() == project,
        "the active project changed"
    );
    Ok(snapshot)
}

async fn execute(
    handle: &AppHandle<CefRuntime>,
    id: JobId,
    project: ProjectId,
    pages: Vec<EntityId>,
    preset: WorkflowPreset,
    stop: StopToken,
) -> Result<bool> {
    let pipeline = handle.state::<koharu_pipeline::Pipeline>().inner().clone();
    let steps = preset.steps()?;
    let scope = Scope::Pages(pages.clone());
    let mut committer = PipelineCommitter {
        handle: handle.clone(),
    };
    if preset.scheduling == Scheduling::PageMajor {
        let progress_handle = handle.clone();
        let report = pipeline
            .execute(
                snapshot(handle, project).await?,
                Request {
                    operation: Operation::Stages {
                        stages: steps.iter().filter_map(|step| step.stage()).collect(),
                    },
                    scope,
                    stop: stop.clone(),
                    progress: Some(Arc::new(move |event| {
                        record_progress(&progress_handle, id, event)
                    })),
                    ..Default::default()
                },
                &mut committer,
            )
            .await?;
        return Ok(report.status == RunStatus::Completed);
    }
    for (index, step) in steps.into_iter().enumerate() {
        if stop.stopped() {
            return Ok(false);
        }
        update(handle, id, |job| {
            job.state = JobState::Running;
            job.stage = step.stage();
            job.model = None;
            job.page = None;
            job.workflow.as_mut().unwrap().stages[index].state = WorkflowStageState::Running;
        });
        if let Some(stage) = step.stage() {
            let progress_handle = handle.clone();
            let report = pipeline
                .execute(
                    snapshot(handle, project).await?,
                    Request {
                        operation: Operation::StageMajor {
                            stages: vec![stage],
                        },
                        scope: scope.clone(),
                        stop: stop.clone(),
                        progress: Some(Arc::new(move |event| {
                            record_progress(&progress_handle, id, event)
                        })),
                        ..Default::default()
                    },
                    &mut committer,
                )
                .await?;
            if report.status == RunStatus::Stopped {
                return Ok(false);
            }
        } else {
            let source = snapshot(handle, project).await?;
            let worker_pages = pages.clone();
            let worker_stop = stop.clone();
            let progress_handle = handle.clone();
            let candidates = tokio::task::spawn_blocking(move || {
                koharu_pipeline::analyze_terminology(
                    &source,
                    &worker_pages,
                    &worker_stop,
                    |completed| {
                        update(&progress_handle, id, |job| {
                            job.workflow.as_mut().unwrap().stages[index].completed =
                                completed as u32
                        });
                    },
                )
            })
            .await??;
            let Some(candidates) = candidates else {
                return Ok(false);
            };
            if stop.stopped() {
                return Ok(false);
            }
            let pending = {
                let projects = handle.state::<CurrentProject>();
                let mut projects = projects.project.lock().await;
                let current = projects.as_mut().context("no project is open")?;
                let source = current.snapshot();
                ensure!(source.project_id() == project, "the active project changed");
                let mut glossary = source
                    .project_component::<ProjectGlossary>()?
                    .unwrap_or_default();
                glossary.replace_candidates(candidates);
                let pending = !glossary.candidates.is_empty();
                let patch = source
                    .patch(|edit| edit.set_project(&glossary))?
                    .with_label("Analyze project terminology");
                if let Some(commit) = current.commit_rebased(patch).await? {
                    current.record_commit(&commit);
                }
                pending
            };
            if pending {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                handle
                    .state::<Processing>()
                    .reviews
                    .lock()
                    .insert(id, sender);
                if stop.stopped() {
                    handle.state::<Processing>().reviews.lock().remove(&id);
                    return Ok(false);
                }
                update(handle, id, |job| {
                    job.state = JobState::AwaitingReview;
                    job.workflow.as_mut().unwrap().stages[index].state =
                        WorkflowStageState::AwaitingReview;
                });
                if receiver.await.is_err() || stop.stopped() {
                    return Ok(false);
                }
            }
        }
        update(handle, id, |job| {
            job.state = JobState::Running;
            let stage = &mut job.workflow.as_mut().unwrap().stages[index];
            stage.completed = stage.total;
            stage.state = WorkflowStageState::Complete;
        });
    }
    Ok(!stop.stopped())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn resume_workflow(
    handle: AppHandle<CefRuntime>,
    job: JobId,
) -> std::result::Result<(), Error> {
    let sender = handle
        .state::<Processing>()
        .reviews
        .lock()
        .remove(&job)
        .context("workflow is not awaiting glossary review")?;
    sender
        .send(())
        .map_err(|_| anyhow::anyhow!("workflow stopped before review completed"))?;
    Ok(())
}

fn record_progress(handle: &AppHandle<CefRuntime>, id: JobId, event: Progress) {
    let (page, stage, model, finished) = match event {
        Progress::Started { .. } => return,
        Progress::Loading { page, stage, model } | Progress::Running { page, stage, model } => {
            (page, stage, Some(model), false)
        }
        Progress::Finished {
            page, stage, model, ..
        } => (page, stage, Some(model), true),
        Progress::Skipped { page, stage } => (page, stage, None, true),
    };
    update(handle, id, |job| {
        job.page = Some(page);
        job.stage = Some(stage);
        job.model = model;
        if let Some(progress) = job.workflow.as_mut().and_then(|workflow| {
            workflow
                .stages
                .iter_mut()
                .find(|step| step.stage.stage() == Some(stage))
        }) {
            progress.state = WorkflowStageState::Running;
            if finished {
                progress.completed = progress.completed.saturating_add(1).min(progress.total);
            }
            if progress.completed == progress.total {
                progress.state = WorkflowStageState::Complete;
            }
        }
    });
}

fn update(handle: &AppHandle<CefRuntime>, id: JobId, change: impl FnOnce(&mut Job)) {
    let job = {
        let processing = handle.state::<Processing>();
        let mut jobs = processing.jobs.lock();
        jobs.get_mut(&id).map(|job| {
            change(job);
            if let Some(workflow) = &job.workflow {
                job.completed = workflow
                    .stages
                    .iter()
                    .map(|stage| stage.completed as usize)
                    .sum();
                job.total = workflow
                    .stages
                    .iter()
                    .map(|stage| stage.total as usize)
                    .sum();
            }
            job.clone()
        })
    };
    if let Some(job) = job {
        handle.state::<JobChannel>().channel.publish(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_roundtrip_and_review_is_never_auto_accepted() {
        let defaults: WorkflowSettings = serde_json::from_str("{}").unwrap();
        let old: WorkflowPreset = serde_json::from_str("{}").unwrap();
        assert_eq!(old.scope, WorkflowScope::Project);
        assert_eq!(old, WorkflowPreset::default());
        let consistent = &defaults.presets[1];
        assert_eq!(
            consistent.steps().unwrap(),
            [
                WorkflowStep::Detection,
                WorkflowStep::Ocr,
                WorkflowStep::Terminology,
                WorkflowStep::Translation,
                WorkflowStep::Inpainting
            ]
        );
        assert!(consistent.review_glossary);
        let saved: WorkflowPreset =
            serde_json::from_str(&serde_json::to_string(consistent).unwrap()).unwrap();
        assert_eq!(&saved, consistent);
        let mut skipped = consistent.clone();
        skipped.review_glossary = false;
        assert!(
            !skipped
                .steps()
                .unwrap()
                .contains(&WorkflowStep::Terminology)
        );
        skipped.stages.reverse();
        assert!(skipped.steps().is_err());
    }

    #[tokio::test]
    async fn selected_pages_follow_project_order() {
        let mut session = koharu_scene::Session::memory().await.unwrap();
        let mut edit = session.snapshot().edit();
        let mut pages = Vec::new();
        for name in ["one", "two", "three"] {
            pages.push(
                edit.add_page(
                    koharu_scene::PageDraft::new(name, 1.0, 1.0),
                    koharu_scene::At::End,
                )
                .unwrap(),
            );
        }
        session.commit(edit.finish().unwrap()).await.unwrap();
        assert_eq!(
            scoped_pages(
                &session.snapshot(),
                &Scope::Pages(vec![pages[2], pages[0], pages[2]])
            )
            .unwrap(),
            [pages[0], pages[2]]
        );
        assert!(scoped_pages(&session.snapshot(), &Scope::Pages(Vec::new())).is_err());
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn enabled_project_runs_use_the_selected_preset() {
        let mut settings = WorkflowSettings::default();
        assert!(
            settings
                .for_run(&Scope::Project, &Operation::Full)
                .unwrap()
                .is_none()
        );
        settings.enabled = true;
        let preset = settings
            .for_run(&Scope::Project, &Operation::Full)
            .unwrap()
            .unwrap();
        assert_eq!(preset.scheduling, Scheduling::StageMajor);
        assert!(preset.stages.contains(&WorkflowStep::Terminology));
        assert!(
            settings
                .for_run(&Scope::Project, &Operation::Only { stage: Stage::Ocr })
                .unwrap()
                .is_none()
        );
        let page = EntityId::new();
        assert!(
            settings
                .for_run(&Scope::Pages(vec![page]), &Operation::Full)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            settings
                .for_run(&Scope::Pages(vec![page, EntityId::new()]), &Operation::Full)
                .unwrap()
                .unwrap()
                .scope,
            WorkflowScope::SelectedPages
        );
        settings.active_preset = "Standard".into();
        assert_eq!(
            settings
                .for_run(&Scope::Project, &Operation::Full)
                .unwrap()
                .unwrap()
                .scheduling,
            Scheduling::PageMajor
        );
    }

    #[test]
    fn old_settings_keep_standard_run_behavior() {
        let settings: WorkflowSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.enabled);
        let reloaded: WorkflowSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(reloaded.active_preset, settings.active_preset);
        assert_eq!(reloaded.presets, settings.presets);
    }
}
