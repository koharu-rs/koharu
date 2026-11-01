//! A run's plan: which page steps are ready, which model the lane runs next,
//! and what a failure takes with it. It holds no models and does no work, so
//! a test drives it without a device.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use crate::{Step, Steps};

/// Pages a run takes at a time.
pub(crate) const WINDOW: usize = 16;

/// A model passed over for this many batches runs next.
const PATIENCE: u32 = 3;

/// Where a step's work runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Place<M> {
    Lane(M),
    Cpu,
    Network,
}

/// One step on one page, by the page's place in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Job {
    pub(crate) step: Step,
    pub(crate) page: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Done,
    Skipped(String),
    Failed(String),
    /// The page changed between the step's read and its edit.
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    Waiting,
    Ready,
    Running,
    Done,
    Skipped(String),
    Failed(String),
}

impl Status {
    fn settled(&self) -> bool {
        matches!(self, Self::Done | Self::Skipped(_) | Self::Failed(_))
    }
}

pub(crate) struct Plan<M> {
    pages: usize,
    /// The first page of the window; the next starts when every step of this
    /// one has settled.
    start: usize,
    places: BTreeMap<Step, Place<M>>,
    status: BTreeMap<Job, Status>,
    /// Jobs already computed again after a stale edit.
    retried: BTreeSet<Job>,
    passed_over: BTreeMap<M, u32>,
}

impl<M: Clone + Ord> Plan<M> {
    pub(crate) fn new(pages: usize, steps: Steps, place: impl Fn(Step) -> Place<M>) -> Self {
        let places = steps.iter().map(|step| (step, place(step))).collect();
        let status = (0..pages)
            .flat_map(|page| steps.iter().map(move |step| Job { step, page }))
            .map(|job| (job, Status::Waiting))
            .collect();
        let mut plan = Self {
            pages,
            start: 0,
            places,
            status,
            retried: BTreeSet::new(),
            passed_over: BTreeMap::new(),
        };
        plan.settle();
        plan
    }

    /// The batch the lane runs next: every ready job of one model, at most
    /// `cap` of them. The model is the one passed over too often, else a
    /// loaded one, else the one with the earliest step, then page.
    pub(crate) fn next_batch(
        &mut self,
        loaded: impl Fn(&M) -> bool,
        cap: impl Fn(&M) -> usize,
    ) -> Option<(M, Vec<Job>)> {
        let mut ready: BTreeMap<M, Vec<Job>> = BTreeMap::new();
        for (job, status) in &self.status {
            if *status == Status::Ready
                && let Some(Place::Lane(model)) = self.places.get(&job.step)
            {
                ready.entry(model.clone()).or_default().push(*job);
            }
        }
        let model = ready
            .iter()
            .max_by_key(|(model, jobs)| {
                let first = jobs
                    .iter()
                    .min()
                    .expect("a model with no ready job isn't listed");
                (
                    self.passed_over.get(*model).copied().unwrap_or(0) >= PATIENCE,
                    loaded(model),
                    Reverse(*first),
                )
            })?
            .0
            .clone();
        for other in ready.keys().filter(|other| **other != model) {
            *self.passed_over.entry(other.clone()).or_default() += 1;
        }
        self.passed_over.remove(&model);
        let mut jobs = ready.remove(&model).unwrap_or_default();
        jobs.sort();
        jobs.truncate(cap(&model).max(1));
        for job in &jobs {
            self.status.insert(*job, Status::Running);
        }
        Some((model, jobs))
    }

    /// Every ready job that runs off the lane, on the CPU or the network.
    pub(crate) fn off_lane(&mut self) -> Vec<(Job, Place<M>)> {
        let jobs: Vec<_> = self
            .status
            .iter()
            .filter(|(job, status)| {
                **status == Status::Ready
                    && !matches!(self.places.get(&job.step), Some(Place::Lane(_)))
            })
            .map(|(job, _)| *job)
            .collect();
        jobs.into_iter()
            .map(|job| {
                self.status.insert(job, Status::Running);
                (job, self.places[&job.step].clone())
            })
            .collect()
    }

    /// Records how `job` ended; a first stale edit makes it ready again.
    /// Returns whether the job is settled.
    pub(crate) fn finish(&mut self, job: Job, outcome: Outcome) -> bool {
        let status = match outcome {
            Outcome::Done => Status::Done,
            Outcome::Skipped(reason) => Status::Skipped(reason),
            Outcome::Failed(reason) => Status::Failed(reason),
            Outcome::Stale if self.retried.insert(job) => Status::Ready,
            Outcome::Stale => Status::Failed("the page changed again while it ran".to_owned()),
        };
        let settled = status.settled();
        self.status.insert(job, status);
        self.settle();
        settled
    }

    /// The earliest job left for `model`, which eviction keeps the longest.
    pub(crate) fn next_use(&self, model: &M) -> Option<Job> {
        self.status
            .iter()
            .filter(|(job, status)| {
                !status.settled() && self.places.get(&job.step) == Some(&Place::Lane(model.clone()))
            })
            .map(|(job, _)| *job)
            .min()
    }

    pub(crate) fn finished(&self) -> bool {
        self.status.values().all(Status::settled)
    }

    /// How many of the run's page steps have settled, of how many.
    pub(crate) fn progress(&self) -> (usize, usize) {
        let settled = self
            .status
            .values()
            .filter(|status| status.settled())
            .count();
        (settled, self.status.len())
    }

    /// Every job that settled with a failure, and its reason.
    pub(crate) fn failures(&self) -> impl Iterator<Item = (Job, &str)> {
        self.status.iter().filter_map(|(job, status)| match status {
            Status::Failed(reason) => Some((*job, reason.as_str())),
            _ => None,
        })
    }

    /// Every job that skipped its page, and why.
    pub(crate) fn skips(&self) -> impl Iterator<Item = (Job, &str)> {
        self.status.iter().filter_map(|(job, status)| match status {
            Status::Skipped(reason) => Some((*job, reason.as_str())),
            _ => None,
        })
    }

    /// Makes the window's waiting jobs ready, or failed when a step they
    /// need failed, and moves to the next window once this one settles.
    fn settle(&mut self) {
        loop {
            let end = (self.start + WINDOW).min(self.pages);
            let waiting: Vec<Job> = self
                .status
                .iter()
                .filter(|(job, status)| {
                    (self.start..end).contains(&job.page) && **status == Status::Waiting
                })
                .map(|(job, _)| *job)
                .collect();
            for job in waiting {
                let needed = job.step.needs().and_then(|step| {
                    self.status.get(&Job {
                        step,
                        page: job.page,
                    })
                });
                let status = match needed {
                    None | Some(Status::Done | Status::Skipped(_)) => Status::Ready,
                    Some(Status::Failed(_)) => {
                        let step = job.step.needs().expect("needed is set");
                        Status::Failed(format!("it needs {step}, which failed"))
                    }
                    Some(Status::Waiting | Status::Ready | Status::Running) => continue,
                };
                self.status.insert(job, status);
            }
            let window_settled = self
                .status
                .iter()
                .filter(|(job, _)| (self.start..end).contains(&job.page))
                .all(|(_, status)| status.settled());
            if !window_settled || end == self.pages {
                return;
            }
            self.start = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use enumflags2::BitFlags;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Model {
        Detector,
        Reader,
        Inpainter,
    }

    fn place(step: Step) -> Place<Model> {
        match step {
            Step::Detection => Place::Lane(Model::Detector),
            Step::Ocr => Place::Lane(Model::Reader),
            Step::Translation => Place::Network,
            Step::Typesetting => Place::Cpu,
            Step::Inpainting => Place::Lane(Model::Inpainter),
        }
    }

    fn job(step: Step, page: usize) -> Job {
        Job { step, page }
    }

    #[test]
    fn a_run_goes_step_by_step_across_its_pages() {
        let mut plan = Plan::new(2, Step::Detection | Step::Ocr, place);
        let (model, jobs) = plan.next_batch(|_| false, |_| 16).unwrap();
        assert_eq!(model, Model::Detector);
        assert_eq!(jobs, [job(Step::Detection, 0), job(Step::Detection, 1)]);
        assert!(plan.next_batch(|_| false, |_| 16).is_none());
        plan.finish(jobs[0], Outcome::Done);
        plan.finish(jobs[1], Outcome::Done);
        let (model, jobs) = plan.next_batch(|_| false, |_| 16).unwrap();
        assert_eq!((model, jobs.len()), (Model::Reader, 2));
    }

    #[test]
    fn a_loaded_model_goes_first_until_another_waits_too_long() {
        let mut plan = Plan::new(8, Step::Detection | Step::Ocr | Step::Inpainting, place);
        // Every page's detection is done, so OCR and inpainting are ready.
        for page in 0..8 {
            plan.finish(job(Step::Detection, page), Outcome::Done);
        }
        let inpainter_loaded = |model: &Model| *model == Model::Inpainter;
        let mut order = Vec::new();
        for _ in 0..4 {
            let (model, jobs) = plan.next_batch(inpainter_loaded, |_| 2).unwrap();
            order.push(model);
            for job in jobs {
                plan.finish(job, Outcome::Done);
            }
        }
        assert_eq!(
            order,
            [
                Model::Inpainter,
                Model::Inpainter,
                Model::Inpainter,
                Model::Reader
            ]
        );
    }

    #[test]
    fn a_failure_fails_only_that_pages_dependents() {
        let mut plan = Plan::new(2, Steps::ALL, place);
        plan.finish(
            job(Step::Detection, 0),
            Outcome::Failed("no scan".to_owned()),
        );
        plan.finish(job(Step::Detection, 1), Outcome::Done);
        let failed: Vec<Step> = plan.failures().map(|(job, _)| job.step).collect();
        assert_eq!(
            failed,
            [
                Step::Detection,
                Step::Ocr,
                Step::Translation,
                Step::Typesetting,
                Step::Inpainting
            ]
        );
        assert!(plan.failures().all(|(job, _)| job.page == 0));
        let (_, jobs) = plan.next_batch(|_| false, |_| 16).unwrap();
        assert_eq!(jobs, [job(Step::Ocr, 1)]);
    }

    #[test]
    fn a_skipped_step_still_lets_its_dependents_run() {
        let mut plan = Plan::new(1, Step::Detection | Step::Ocr, place);
        plan.finish(
            job(Step::Detection, 0),
            Outcome::Skipped("detected".to_owned()),
        );
        assert_eq!(plan.next_batch(|_| false, |_| 16).unwrap().0, Model::Reader);
    }

    #[test]
    fn the_next_window_starts_when_this_one_settles() {
        let mut plan = Plan::new(WINDOW + 1, BitFlags::from(Step::Detection), place);
        let (_, jobs) = plan.next_batch(|_| false, |_| usize::MAX).unwrap();
        assert_eq!(jobs.len(), WINDOW);
        for job in &jobs[..WINDOW - 1] {
            plan.finish(*job, Outcome::Done);
        }
        assert!(plan.next_batch(|_| false, |_| usize::MAX).is_none());
        plan.finish(jobs[WINDOW - 1], Outcome::Done);
        let (_, jobs) = plan.next_batch(|_| false, |_| usize::MAX).unwrap();
        assert_eq!(jobs, [job(Step::Detection, WINDOW)]);
    }

    #[test]
    fn a_stale_edit_is_computed_again_once() {
        let mut plan = Plan::new(1, BitFlags::from(Step::Detection), place);
        let (_, jobs) = plan.next_batch(|_| false, |_| 16).unwrap();
        assert!(!plan.finish(jobs[0], Outcome::Stale));
        let (_, jobs) = plan.next_batch(|_| false, |_| 16).unwrap();
        assert!(plan.finish(jobs[0], Outcome::Stale));
        assert_eq!(plan.failures().count(), 1);
        assert!(plan.finished());
    }

    #[test]
    fn off_lane_steps_run_beside_the_lane() {
        let mut plan = Plan::new(1, Step::Translation | Step::Typesetting, place);
        assert!(plan.next_batch(|_| false, |_| 16).is_none());
        let off = plan.off_lane();
        assert_eq!(off, [(job(Step::Translation, 0), Place::Network)]);
        plan.finish(off[0].0, Outcome::Done);
        assert_eq!(plan.off_lane(), [(job(Step::Typesetting, 0), Place::Cpu)]);
    }

    #[test]
    fn next_use_names_the_earliest_job_left() {
        let mut plan = Plan::new(2, Step::Detection | Step::Inpainting, place);
        assert_eq!(
            plan.next_use(&Model::Inpainter),
            Some(job(Step::Inpainting, 0))
        );
        assert_eq!(plan.next_use(&Model::Reader), None);
        let (_, jobs) = plan.next_batch(|_| false, |_| 16).unwrap();
        for job in jobs {
            plan.finish(job, Outcome::Done);
        }
        assert_eq!(plan.next_use(&Model::Detector), None);
    }
}
