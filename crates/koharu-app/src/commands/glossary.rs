use anyhow::Context as _;
use koharu_scene::{ProjectGlossary, ProjectId};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager as _, State, WebviewWindow};
use tauri_runtime_cef::CefRuntime;

use super::{
    ChannelExt as _, Error,
    canvas::CanvasChannel,
    processing::{JobKind, JobState, Processing},
    project::CurrentProject,
};

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct GlossaryDocument {
    pub project: ProjectId,
    pub glossary: ProjectGlossary,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_project_glossary(
    project: State<'_, CurrentProject>,
) -> Result<GlossaryDocument, Error> {
    let snapshot = project
        .project
        .lock()
        .await
        .as_ref()
        .context("no project is open")?
        .snapshot();
    Ok(GlossaryDocument {
        project: snapshot.project_id(),
        glossary: snapshot.project_component()?.unwrap_or_default(),
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn save_project_glossary(
    handle: AppHandle<CefRuntime>,
    expected: GlossaryDocument,
    glossary: ProjectGlossary,
) -> Result<GlossaryDocument, Error> {
    glossary.validate_entries()?;
    if handle
        .state::<Processing>()
        .jobs
        .lock()
        .values()
        .any(|job| !matches!(job.kind, JobKind::Export) && job.state == JobState::Running)
    {
        return Err(anyhow::anyhow!(
            "edit the glossary during review or after stopping processing"
        )
        .into());
    }
    let (commit, page) = {
        let project = handle.state::<CurrentProject>();
        let mut project = project.project.lock().await;
        let project = project.as_mut().context("no project is open")?;
        let snapshot = project.snapshot();
        if snapshot.project_id() != expected.project {
            return Err(anyhow::anyhow!("the active project changed").into());
        }
        let current = snapshot
            .project_component::<ProjectGlossary>()?
            .unwrap_or_default();
        if current != expected.glossary {
            return Err(anyhow::anyhow!("the glossary changed; reload before saving").into());
        }
        let patch = snapshot
            .patch(|edit| edit.set_project(&glossary))?
            .with_label("Edit project glossary");
        let commit = project.commit_rebased(patch).await?;
        if let Some(commit) = &commit {
            project.record_commit(commit);
        }
        (commit, project.active_page())
    };
    if let Some(commit) = commit {
        let desktop = handle.state::<koharu_desktop::Desktop>();
        desktop.synchronize(&commit.snapshot, page, &commit).await?;
        handle
            .state::<CanvasChannel>()
            .channel
            .publish(desktop.canvas_state());
    }
    Ok(GlossaryDocument {
        project: expected.project,
        glossary,
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn import_project_glossary(
    handle: AppHandle<CefRuntime>,
    window: WebviewWindow<CefRuntime>,
    expected: GlossaryDocument,
) -> Result<Option<GlossaryDocument>, Error> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter("Glossary JSON", &["json"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    if tokio::fs::metadata(file.path()).await?.len() > 4 * 1024 * 1024 {
        return Err(anyhow::anyhow!("glossary JSON exceeds 4 MiB").into());
    }
    let imported: ProjectGlossary = serde_json::from_slice(&tokio::fs::read(file.path()).await?)?;
    imported.validate_entries()?;
    let mut merged = expected.glossary.clone();
    for entry in imported.entries {
        if let Some(existing) = merged.entries.iter().find(|existing| {
            koharu_scene::normalize_term(&existing.source)
                == koharu_scene::normalize_term(&entry.source)
        }) {
            if existing != &entry {
                return Err(anyhow::anyhow!("conflicting imported term: {}", entry.source).into());
            }
        } else {
            merged.entries.push(entry);
        }
    }
    let candidates = std::mem::take(&mut merged.candidates);
    merged.replace_candidates(candidates);
    Ok(Some(save_project_glossary(handle, expected, merged).await?))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn export_project_glossary(
    window: WebviewWindow<CefRuntime>,
    project: State<'_, CurrentProject>,
) -> Result<(), Error> {
    let document = get_project_glossary(project).await?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .add_filter("Glossary JSON", &["json"])
        .set_file_name("glossary.json")
        .save_file()
        .await
    else {
        return Ok(());
    };
    let bytes = serde_json::to_vec_pretty(&document.glossary)?;
    let path = file.path().to_owned();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        use std::io::Write as _;
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().context("file has no directory")?)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    })
    .await??;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn suggest_glossary_translations(
    handle: AppHandle<CefRuntime>,
    project: ProjectId,
    sources: Vec<String>,
    glossary: Vec<koharu_scene::GlossaryEntry>,
) -> Result<Vec<String>, Error> {
    ProjectGlossary {
        entries: glossary.clone(),
        ..Default::default()
    }
    .validate_entries()?;
    let current = get_project_glossary(handle.state::<CurrentProject>()).await?;
    if current.project != project {
        return Err(anyhow::anyhow!("the active project changed").into());
    }
    if handle
        .state::<Processing>()
        .jobs
        .lock()
        .values()
        .any(|job| !matches!(job.kind, JobKind::Export) && job.state == JobState::Running)
    {
        return Err(anyhow::anyhow!(
            "wait for glossary review or stop processing before translating candidates"
        )
        .into());
    }
    let result = handle
        .state::<koharu_pipeline::Pipeline>()
        .suggest_glossary_translations(sources, &glossary)
        .await?;
    if get_project_glossary(handle.state::<CurrentProject>())
        .await?
        .project
        != project
    {
        return Err(anyhow::anyhow!("the active project changed").into());
    }
    Ok(result)
}
