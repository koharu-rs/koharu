use super::*;

#[test]
fn configuration_ignores_unknown_fields() {
    let config = toml::from_str::<PipelineConfig>("legacy_limit = 1").unwrap();
    assert_eq!(config, PipelineConfig::default());
}

#[tokio::test]
async fn stop_is_a_successful_partial_result() {
    let pipeline = pipeline(Default::default());
    let stop = StopToken::default();
    stop.stop();
    let request = Request {
        stop,
        ..Request::default()
    };
    let mut committer = RejectCommitter;

    let report = pipeline
        .execute(
            koharu_scene::Session::memory().await.unwrap().snapshot(),
            request,
            &mut committer,
        )
        .await
        .unwrap();

    assert_eq!(report.status, RunStatus::Stopped);
    assert_eq!(report.completed, 0);
}

#[tokio::test]
async fn stop_after_a_page_keeps_completed_progress() {
    let translation = TranslationConfig {
        model: koharu_translator::ModelSelection {
            provider: koharu_translator::Provider::OpenAi,
            model: Some("gpt-5.6-luna".to_owned()),
            quantization: None,
            vision: true,
            reasoning: true,
        },
        ..Default::default()
    };
    let pipeline = pipeline(translation);
    let mut session = koharu_scene::Session::memory().await.unwrap();
    let patch = session
        .snapshot()
        .patch(|edit| {
            edit.add_page(
                koharu_scene::PageDraft::new("one", 1.0, 1.0),
                koharu_scene::At::End,
            )?;
            edit.add_page(
                koharu_scene::PageDraft::new("two", 1.0, 1.0),
                koharu_scene::At::End,
            )?;
            Ok(())
        })
        .unwrap();
    session.commit(patch).await.unwrap();
    let stop = StopToken::default();
    let progress_stop = stop.clone();
    let request = Request {
        operation: Operation::Only {
            stage: Stage::Translation,
        },
        stop,
        progress: Some(std::sync::Arc::new(move |event| {
            if matches!(event, Progress::Skipped { .. }) {
                progress_stop.stop();
            }
        })),
        ..Request::default()
    };
    let mut committer = RejectCommitter;

    let report = pipeline
        .execute(session.snapshot(), request, &mut committer)
        .await
        .unwrap();

    assert_eq!(report.status, RunStatus::Stopped);
    assert_eq!(report.completed, 1);
    assert_eq!(report.total, 2);
}

struct RejectCommitter;

fn pipeline(translation: TranslationConfig) -> Pipeline {
    let config = PipelineConfig {
        translation,
        ..PipelineConfig::default()
    };
    Pipeline::from_config(
        koharu_config::Config::memory(config),
        koharu_config::Config::memory(koharu_translator::ProvidersConfig::default()),
        koharu_ml::Device::cpu(),
    )
    .unwrap()
}

#[async_trait::async_trait]
impl Committer for RejectCommitter {
    async fn commit(&mut self, _output: StageOutput) -> anyhow::Result<koharu_scene::Snapshot> {
        anyhow::bail!("stopped execution must not commit")
    }
}

#[test]
fn operations_expand_to_the_supported_workflows() {
    assert_eq!(
        Operation::Through {
            stage: Stage::Translation,
        }
        .stages()
        .unwrap(),
        vec![Stage::Detection, Stage::Ocr, Stage::Translation],
    );
    assert_eq!(
        Operation::Through {
            stage: Stage::Inpainting,
        }
        .stages()
        .unwrap(),
        vec![Stage::Detection, Stage::Inpainting],
    );
    assert_eq!(
        Operation::Only {
            stage: Stage::Translation,
        }
        .stages()
        .unwrap(),
        vec![Stage::Translation],
    );
    assert_eq!(
        Operation::Stages {
            stages: vec![Stage::Translation, Stage::Detection, Stage::Translation],
        }
        .stages()
        .unwrap(),
        vec![Stage::Detection, Stage::Translation],
    );
}

async fn project_with_ocr() -> koharu_scene::Session {
    use koharu_scene::*;
    let mut session = Session::memory().await.unwrap();
    let mut edit = session.snapshot().edit();
    for index in 0..20 {
        let page = edit
            .add_page(PageDraft::new(format!("page {index}"), 8.0, 8.0), At::End)
            .unwrap();
        edit.add_analysis_region::<TextRegion>(
            page,
            At::End,
            &Geometry::rectangle(0.0, 0.0, 8.0, 8.0),
            None,
        )
        .unwrap();
        let content = edit.add_text_content(page, At::End).unwrap();
        edit.set(
            content,
            &SourceText {
                text: Authored::user(if index < 12 { "高橋" } else { "ｴｰﾃﾙ" }.into()),
                language: Some(LanguageTag::new("ja").unwrap()),
            },
        )
        .unwrap();
        edit.add_text_layer(
            page,
            At::End,
            content,
            &TextLayout {
                origin: Origin::User,
                kind: TextLayoutKind::Paragraph,
            },
        )
        .unwrap();
    }
    session.commit(edit.finish().unwrap()).await.unwrap();
    session
}

async fn confirm_project_terms(session: &mut koharu_scene::Session) {
    use koharu_scene::*;
    let snapshot = session.snapshot();
    let pages = snapshot.pages().map(|page| page.id()).collect::<Vec<_>>();
    let candidates = analyze_terminology(&snapshot, &pages, &StopToken::default(), |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .find(|term| term.source == "高橋")
            .map(|term| (term.occurrences, term.page_count)),
        Some((12, 12))
    );
    assert_eq!(
        candidates
            .iter()
            .find(|term| term.source == "エーテル")
            .map(|term| (term.occurrences, term.page_count)),
        Some((8, 8))
    );
    let glossary = ProjectGlossary {
        entries: candidates
            .into_iter()
            .map(|candidate| GlossaryEntry {
                target: if candidate.source == "高橋" {
                    "高桥"
                } else {
                    "以太"
                }
                .into(),
                source: candidate.source,
                category: candidate.category,
                notes: String::new(),
                enabled: true,
            })
            .collect(),
        ..Default::default()
    };
    session
        .commit(snapshot.patch(|edit| edit.set_project(&glossary)).unwrap())
        .await
        .unwrap();
}

fn glossary_pipeline() -> Pipeline {
    pipeline(TranslationConfig {
        model: koharu_translator::ModelSelection {
            provider: koharu_translator::Provider::Caiyun,
            model: None,
            quantization: None,
            vision: false,
            reasoning: false,
        },
        ..Default::default()
    })
}

struct RecordingCommitter<'a> {
    session: &'a mut koharu_scene::Session,
    fail_after: Option<usize>,
    committed: usize,
}

#[async_trait::async_trait]
impl Committer for RecordingCommitter<'_> {
    async fn commit(&mut self, output: StageOutput) -> anyhow::Result<koharu_scene::Snapshot> {
        anyhow::ensure!(
            self.fail_after != Some(self.committed),
            "injected commit failure"
        );
        let commit = self.session.commit(output.patch).await?;
        self.committed += 1;
        Ok(commit.snapshot)
    }
}

#[tokio::test]
async fn twenty_page_ocr_corpus_drives_confirmed_translation_through_shared_provider() {
    let mut session = project_with_ocr().await;
    confirm_project_terms(&mut session).await;
    let snapshot = session.snapshot();
    let report = glossary_pipeline()
        .execute(
            snapshot,
            Request {
                operation: Operation::StageMajor {
                    stages: vec![Stage::Detection, Stage::Translation],
                },
                ..Default::default()
            },
            &mut RecordingCommitter {
                session: &mut session,
                fail_after: None,
                committed: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (report.status, report.completed, report.total),
        (RunStatus::Completed, 40, 40)
    );
    let snapshot = session.snapshot();
    let translated = snapshot
        .entities_with::<koharu_scene::Translation>()
        .unwrap();
    assert_eq!(translated.len(), 20);
    for entity in translated {
        let source = snapshot
            .component::<koharu_scene::SourceText>(entity.id())
            .unwrap()
            .unwrap();
        let translation = snapshot
            .component::<koharu_scene::Translation>(entity.id())
            .unwrap()
            .unwrap();
        assert_eq!(
            translation.text.value,
            if source.text.value == "高橋" {
                "高桥"
            } else {
                "以太"
            }
        );
    }
}

#[tokio::test]
async fn stage_major_cancel_preserves_ocr_and_completed_translation_and_can_run_again() {
    let mut session = project_with_ocr().await;
    confirm_project_terms(&mut session).await;
    let stop = StopToken::default();
    let progress_stop = stop.clone();
    let report = glossary_pipeline()
        .execute(
            session.snapshot(),
            Request {
                operation: Operation::StageMajor {
                    stages: vec![Stage::Detection, Stage::Translation, Stage::Inpainting],
                },
                stop,
                progress: Some(std::sync::Arc::new(move |event| {
                    if matches!(
                        event,
                        Progress::Finished {
                            stage: Stage::Translation,
                            ..
                        }
                    ) {
                        progress_stop.stop();
                    }
                    assert!(!matches!(
                        event,
                        Progress::Loading {
                            stage: Stage::Inpainting,
                            ..
                        }
                    ));
                })),
                ..Default::default()
            },
            &mut RecordingCommitter {
                session: &mut session,
                fail_after: None,
                committed: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(report.status, RunStatus::Stopped);
    assert_eq!(report.completed, 21);
    assert_eq!(
        session
            .snapshot()
            .entities_with::<koharu_scene::SourceText>()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(
        session
            .snapshot()
            .entities_with::<koharu_scene::Translation>()
            .unwrap()
            .len(),
        1
    );
    glossary_pipeline()
        .execute(
            session.snapshot(),
            Request {
                operation: Operation::StageMajor {
                    stages: vec![Stage::Translation],
                },
                ..Default::default()
            },
            &mut RecordingCommitter {
                session: &mut session,
                fail_after: None,
                committed: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        session
            .snapshot()
            .entities_with::<koharu_scene::Translation>()
            .unwrap()
            .len(),
        20
    );
}

#[tokio::test]
async fn stage_major_failure_stops_later_stages_without_rolling_back_completed_pages() {
    let mut session = project_with_ocr().await;
    confirm_project_terms(&mut session).await;
    let result = glossary_pipeline()
        .execute(
            session.snapshot(),
            Request {
                operation: Operation::StageMajor {
                    stages: vec![Stage::Detection, Stage::Translation, Stage::Inpainting],
                },
                progress: Some(std::sync::Arc::new(move |event| {
                    assert!(!matches!(
                        event,
                        Progress::Loading {
                            stage: Stage::Inpainting,
                            ..
                        }
                    ));
                })),
                ..Default::default()
            },
            &mut RecordingCommitter {
                session: &mut session,
                fail_after: Some(1),
                committed: 0,
            },
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        session
            .snapshot()
            .entities_with::<koharu_scene::SourceText>()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(
        session
            .snapshot()
            .entities_with::<koharu_scene::Translation>()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn glossary_suggestions_use_shared_configuration_and_bound_requests() {
    let pipeline = glossary_pipeline();
    let glossary = vec![koharu_scene::GlossaryEntry {
        source: "高橋".into(),
        target: "高桥".into(),
        category: koharu_scene::GlossaryCategory::Person,
        notes: String::new(),
        enabled: true,
    }];
    assert_eq!(
        pipeline
            .suggest_glossary_translations(vec!["高橋".into()], &glossary)
            .await
            .unwrap(),
        ["高桥"]
    );
    assert!(
        pipeline
            .suggest_glossary_translations(Vec::new(), &[])
            .await
            .is_err()
    );
    assert!(
        pipeline
            .suggest_glossary_translations(vec!["高橋".into(); 25], &[])
            .await
            .is_err()
    );
    assert!(
        pipeline
            .suggest_glossary_translations(vec!["a".repeat(1025)], &[])
            .await
            .is_err()
    );
    assert!(
        pipeline
            .suggest_glossary_translations(vec!["a".repeat(1024); 5], &[])
            .await
            .is_err()
    );
}
