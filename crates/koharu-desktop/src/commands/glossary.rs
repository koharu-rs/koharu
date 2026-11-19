use std::{collections::BTreeSet, sync::Arc};

use anyhow::Context as _;
use koharu_pipeline::{PipelineConfig, Request, RunId, Stage, Target};
use koharu_scene::{
    AliasSuggestion, Edit, GlossaryEntry, GlossaryKind, Op, ProjectId, TermIssue, TermLocation,
    TextSuggestion,
};
use serde::Serialize;
use specta::Type;
use tauri::{State, Window};
use tauri_runtime_cef::CefRuntime;

use super::Result;
use crate::app::App;

#[derive(Serialize, Type)]
pub struct GlossaryView {
    project: ProjectId,
    entries: Vec<GlossaryEntry>,
    issues: Vec<TermIssue>,
    lines: usize,
    suggestions: Vec<AliasSuggestion>,
    corrections: Vec<TextSuggestion>,
    inks: Vec<koharu_scene::InkSuggestion>,
    candidates: Option<Vec<TermCandidate>>,
}

#[tauri::command]
#[specta::specta]
pub fn get_glossary(app: State<'_, Arc<App>>) -> Result<GlossaryView> {
    let project = app.session()?.lock().project();
    let lines = project.transcript()?;
    let candidates = (!project.glossary_candidates().is_empty()).then(|| {
        let mut candidates: Vec<_> = project
            .glossary_candidates()
            .iter()
            .filter_map(|entries| {
                let first = entries.first()?;
                let active = project
                    .glossary()
                    .iter()
                    .find(|entry| entry.source == first.source);
                let matcher = entries
                    .iter()
                    .chain(active)
                    .find(|entry| entry.kind != GlossaryKind::PreserveBlock)
                    .unwrap_or(first);
                let locations: Vec<_> = lines
                    .iter()
                    .filter(|line| matcher.matches(&line.text))
                    .map(|line| TermLocation {
                        page: line.page,
                        number: line.number,
                        layer: line.layer,
                        text: line.text.clone(),
                    })
                    .collect();
                Some(TermCandidate {
                    entries: entries.clone(),
                    locations,
                })
            })
            .collect();
        candidates.sort_by_key(|entry| std::cmp::Reverse(entry.locations.len()));
        candidates
    });
    Ok(GlossaryView {
        project: project.id(),
        entries: project.glossary().to_vec(),
        issues: project.check_glossary(&koharu_config::get::<PipelineConfig>()?.glossary)?,
        lines: lines.len(),
        suggestions: project.alias_suggestions().to_vec(),
        corrections: project.text_suggestions().to_vec(),
        inks: project.ink_suggestions()?,
        candidates,
    })
}

#[tauri::command]
#[specta::specta]
pub fn extract_source(app: State<'_, Arc<App>>, project: ProjectId) -> Result<Vec<RunId>> {
    let session = app.session()?;
    let snapshot = session.lock().project();
    if snapshot.id() != project {
        return Err(anyhow::anyhow!("the open project changed").into());
    }
    let mut pages = Vec::new();
    let mut blocks = Vec::new();
    for page in snapshot
        .pages()
        .filter(|page| !page.excluded && page.background.is_some())
    {
        let found = snapshot.blocks(page.id)?;
        if found.is_empty() {
            pages.push(page.id);
        } else {
            blocks.extend(found.into_iter().filter_map(|(id, block)| {
                block.source.content.text().trim().is_empty().then_some(id)
            }));
        }
    }
    let mut queued = Vec::new();
    for (target, stages) in [
        (!pages.is_empty()).then_some((
            Target::Pages(pages),
            BTreeSet::from([Stage::Detect, Stage::Recognize]),
        )),
        (!blocks.is_empty())
            .then_some((Target::Blocks(blocks), BTreeSet::from([Stage::Recognize]))),
    ]
    .into_iter()
    .flatten()
    {
        queued.push(app.pipeline.run(
            Arc::clone(&session),
            Request {
                target,
                stages,
                resume: false,
                config: None,
            },
        )?);
    }
    Ok(queued)
}

#[tauri::command]
#[specta::specta]
pub fn save_glossary(
    app: State<'_, Arc<App>>,
    project: ProjectId,
    previous: Vec<GlossaryEntry>,
    entries: Vec<GlossaryEntry>,
) -> Result<()> {
    let opened = app.project.lock();
    let opened = opened.as_ref().context("no project is open")?;
    let mut session = opened.session.lock();
    if session.id() != project {
        return Err(anyhow::anyhow!("the open project changed").into());
    }
    session.apply(Edit {
        label: "edit_glossary".to_owned(),
        ops: vec![Op::SetGlossary { previous, entries }],
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn resolve_alias(
    app: State<'_, Arc<App>>,
    project: ProjectId,
    suggestion: AliasSuggestion,
    translation: Option<String>,
) -> Result<()> {
    resolve_review(
        &app,
        project,
        Op::ResolveAlias {
            suggestion,
            translation,
        },
    )
}

#[tauri::command]
#[specta::specta]
pub fn resolve_translation(
    app: State<'_, Arc<App>>,
    project: ProjectId,
    suggestion: TextSuggestion,
    translation: Option<String>,
) -> Result<()> {
    resolve_review(
        &app,
        project,
        Op::ResolveTranslation {
            suggestion,
            translation,
        },
    )
}

#[tauri::command]
#[specta::specta]
pub fn resolve_ink(
    app: State<'_, Arc<App>>,
    project: ProjectId,
    suggestion: koharu_scene::InkSuggestion,
    choice: koharu_scene::InkChoice,
) -> Result<()> {
    resolve_review(&app, project, Op::ResolveInk { suggestion, choice })
}

fn resolve_review(app: &App, project: ProjectId, op: Op) -> Result<()> {
    let opened = app.project.lock();
    let mut session = opened
        .as_ref()
        .context("no project is open")?
        .session
        .lock();
    if session.id() != project {
        return Err(anyhow::anyhow!("the open project changed").into());
    }
    session.apply(Edit {
        label: match op {
            Op::ResolveInk { .. } => "review_ink",
            Op::ResolveTranslation { .. } => "set_text",
            _ => "edit_glossary",
        }
        .to_owned(),
        ops: vec![op],
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn export_source(app: State<'_, Arc<App>>, window: Window<CefRuntime>) -> Result<()> {
    let session = app.session()?;
    let (project, suggested) = {
        let session = session.lock();
        let name = session
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        (session.project(), format!("{name}-source.json"))
    };
    let Some(path) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter("JSON", &["json"])
        .set_file_name(suggested)
        .save_file()
        .await
    else {
        return Ok(());
    };
    let file = path.path().to_owned();
    Ok(tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let lines: Vec<_> = project
            .transcript()?
            .into_iter()
            .map(|line| {
                serde_json::json!({
                    "page": line.page, "number": line.number, "layer": line.layer, "text": line.text
                })
            })
            .collect();
        fs_err::write(file, serde_json::to_vec_pretty(&lines)?)?;
        Ok(())
    })
    .await??)
}

#[derive(Serialize, Type)]
pub struct TermCandidate {
    entries: Vec<GlossaryEntry>,
    locations: Vec<TermLocation>,
}

#[tauri::command]
#[specta::specta]
pub fn analyze_glossary(app: State<'_, Arc<App>>, project: ProjectId) -> Result<RunId> {
    let session = app.session()?;
    if session.lock().id() != project {
        return Err(anyhow::anyhow!("the open project changed").into());
    }
    Ok(app.pipeline.run(
        session,
        Request {
            target: Target::Project,
            stages: BTreeSet::from([Stage::Glossary]),
            resume: false,
            config: None,
        },
    )?)
}
