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

struct AppScanRuntime {
    pipeline: crate::host::Pipeline,
    project: crate::commands::project::CurrentProject,
    processing: crate::commands::processing::Processing,
    jobs: crate::commands::processing::JobChannel,
    job: crate::commands::processing::JobId,
    desktop: koharu_desktop::Desktop,
    canvas: crate::commands::canvas::CanvasChannel,
    project_channel: crate::commands::lifecycle::ProjectChannel,
}

#[async_trait]
impl ScanRuntime for AppScanRuntime {
    async fn run_ocr(&mut self, stop: &koharu_pipeline::StopToken) -> Result<ScanOutcome> {
        use std::sync::Arc;

        use koharu_pipeline::{Operation, Progress, RunStatus, Scope, Stage};
        use parking_lot::Mutex;

        let snapshot = self
            .project
            .project
            .lock()
            .await
            .as_ref()
            .context("no project is open")?
            .snapshot();
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
        let mut committer = crate::commands::processing::ProjectCommitter {
            project: self.project.clone(),
            desktop: self.desktop.clone(),
            canvas: self.canvas.clone(),
        };
        let report = self
            .pipeline
            .execute(snapshot, request, &mut committer)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
        Ok(if report.status == RunStatus::Stopped {
            ScanOutcome::Stopped
        } else {
            ScanOutcome::Finished
        })
    }

    async fn read_sources(&mut self) -> Result<ScanInput> {
        use koharu_scene::SourceText;

        let snapshot = self
            .project
            .project
            .lock()
            .await
            .as_ref()
            .context("no project is open")?
            .snapshot();
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
            for entity in self.pipeline.extract_glossary_terms(text, 0.3).await? {
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

    let (entries, target_language) = {
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
        (entries, target_language)
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
    {
        let current = project.project.lock().await;
        current.as_ref().context("no project is open")?;
    }
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
            pipeline,
            project,
            processing,
            jobs,
            job: id,
            desktop,
            canvas,
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
    use koharu_scene::Revision;

    use super::{ScanInput, ScanOutcome, ScanRuntime, run_scan_workflow};
    use crate::commands::{
        glossary::ScanCandidate,
        processing::{JobPhase, JobState},
    };

    struct FakeScan {
        events: Vec<&'static str>,
        texts: Vec<String>,
        stop_after_ocr: bool,
        stop_during_ner: bool,
        fail_ocr: bool,
        fingerprint_changed: bool,
    }

    impl FakeScan {
        fn success() -> Self {
            Self {
                events: Vec::new(),
                texts: vec!["アリス".to_owned()],
                stop_after_ocr: false,
                stop_during_ner: false,
                fail_ocr: false,
                fingerprint_changed: false,
            }
        }
    }

    #[async_trait]
    impl ScanRuntime for FakeScan {
        async fn run_ocr(&mut self, stop: &koharu_pipeline::StopToken) -> Result<ScanOutcome> {
            self.events.push("all OCR commits");
            if self.fail_ocr {
                bail!("OCR failed");
            }
            if self.stop_after_ocr {
                stop.stop();
                return Ok(ScanOutcome::Stopped);
            }
            Ok(ScanOutcome::Finished)
        }

        async fn read_sources(&mut self) -> Result<ScanInput> {
            self.events.push("fresh text");
            Ok(ScanInput {
                revision: Revision::new(4),
                fingerprint: "before".to_owned(),
                source_language: None,
                target_language: None,
                texts: self.texts.clone(),
            })
        }

        async fn extract_terms(
            &mut self,
            _input: &ScanInput,
            stop: &koharu_pipeline::StopToken,
        ) -> Result<Vec<ScanCandidate>> {
            self.events.push("NER");
            if self.stop_during_ner {
                stop.stop();
            }
            Ok(Vec::new())
        }

        async fn verify_and_commit(
            &mut self,
            _input: ScanInput,
            _candidates: Vec<ScanCandidate>,
        ) -> Result<()> {
            self.events.push("fingerprint/revision check");
            if self.fingerprint_changed {
                bail!("project OCR text changed during glossary extraction; rescan the glossary");
            }
            self.events.push("glossary commit");
            Ok(())
        }
    }

    #[tokio::test]
    async fn scan_workflow_enforces_the_ocr_barrier_before_one_glossary_commit() {
        let mut runtime = FakeScan::success();
        let stop = koharu_pipeline::StopToken::default();
        let phases = std::sync::Mutex::new(Vec::new());

        let outcome = run_scan_workflow(&mut runtime, &stop, |phase| {
            phases.lock().unwrap().push(phase);
        })
        .await
        .unwrap();

        assert_eq!(outcome, ScanOutcome::Finished);
        assert_eq!(
            runtime.events,
            [
                "all OCR commits",
                "fresh text",
                "NER",
                "fingerprint/revision check",
                "glossary commit"
            ]
        );
        assert_eq!(
            phases.into_inner().unwrap(),
            [JobPhase::PreparingOcr, JobPhase::ExtractingTerms]
        );
    }

    #[tokio::test]
    async fn scan_stop_failure_empty_text_and_fingerprint_change_never_commit() {
        let cases = [
            (true, false, false, false, vec!["text".to_owned()]),
            (false, true, false, false, vec!["text".to_owned()]),
            (false, false, true, false, vec!["text".to_owned()]),
            (false, false, false, false, Vec::new()),
            (false, false, false, true, vec!["text".to_owned()]),
        ];

        for (stop_after_ocr, stop_during_ner, fail_ocr, fingerprint_changed, texts) in cases {
            let mut runtime = FakeScan {
                events: Vec::new(),
                texts,
                stop_after_ocr,
                stop_during_ner,
                fail_ocr,
                fingerprint_changed,
            };
            let stop = koharu_pipeline::StopToken::default();
            let result = run_scan_workflow(&mut runtime, &stop, |_| {}).await;

            if stop_after_ocr || stop_during_ner {
                assert_eq!(result.unwrap(), ScanOutcome::Stopped);
            } else {
                assert!(result.is_err());
            }
            assert!(!runtime.events.contains(&"glossary commit"));
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
}
