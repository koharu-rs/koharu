use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use koharu_scene::{LanguageTag, Revision};

use super::ScanCandidate;
use crate::commands::processing::{JobPhase, JobState};

#[derive(Debug)]
struct ScanInput {
    revision: Revision,
    fingerprint: String,
    source_language: Option<LanguageTag>,
    target_language: Option<LanguageTag>,
    texts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanOutcome {
    Finished,
    Stopped,
}

impl ScanOutcome {
    const fn job_state(self) -> JobState {
        match self {
            Self::Finished => JobState::Finished,
            Self::Stopped => JobState::Stopped,
        }
    }
}

#[async_trait]
trait ScanRuntime {
    async fn run_ocr(&mut self, stop: &koharu_pipeline::StopToken) -> Result<ScanOutcome>;
    async fn read_sources(&mut self) -> Result<ScanInput>;
    async fn extract_terms(
        &mut self,
        input: &ScanInput,
        stop: &koharu_pipeline::StopToken,
    ) -> Result<Vec<ScanCandidate>>;
    async fn verify_and_commit(
        &mut self,
        input: ScanInput,
        candidates: Vec<ScanCandidate>,
    ) -> Result<()>;
}

async fn run_scan_workflow(
    runtime: &mut impl ScanRuntime,
    stop: &koharu_pipeline::StopToken,
    set_phase: impl Fn(JobPhase),
) -> Result<ScanOutcome> {
    set_phase(JobPhase::PreparingOcr);
    if runtime.run_ocr(stop).await? == ScanOutcome::Stopped || stop.stopped() {
        return Ok(ScanOutcome::Stopped);
    }

    let input = runtime.read_sources().await?;
    if input.texts.iter().all(|text| text.trim().is_empty()) {
        bail!("the project has no OCR text to scan");
    }

    set_phase(JobPhase::ExtractingTerms);
    let candidates = runtime.extract_terms(&input, stop).await?;
    if stop.stopped() {
        return Ok(ScanOutcome::Stopped);
    }
    runtime.verify_and_commit(input, candidates).await?;
    Ok(ScanOutcome::Finished)
}

#[derive(Clone, Debug)]
struct TermSource {
    id: koharu_scene::GlossaryEntryId,
    source: String,
}

struct TermTranslationInput {
    project_identity: crate::commands::project::ProjectIdentity,
    entries: Vec<TermSource>,
    target_language: koharu_translator::Language,
    model: koharu_translator::ModelSelection,
    generation: koharu_translator::GenerationConfig,
    stop: koharu_pipeline::StopToken,
}

#[async_trait]
trait TermTranslationRuntime {
    async fn translate(&mut self, input: &TermTranslationInput) -> Result<Vec<String>>;
    async fn commit(
        &mut self,
        input: TermTranslationInput,
        translations: Vec<String>,
    ) -> Result<()>;
}

async fn run_term_translation_workflow(
    runtime: &mut impl TermTranslationRuntime,
    input: TermTranslationInput,
) -> Result<ScanOutcome> {
    if input.stop.stopped() {
        return Ok(ScanOutcome::Stopped);
    }
    let translations = runtime.translate(&input).await?;
    if translations.len() != input.entries.len() {
        bail!(
            "term translation returned {} results for {} entries",
            translations.len(),
            input.entries.len()
        );
    }
    if input.stop.stopped() {
        return Ok(ScanOutcome::Stopped);
    }
    runtime.commit(input, translations).await?;
    Ok(ScanOutcome::Finished)
}

#[async_trait]
trait ScanPipelineRuntime {
    async fn execute_ocr(
        &mut self,
        snapshot: koharu_scene::Snapshot,
        request: koharu_pipeline::Request,
    ) -> Result<koharu_pipeline::RunStatus>;

    async fn extract_terms(
        &mut self,
        text: &str,
        confidence_threshold: f32,
        stop: &koharu_pipeline::StopToken,
    ) -> Result<Option<Vec<koharu_ml::glossary_ner::GlossaryEntity>>>;
}

struct ProductionScanPipeline {
    pipeline: crate::host::Pipeline,
    project: crate::commands::project::CurrentProject,
    desktop: koharu_desktop::Desktop,
    canvas: crate::commands::canvas::CanvasChannel,
}

#[async_trait]
impl ScanPipelineRuntime for ProductionScanPipeline {
    async fn execute_ocr(
        &mut self,
        snapshot: koharu_scene::Snapshot,
        request: koharu_pipeline::Request,
    ) -> Result<koharu_pipeline::RunStatus> {
        let mut committer = crate::commands::processing::ProjectCommitter {
            project: self.project.clone(),
            desktop: self.desktop.clone(),
            canvas: self.canvas.clone(),
        };
        self.pipeline
            .execute(snapshot, request, &mut committer)
            .await
            .map(|report| report.status)
            .map_err(|error| anyhow::anyhow!(error))
    }

    async fn extract_terms(
        &mut self,
        text: &str,
        confidence_threshold: f32,
        stop: &koharu_pipeline::StopToken,
    ) -> Result<Option<Vec<koharu_ml::glossary_ner::GlossaryEntity>>> {
        self.pipeline
            .extract_glossary_terms(text, confidence_threshold, stop)
            .await
    }
}

struct AppScanRuntime<P> {
    pipeline: P,
    project: crate::commands::project::CurrentProject,
    project_identity: crate::commands::project::ProjectIdentity,
    processing: crate::commands::processing::Processing,
    jobs: crate::commands::processing::JobChannel,
    job: crate::commands::processing::JobId,
    project_channel: crate::commands::lifecycle::ProjectChannel,
}

#[async_trait]
impl<P: ScanPipelineRuntime + Send> ScanRuntime for AppScanRuntime<P> {
    async fn run_ocr(&mut self, stop: &koharu_pipeline::StopToken) -> Result<ScanOutcome> {
        use std::sync::Arc;

        use koharu_pipeline::{Operation, Progress, RunStatus, Scope, Stage};
        use parking_lot::Mutex;

        let current = self.project.project.lock().await;
        let project = current.as_ref().context("no project is open")?;
        if project.identity() != self.project_identity {
            bail!("project changed during glossary scan");
        }
        let snapshot = project.snapshot();
        drop(current);
        let completed = Arc::new(Mutex::new((0_usize, 0_usize)));
        let progress_processing = self.processing.clone();
        let progress_jobs = self.jobs.clone();
        let progress_completed = completed.clone();
        let job = self.job;
        let request = koharu_pipeline::Request::new(
            Operation::Through { stage: Stage::Ocr },
            Scope::Project,
            stop.clone(),
            Arc::from([]),
        )
        .with_progress(Arc::new(move |event| {
            let update = match event {
                Progress::Started { pages, stages } => {
                    let mut completed = progress_completed.lock();
                    *completed = (0, pages.len().saturating_mul(stages.len()));
                    Some((completed.0, completed.1, None))
                }
                Progress::Loading { page, .. } => {
                    let completed = progress_completed.lock();
                    Some((completed.0, completed.1, Some(page)))
                }
                Progress::Finished { page, .. } | Progress::Skipped { page, .. } => {
                    let mut completed = progress_completed.lock();
                    completed.0 = completed.0.saturating_add(1).min(completed.1);
                    Some((completed.0, completed.1, Some(page)))
                }
                Progress::Running { .. } => None,
            };
            if let Some((completed, total, page)) = update {
                progress_processing.update_job(job, &progress_jobs, |job| {
                    job.completed = completed;
                    job.total = total;
                    job.page = page;
                });
            }
        }));
        let status = self.pipeline.execute_ocr(snapshot, request).await?;
        Ok(if status == RunStatus::Stopped {
            ScanOutcome::Stopped
        } else {
            ScanOutcome::Finished
        })
    }

    async fn read_sources(&mut self) -> Result<ScanInput> {
        use koharu_scene::SourceText;

        let current = self.project.project.lock().await;
        let project = current.as_ref().context("no project is open")?;
        if project.identity() != self.project_identity {
            bail!("project changed during glossary scan");
        }
        let snapshot = project.snapshot();
        drop(current);
        let mut texts = Vec::new();
        let mut source_language = None;
        let mut mixed_languages = false;
        for page in snapshot.pages() {
            for entity in snapshot.subtree(page.id())? {
                let Some(source) = snapshot.component::<SourceText>(entity.id())? else {
                    continue;
                };
                if source.text.value.trim().is_empty() {
                    continue;
                }
                if let Some(language) = source.language {
                    match source_language.as_ref() {
                        None => source_language = Some(language),
                        Some(current) if current != &language => mixed_languages = true,
                        Some(_) => {}
                    }
                }
                texts.push(source.text.value);
            }
        }
        if mixed_languages {
            source_language = None;
        }
        let glossary = super::glossary(&snapshot)?;
        let target_language = match glossary.target_language {
            Some(language) => Some(language),
            None => Some(LanguageTag::new(
                crate::commands::preferences::Preferences::load()?
                    .pipeline
                    .translation
                    .target_language
                    .tag(),
            )?),
        };
        Ok(ScanInput {
            revision: snapshot.revision(),
            fingerprint: super::ocr_source_fingerprint(&snapshot)?,
            source_language: source_language.or(glossary.source_language),
            target_language,
            texts,
        })
    }

    async fn extract_terms(
        &mut self,
        input: &ScanInput,
        stop: &koharu_pipeline::StopToken,
    ) -> Result<Vec<ScanCandidate>> {
        use koharu_ml::glossary_ner::GlossaryEntityKind;
        use koharu_scene::{GLOSSARY_EXAMPLE_MAX_CHARS, GlossaryKind};

        let mut candidates = Vec::new();
        for text in &input.texts {
            if stop.stopped() {
                break;
            }
            let example = text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(GLOSSARY_EXAMPLE_MAX_CHARS)
                .collect::<String>();
            let example = (!example.is_empty()).then_some(example);
            let Some(entities) = self.pipeline.extract_terms(text, 0.3, stop).await? else {
                break;
            };
            for entity in entities {
                let kind = match entity.kind {
                    GlossaryEntityKind::Person => GlossaryKind::Person,
                    GlossaryEntityKind::Place => GlossaryKind::Place,
                    GlossaryEntityKind::Organization => GlossaryKind::Organization,
                    GlossaryEntityKind::Item => GlossaryKind::Item,
                    GlossaryEntityKind::Ability => GlossaryKind::Ability,
                    GlossaryEntityKind::WorkSpecificTerm => GlossaryKind::Term,
                };
                candidates.push(ScanCandidate {
                    source: entity.surface,
                    kind,
                    confidence: entity.confidence,
                    example: example.clone(),
                });
            }
        }
        Ok(candidates)
    }

    async fn verify_and_commit(
        &mut self,
        input: ScanInput,
        candidates: Vec<ScanCandidate>,
    ) -> Result<()> {
        let info = {
            let mut current = self.project.project.lock().await;
            let project = current.as_mut().context("no project is open")?;
            if project.identity() != self.project_identity {
                bail!("project changed during glossary scan");
            }
            let snapshot = project.snapshot();
            if super::ocr_source_fingerprint(&snapshot)? != input.fingerprint {
                bail!("project OCR text changed during glossary extraction; rescan the glossary");
            }
            if snapshot.revision() != input.revision {
                bail!(
                    "glossary scan expected revision {}, but the project is at {}",
                    input.revision,
                    snapshot.revision()
                );
            }
            project
                .apply_glossary_scan(
                    input.revision,
                    input.source_language,
                    input.target_language,
                    candidates,
                )
                .await?;
            project.info()
        };
        use crate::commands::ChannelExt as _;
        self.project_channel.channel.publish(Some(info));
        Ok(())
    }
}

#[koharu_macros::command]
pub(crate) async fn translate_glossary_entries(
    expected_revision: Revision,
    ids: Option<Vec<koharu_scene::GlossaryEntryId>>,
    pipeline: crate::host::Pipeline,
    project: crate::commands::project::CurrentProject,
    processing: crate::commands::processing::Processing,
    jobs: crate::commands::processing::JobChannel,
    project_channel: crate::commands::lifecycle::ProjectChannel,
) -> std::result::Result<crate::commands::processing::JobId, crate::commands::Error> {
    use std::collections::HashSet;

    let (project_identity, entries, target_language) = {
        let current = project.project.lock().await;
        let project = current.as_ref().context("no project is open")?;
        let snapshot = project.snapshot();
        if snapshot.revision() != expected_revision {
            return Err(anyhow::anyhow!(
                "glossary translation expected revision {expected_revision}, but the project is at {}",
                snapshot.revision()
            )
            .into());
        }
        let glossary = super::glossary(&snapshot)?;
        let target = glossary
            .target_language
            .context("glossary target language is required before translating entries")?;
        let target_language = target
            .as_str()
            .parse::<koharu_translator::Language>()
            .with_context(|| format!("unsupported glossary target language {target}"))?;
        let selected = ids.map(|ids| ids.into_iter().collect::<HashSet<_>>());
        let entries = glossary
            .entries
            .into_iter()
            .filter(|entry| {
                entry.enabled
                    && entry.translation.is_none()
                    && selected.as_ref().is_none_or(|ids| ids.contains(&entry.id))
            })
            .map(|entry| TermSource {
                id: entry.id,
                source: entry.source,
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Err(
                anyhow::anyhow!("no enabled untranslated glossary entries were selected").into(),
            );
        }
        (project.identity(), entries, target_language)
    };
    let translation = crate::commands::preferences::Preferences::load()?
        .pipeline
        .translation;
    if !translation.model.provider.supports_system_prompt() {
        return Err(anyhow::anyhow!(
            "translation provider {} does not support glossary term translation",
            translation.model.provider
        )
        .into());
    }
    let (id, stop) = processing.start_job(
        crate::commands::processing::JobKind::GlossaryTranslation,
        JobPhase::TranslatingTerms,
        &jobs,
    )?;
    let input = TermTranslationInput {
        project_identity,
        entries,
        target_language,
        model: translation.model,
        generation: translation.generation,
        stop,
    };
    let finish_processing = processing.clone();
    let finish_jobs = jobs.clone();
    drop(tokio::spawn(async move {
        let mut runtime = AppTermTranslationRuntime {
            pipeline,
            project,
            project_channel,
        };
        match run_term_translation_workflow(&mut runtime, input).await {
            Ok(outcome) => {
                finish_processing.finish_job(id, outcome.job_state(), None, &finish_jobs);
            }
            Err(error) => {
                tracing::error!(%error, "glossary entry translation failed");
                finish_processing.finish_job(
                    id,
                    JobState::Failed,
                    Some(format!("{error:#}")),
                    &finish_jobs,
                );
            }
        }
    }));
    Ok(id)
}

struct AppTermTranslationRuntime {
    pipeline: crate::host::Pipeline,
    project: crate::commands::project::CurrentProject,
    project_channel: crate::commands::lifecycle::ProjectChannel,
}

#[async_trait]
impl TermTranslationRuntime for AppTermTranslationRuntime {
    async fn translate(&mut self, input: &TermTranslationInput) -> Result<Vec<String>> {
        let request = koharu_translator::TranslationRequest::new_term_translation(
            input.entries.iter().map(|entry| entry.source.clone()),
            input.target_language,
        );
        self.pipeline
            .translate_terms(&input.model, input.generation, request)
            .await
    }

    async fn commit(
        &mut self,
        input: TermTranslationInput,
        translations: Vec<String>,
    ) -> Result<()> {
        let info = {
            let mut current = self.project.project.lock().await;
            let project = current.as_mut().context("no project is open")?;
            if project.identity() != input.project_identity {
                bail!("project changed during glossary entry translation");
            }
            let revision = project.revision();
            let results = input
                .entries
                .into_iter()
                .zip(translations)
                .map(|(entry, translation)| super::TermTranslationResult {
                    id: entry.id,
                    source: entry.source,
                    translation,
                })
                .collect();
            project.apply_term_translations(revision, results).await?;
            project.info()
        };
        use crate::commands::ChannelExt as _;
        self.project_channel.channel.publish(Some(info));
        Ok(())
    }
}

#[koharu_macros::command]
pub(crate) async fn scan_glossary(
    pipeline: crate::host::Pipeline,
    project: crate::commands::project::CurrentProject,
    processing: crate::commands::processing::Processing,
    jobs: crate::commands::processing::JobChannel,
    desktop: koharu_desktop::Desktop,
    canvas: crate::commands::canvas::CanvasChannel,
    project_channel: crate::commands::lifecycle::ProjectChannel,
) -> std::result::Result<crate::commands::processing::JobId, crate::commands::Error> {
    let project_identity = {
        let current = project.project.lock().await;
        current.as_ref().context("no project is open")?.identity()
    };
    let (id, stop) = processing.start_job(
        crate::commands::processing::JobKind::GlossaryScan,
        JobPhase::PreparingOcr,
        &jobs,
    )?;
    let finish_processing = processing.clone();
    let finish_jobs = jobs.clone();
    drop(tokio::spawn(async move {
        let phase_processing = processing.clone();
        let phase_jobs = jobs.clone();
        let mut runtime = AppScanRuntime {
            pipeline: ProductionScanPipeline {
                pipeline,
                project: project.clone(),
                desktop,
                canvas,
            },
            project,
            project_identity,
            processing,
            jobs,
            job: id,
            project_channel,
        };
        let result = run_scan_workflow(&mut runtime, &stop, move |phase| {
            phase_processing.update_job(id, &phase_jobs, |job| job.phase = phase);
        })
        .await;
        match result {
            Ok(outcome) => {
                finish_processing.finish_job(id, outcome.job_state(), None, &finish_jobs);
            }
            Err(error) => {
                tracing::error!(%error, "glossary scan failed");
                finish_processing.finish_job(
                    id,
                    JobState::Failed,
                    Some(format!("{error:#}")),
                    &finish_jobs,
                );
            }
        }
    }));
    Ok(id)
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use async_trait::async_trait;
    use koharu_scene::{At, Authored, PageDraft, Session, SourceText};

    use super::{
        AppScanRuntime, AppTermTranslationRuntime, ScanOutcome, ScanPipelineRuntime,
        TermTranslationRuntime, run_scan_workflow,
    };
    use crate::commands::{
        glossary::ocr_source_fingerprint,
        processing::{JobKind, JobPhase, JobState},
        project::{CurrentProject, Project},
    };

    #[derive(Clone, Copy)]
    enum ScanScenario {
        Success,
        StopAfterOcr,
        StopDuringNer,
        FailOcr,
        NoText,
        FingerprintChange,
    }

    struct RecordingPipeline {
        project: CurrentProject,
        requests: std::sync::Arc<
            std::sync::Mutex<Vec<(koharu_pipeline::Operation, koharu_pipeline::Scope, usize)>>,
        >,
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        scenario: ScanScenario,
    }

    #[async_trait]
    impl ScanPipelineRuntime for RecordingPipeline {
        async fn execute_ocr(
            &mut self,
            _snapshot: koharu_scene::Snapshot,
            request: koharu_pipeline::Request,
        ) -> Result<koharu_pipeline::RunStatus> {
            self.requests.lock().unwrap().push((
                request.operation().clone(),
                request.scope().clone(),
                request.terminology().len(),
            ));
            if matches!(self.scenario, ScanScenario::FailOcr) {
                bail!("OCR failed");
            }
            if !matches!(self.scenario, ScanScenario::NoText) {
                set_only_source_text(&self.project, "アリス").await?;
                self.events.lock().unwrap().push("ocr commit".to_owned());
            }
            Ok(if matches!(self.scenario, ScanScenario::StopAfterOcr) {
                koharu_pipeline::RunStatus::Stopped
            } else {
                koharu_pipeline::RunStatus::Completed
            })
        }

        async fn extract_terms(
            &mut self,
            text: &str,
            _confidence_threshold: f32,
            stop: &koharu_pipeline::StopToken,
        ) -> Result<Option<Vec<koharu_ml::glossary_ner::GlossaryEntity>>> {
            self.events.lock().unwrap().push(format!("NER: {text}"));
            if matches!(self.scenario, ScanScenario::StopDuringNer) {
                stop.stop();
                return Ok(None);
            }
            if matches!(self.scenario, ScanScenario::FingerprintChange) {
                set_only_source_text(&self.project, "アリス改").await?;
            }
            Ok(Some(vec![koharu_ml::glossary_ner::GlossaryEntity {
                start: 0,
                end: text.len(),
                surface: text.to_owned(),
                kind: koharu_ml::glossary_ner::GlossaryEntityKind::Person,
                confidence: 0.9,
            }]))
        }
    }

    async fn set_only_source_text(project: &CurrentProject, text: &str) -> Result<()> {
        let mut current = project.project.lock().await;
        let project = current.as_mut().unwrap();
        let snapshot = project.snapshot();
        let page = snapshot.pages().next().unwrap().id();
        let existing = snapshot
            .subtree(page)?
            .find(|entity| {
                entity
                    .component::<SourceText>()
                    .is_ok_and(|value| value.is_some())
            })
            .map(|entity| entity.id());
        let patch = snapshot.patch(|edit| {
            let content = match existing {
                Some(content) => content,
                None => edit.add_text_content(page, At::End)?,
            };
            edit.set(
                content,
                &SourceText {
                    text: Authored::user(text.to_owned()),
                    language: Some(koharu_scene::LanguageTag::new("ja")?),
                },
            )
        })?;
        let commit = project.session.commit(patch).await?;
        project.record_commit(&commit);
        Ok(())
    }

    async fn scan_fixture(
        scenario: ScanScenario,
    ) -> Result<(
        AppScanRuntime<RecordingPipeline>,
        CurrentProject,
        koharu_pipeline::StopToken,
        std::sync::Arc<
            std::sync::Mutex<Vec<(koharu_pipeline::Operation, koharu_pipeline::Scope, usize)>>,
        >,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    )> {
        let mut session = Session::memory().await?;
        let patch = session.snapshot().patch(|edit| {
            edit.add_page(PageDraft::new("page", 100.0, 100.0), At::End)?;
            Ok(())
        })?;
        session.commit(patch).await?;
        let project = CurrentProject {
            project: std::sync::Arc::new(tokio::sync::Mutex::new(Some(Project::new(
                session,
                "fixture".to_owned(),
            )))),
        };
        let processing = crate::commands::processing::Processing::default();
        let jobs = crate::commands::processing::JobChannel::default();
        let (job, stop) =
            processing.start_job(JobKind::GlossaryScan, JobPhase::PreparingOcr, &jobs)?;
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let project_identity = project.project.lock().await.as_ref().unwrap().identity();
        let runtime = AppScanRuntime {
            pipeline: RecordingPipeline {
                project: project.clone(),
                requests: requests.clone(),
                events: events.clone(),
                scenario,
            },
            project: project.clone(),
            project_identity,
            processing,
            jobs,
            job,
            project_channel: crate::commands::lifecycle::ProjectChannel::default(),
        };
        Ok((runtime, project, stop, requests, events))
    }

    #[tokio::test]
    async fn app_scan_runtime_enforces_ocr_first_request_and_commits_glossary_once() {
        let (mut runtime, project, stop, requests, events) =
            scan_fixture(ScanScenario::Success).await.unwrap();
        let phases = std::sync::Mutex::new(Vec::new());

        let outcome = run_scan_workflow(&mut runtime, &stop, |phase| {
            phases.lock().unwrap().push(phase);
        })
        .await
        .unwrap();

        assert_eq!(outcome, ScanOutcome::Finished);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [(
                koharu_pipeline::Operation::Through {
                    stage: koharu_pipeline::Stage::Ocr,
                },
                koharu_pipeline::Scope::Project,
                0,
            )]
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["ocr commit", "NER: アリス"]
        );
        assert_eq!(
            phases.into_inner().unwrap(),
            [JobPhase::PreparingOcr, JobPhase::ExtractingTerms]
        );
        let current = project.project.lock().await;
        let project = current.as_ref().unwrap();
        let snapshot = project.snapshot();
        let glossary = snapshot
            .project_component::<koharu_scene::Glossary>()
            .unwrap()
            .unwrap();
        assert_eq!(glossary.entries.len(), 1);
        assert_eq!(glossary.entries[0].source, "アリス");
        assert_eq!(
            glossary.source_fingerprint.as_deref(),
            Some(ocr_source_fingerprint(&snapshot).unwrap().as_str())
        );
        assert_eq!(project.undo.len(), 2);
    }

    #[tokio::test]
    async fn app_scan_runtime_negative_paths_never_commit_glossary() {
        for scenario in [
            ScanScenario::StopAfterOcr,
            ScanScenario::StopDuringNer,
            ScanScenario::FailOcr,
            ScanScenario::NoText,
            ScanScenario::FingerprintChange,
        ] {
            let (mut runtime, project, stop, _, _) = scan_fixture(scenario).await.unwrap();
            let result = run_scan_workflow(&mut runtime, &stop, |_| {}).await;
            if matches!(
                scenario,
                ScanScenario::StopAfterOcr | ScanScenario::StopDuringNer
            ) {
                assert_eq!(result.unwrap(), ScanOutcome::Stopped);
            } else {
                assert!(result.is_err());
            }
            assert!(
                project
                    .project
                    .lock()
                    .await
                    .as_ref()
                    .unwrap()
                    .snapshot()
                    .project_component::<koharu_scene::Glossary>()
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn scan_outcomes_map_to_terminal_job_states() {
        assert_eq!(ScanOutcome::Finished.job_state(), JobState::Finished);
        assert_eq!(ScanOutcome::Stopped.job_state(), JobState::Stopped);
    }

    struct FakeTranslation {
        events: Vec<&'static str>,
        fail: bool,
        stop_after_translation: bool,
    }

    #[async_trait]
    impl super::TermTranslationRuntime for FakeTranslation {
        async fn translate(&mut self, input: &super::TermTranslationInput) -> Result<Vec<String>> {
            self.events.push("translation");
            if self.fail {
                bail!("translation failed");
            }
            if self.stop_after_translation {
                input.stop.stop();
            }
            Ok(input
                .entries
                .iter()
                .map(|entry| format!("translated {}", entry.source))
                .collect())
        }

        async fn commit(
            &mut self,
            _input: super::TermTranslationInput,
            _translations: Vec<String>,
        ) -> Result<()> {
            self.events.push("commit");
            Ok(())
        }
    }

    fn term_input() -> super::TermTranslationInput {
        super::TermTranslationInput {
            project_identity: crate::commands::project::ProjectIdentity::new(),
            entries: vec![super::TermSource {
                id: koharu_scene::GlossaryEntryId::new(),
                source: "アリス".to_owned(),
            }],
            target_language: koharu_translator::Language::English,
            model: koharu_translator::ModelSelection::default(),
            generation: koharu_translator::GenerationConfig::default(),
            stop: koharu_pipeline::StopToken::default(),
        }
    }

    #[tokio::test]
    async fn term_translation_commits_once_only_after_every_result() {
        let mut runtime = FakeTranslation {
            events: Vec::new(),
            fail: false,
            stop_after_translation: false,
        };
        let outcome = super::run_term_translation_workflow(&mut runtime, term_input())
            .await
            .unwrap();

        assert_eq!(outcome, ScanOutcome::Finished);
        assert_eq!(runtime.events, ["translation", "commit"]);
    }

    #[tokio::test]
    async fn term_translation_stop_and_failure_do_not_partially_commit() {
        for (fail, stop_after_translation) in [(true, false), (false, true)] {
            let mut runtime = FakeTranslation {
                events: Vec::new(),
                fail,
                stop_after_translation,
            };
            let result = super::run_term_translation_workflow(&mut runtime, term_input()).await;
            if stop_after_translation {
                assert_eq!(result.unwrap(), ScanOutcome::Stopped);
            } else {
                assert!(result.is_err());
            }
            assert!(!runtime.events.contains(&"commit"));
        }
    }

    #[tokio::test]
    async fn term_translation_does_not_commit_after_project_switch() {
        let entry = super::TermSource {
            id: koharu_scene::GlossaryEntryId::new(),
            source: "アリス".to_owned(),
        };
        let old_project = project_with_term(entry.clone()).await.unwrap();
        let current = CurrentProject {
            project: std::sync::Arc::new(tokio::sync::Mutex::new(Some(old_project))),
        };
        let input = super::TermTranslationInput {
            project_identity: current.project.lock().await.as_ref().unwrap().identity(),
            entries: vec![entry.clone()],
            target_language: koharu_translator::Language::English,
            model: koharu_translator::ModelSelection::default(),
            generation: koharu_translator::GenerationConfig::default(),
            stop: koharu_pipeline::StopToken::default(),
        };
        let replacement = project_with_term(entry).await.unwrap();
        *current.project.lock().await = Some(replacement);
        let mut runtime = AppTermTranslationRuntime {
            pipeline: crate::host::Pipeline::empty(),
            project: current.clone(),
            project_channel: crate::commands::lifecycle::ProjectChannel::default(),
        };

        let error = runtime
            .commit(input, vec!["Alice".to_owned()])
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "project changed during glossary entry translation"
        );
        let current = current.project.lock().await;
        let project = current.as_ref().unwrap();
        assert_eq!(project.revision(), koharu_scene::Revision::new(1));
        let glossary = project
            .snapshot()
            .project_component::<koharu_scene::Glossary>()
            .unwrap()
            .unwrap();
        assert_eq!(glossary.entries[0].translation, None);
    }

    async fn project_with_term(entry: super::TermSource) -> Result<Project> {
        let mut session = Session::memory().await?;
        let patch = session.snapshot().patch(|edit| {
            edit.set_project(&koharu_scene::Glossary {
                enabled: true,
                source_language: Some(koharu_scene::LanguageTag::new("ja")?),
                target_language: Some(koharu_scene::LanguageTag::new("en")?),
                source_fingerprint: None,
                entries: vec![koharu_scene::GlossaryEntry {
                    id: entry.id,
                    source: entry.source,
                    translation: None,
                    kind: koharu_scene::GlossaryKind::Person,
                    enabled: true,
                    confidence: None,
                    occurrence_count: 0,
                    examples: Vec::new(),
                    source_origin: koharu_scene::GlossaryValueOrigin::Detected,
                    translation_origin: None,
                    present_in_last_scan: true,
                }],
            })
        })?;
        session.commit(patch).await?;
        Ok(Project::new(session, "fixture".to_owned()))
    }
}
