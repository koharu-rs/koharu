use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, Result, ensure};
use futures::{StreamExt as _, stream::FuturesUnordered};
use koharu_scene::{EntityId, Snapshot};

use crate::{
    Committer, ErrorKind, PipelineError, Progress, ProgressSink, Report, Request, RunStatus, Stage,
    StageOutput, StopToken,
    images::ImageCache,
    progress,
    resources::ResourceMonitor,
    scheduler::Scheduler,
    scope::NormalizedScope,
    stage_runner::{StageCompletion, StageJob, StageOutcome, StageRunner},
    stages::StageInput,
};

// Provider-backed stages pay network latency per page, so a handful of pages
// overlap to hide it without opening unbounded requests.
const MAX_OVERLAPPING_PAGES: usize = 4;

pub(crate) struct Execution<'a> {
    runner: Arc<StageRunner>,
    resources: Arc<ResourceMonitor>,
    committer: &'a mut dyn Committer,
    stop: StopToken,
    progress: Option<ProgressSink>,
    scope: NormalizedScope,
    scheduler: Scheduler,
    scene: Snapshot,
    images: BTreeMap<EntityId, Arc<ImageCache>>,
    running: BTreeMap<Stage, usize>,
    completed: usize,
    failure: Option<PipelineError>,
    base: koharu_scene::Revision,
    started: Instant,
    inpainting_mask: Option<crate::InpaintingMask>,
}

impl<'a> Execution<'a> {
    pub(crate) fn new(
        runner: Arc<StageRunner>,
        resources: Arc<ResourceMonitor>,
        snapshot: Snapshot,
        request: Request,
        committer: &'a mut dyn Committer,
    ) -> std::result::Result<Self, PipelineError> {
        let started = Instant::now();
        let base = snapshot.revision();
        let stages = request
            .operation
            .stages()
            .map_err(|error| PipelineError::new(ErrorKind::InvalidInput, None, error))?;
        let scope = NormalizedScope::new(&snapshot, &request.scope, &stages)
            .map_err(|error| PipelineError::new(ErrorKind::InvalidInput, None, error))?;
        let pages = scope.pages().to_vec();
        if let Some(mask) = request.inpainting_mask.as_ref()
            && (!pages.contains(&mask.page) || !stages.contains(&Stage::Inpainting))
        {
            return Err(PipelineError::new(
                ErrorKind::InvalidInput,
                Some(Stage::Inpainting),
                anyhow::anyhow!("the inpainting mask page is outside the inpainting scope"),
            ));
        }
        progress::emit(
            request.progress.as_ref(),
            Progress::Started {
                pages: pages.clone(),
                stages: stages.clone(),
            },
        );

        Ok(Self {
            runner,
            resources,
            committer,
            stop: request.stop,
            progress: request.progress,
            scope,
            scheduler: if matches!(request.operation, crate::Operation::StageMajor { .. }) {
                Scheduler::new(&pages, &stages).stage_major()
            } else {
                Scheduler::new(&pages, &stages)
            },
            scene: snapshot,
            images: BTreeMap::new(),
            running: BTreeMap::new(),
            completed: 0,
            failure: None,
            base,
            started,
            inpainting_mask: request.inpainting_mask,
        })
    }

    pub(crate) async fn run(mut self) -> std::result::Result<Report, PipelineError> {
        if self.stopped() {
            return Ok(self.report(RunStatus::Stopped));
        }

        self.resources.start();
        self.resources.wait_for_sample().await;

        let runner = self.runner.clone();
        let mut running = FuturesUnordered::new();
        loop {
            while let Some(job) = self.take_ready_job() {
                running.push(runner.run(job));
            }

            let Some(completion) = running.next().await else {
                break;
            };
            if let Some(count) = self.running.get_mut(&completion.stage) {
                *count = count.saturating_sub(1);
            }
            if self.stopped() || self.failure.is_some() {
                continue;
            }
            if let Err(error) = self.apply_completion(completion).await {
                self.failure = Some(error);
            }
        }

        if self.scheduler.is_stage_major() {
            for stage in Stage::ALL {
                self.runner.unload(stage);
            }
        }
        self.finalize()
    }

    fn take_ready_job(&mut self) -> Option<StageJob> {
        if self.stopped() || self.failure.is_some() {
            return None;
        }
        let busy = self.saturated_stages();
        let (page, stage) = self.scheduler.start_next(&busy)?;
        *self.running.entry(stage).or_default() += 1;
        let images = self
            .images
            .entry(page)
            .or_insert_with(|| Arc::new(ImageCache::default()))
            .clone();
        Some(StageJob::new(
            stage,
            StageInput::new(
                self.scene.clone(),
                page,
                self.scope.entities(),
                self.scope.region(page),
                images,
                self.inpainting_mask
                    .as_ref()
                    .filter(|mask| stage == Stage::Inpainting && mask.page == page)
                    .cloned(),
            ),
            self.stop.clone(),
            self.progress.clone(),
        ))
    }

    fn saturated_stages(&self) -> BTreeSet<Stage> {
        saturated_stages(&self.running, |stage| self.page_capacity(stage))
    }

    // Provider-backed stages overlap pages in either scheduling mode; accelerator
    // models stay serialized per stage to keep one model resident at a time.
    fn page_capacity(&self, stage: Stage) -> usize {
        page_capacity(self.runner.concurrent(stage))
    }

    async fn apply_completion(
        &mut self,
        completion: StageCompletion,
    ) -> std::result::Result<(), PipelineError> {
        let StageCompletion {
            page,
            stage,
            model,
            elapsed,
            outcome,
        } = completion;
        match outcome? {
            StageOutcome::Stopped => {}
            StageOutcome::Skipped => {
                self.mark_complete(page, stage);
                progress::emit(self.progress.as_ref(), Progress::Skipped { page, stage });
            }
            StageOutcome::Patch(patch) => {
                if !self.commit_patch(page, stage, patch).await? {
                    return Ok(());
                }
                self.mark_complete(page, stage);
                progress::emit(
                    self.progress.as_ref(),
                    Progress::Finished {
                        page,
                        stage,
                        model,
                        elapsed,
                    },
                );
            }
        }
        Ok(())
    }

    async fn commit_patch(
        &mut self,
        page: EntityId,
        stage: Stage,
        patch: koharu_scene::Patch,
    ) -> std::result::Result<bool, PipelineError> {
        let patch = patch
            .rebase_on(&self.scene)
            .and_then(|patch| {
                patch.validate_on(&self.scene)?;
                Ok(patch.with_label(format!("Pipeline {stage} for page {page}")))
            })
            .context("failed to rebase stage output onto the latest scene")
            .map_err(|error| PipelineError::new(ErrorKind::InvalidOutput, Some(stage), error))?;
        if self.stopped() {
            return Ok(false);
        }

        let next = self
            .committer
            .commit(StageOutput { page, stage, patch })
            .await
            .with_context(|| format!("failed to commit {stage} output for page {page}"))
            .map_err(|error| PipelineError::new(ErrorKind::Commit, Some(stage), error))?;
        validate_commit(&self.scene, &next)
            .map_err(|error| PipelineError::new(ErrorKind::Commit, Some(stage), error))?;
        self.scene = next;
        Ok(true)
    }

    fn mark_complete(&mut self, page: EntityId, stage: Stage) {
        if self.scheduler.complete_stage(page, stage) || self.scheduler.is_stage_major() {
            self.images.remove(&page);
        }
        if self.scheduler.is_stage_major() && self.scheduler.stage_complete(stage) {
            self.runner.unload(stage);
        }
        self.completed += 1;
    }

    fn stopped(&self) -> bool {
        self.stop.stopped()
    }

    fn finalize(mut self) -> std::result::Result<Report, PipelineError> {
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        if !self.stopped() && self.completed != self.scheduler.total() {
            return Err(PipelineError::new(
                ErrorKind::InvalidOutput,
                None,
                anyhow::anyhow!(
                    "pipeline scheduler stopped after {} of {} work items",
                    self.completed,
                    self.scheduler.total()
                ),
            ));
        }
        let status = if self.stopped() {
            RunStatus::Stopped
        } else {
            RunStatus::Completed
        };
        Ok(self.report(status))
    }

    fn report(&self, status: RunStatus) -> Report {
        Report {
            status,
            base: self.base,
            final_revision: self.scene.revision(),
            completed: self.completed,
            total: self.scheduler.total(),
            elapsed: self.started.elapsed(),
        }
    }
}

fn validate_commit(previous: &Snapshot, next: &Snapshot) -> Result<()> {
    ensure!(
        previous.project_id() == next.project_id(),
        "committer returned a snapshot from another project"
    );
    ensure!(
        next.revision() > previous.revision(),
        "committer did not advance the scene revision"
    );
    Ok(())
}

const fn page_capacity(concurrent: bool) -> usize {
    if concurrent { MAX_OVERLAPPING_PAGES } else { 1 }
}

fn saturated_stages(
    running: &BTreeMap<Stage, usize>,
    capacity: impl Fn(Stage) -> usize,
) -> BTreeSet<Stage> {
    running
        .iter()
        .filter(|(stage, count)| **count >= capacity(**stage))
        .map(|(stage, _)| *stage)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_provider_stages_overlap_pages() {
        assert_eq!(page_capacity(true), MAX_OVERLAPPING_PAGES);
        assert_eq!(page_capacity(false), 1);
    }

    // Mirrors the execution loop: a stage stays dispatchable until it reaches
    // its page capacity.
    fn overlapping_translations(provider: bool) -> usize {
        let pages: Vec<EntityId> = (0..8).map(|_| EntityId::new()).collect();
        let mut scheduler = Scheduler::new(&pages, &Stage::ALL);
        let capacity = |stage: Stage| page_capacity(provider && stage == Stage::Translation);
        let mut in_flight: BTreeMap<Stage, usize> = BTreeMap::new();
        let mut translations = 0;
        while translations < MAX_OVERLAPPING_PAGES {
            let Some((page, stage)) = scheduler.start_next(&saturated_stages(&in_flight, capacity))
            else {
                break;
            };
            if stage == Stage::Translation {
                translations += 1;
                in_flight.insert(stage, translations);
                continue;
            }
            // Every other stage finishes at once, so it never backpressures.
            scheduler.complete_stage(page, stage);
        }
        translations
    }

    #[test]
    fn page_major_overlaps_provider_stages_but_serializes_local_models() {
        assert_eq!(overlapping_translations(true), MAX_OVERLAPPING_PAGES);
        assert_eq!(overlapping_translations(false), 1);
    }
}
