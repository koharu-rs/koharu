use anyhow::Context as _;
use koharu_scene::{GlossaryEntryId, Revision};

use super::{
    GlossaryEntryDraft, GlossaryEntryPatch, GlossaryImportPreview, GlossaryImportStrategy,
    GlossaryView,
};
use crate::{
    commands::{ChannelExt as _, Error, lifecycle::ProjectChannel, project::CurrentProject},
    host::HttpFile,
};

#[koharu_macros::command]
pub(crate) async fn get_glossary(
    project: CurrentProject,
) -> std::result::Result<GlossaryView, Error> {
    let current = project.project.lock().await;
    let project = current.as_ref().context("no project is open")?;
    Ok(project.glossary_view()?)
}

#[koharu_macros::command]
pub(crate) async fn set_glossary_enabled(
    expected_revision: Revision,
    enabled: bool,
    project: CurrentProject,
    project_channel: ProjectChannel,
) -> std::result::Result<GlossaryView, Error> {
    let (view, info) = {
        let mut current = project.project.lock().await;
        let project = current.as_mut().context("no project is open")?;
        let view = project
            .set_glossary_enabled(expected_revision, enabled)
            .await?;
        (view, project.info())
    };
    project_channel.channel.publish(Some(info));
    Ok(view)
}

#[koharu_macros::command]
pub(crate) async fn add_glossary_entry(
    expected_revision: Revision,
    draft: GlossaryEntryDraft,
    project: CurrentProject,
    project_channel: ProjectChannel,
) -> std::result::Result<GlossaryView, Error> {
    let (view, info) = {
        let mut current = project.project.lock().await;
        let project = current.as_mut().context("no project is open")?;
        let view = project.add_glossary_entry(expected_revision, draft).await?;
        (view, project.info())
    };
    project_channel.channel.publish(Some(info));
    Ok(view)
}

#[koharu_macros::command]
pub(crate) async fn update_glossary_entry(
    expected_revision: Revision,
    id: GlossaryEntryId,
    patch: GlossaryEntryPatch,
    project: CurrentProject,
    project_channel: ProjectChannel,
) -> std::result::Result<GlossaryView, Error> {
    let (view, info) = {
        let mut current = project.project.lock().await;
        let project = current.as_mut().context("no project is open")?;
        let view = project
            .update_glossary_entry(expected_revision, id, patch)
            .await?;
        (view, project.info())
    };
    project_channel.channel.publish(Some(info));
    Ok(view)
}

#[koharu_macros::command]
pub(crate) async fn delete_glossary_entries(
    expected_revision: Revision,
    ids: Vec<GlossaryEntryId>,
    project: CurrentProject,
    project_channel: ProjectChannel,
) -> std::result::Result<GlossaryView, Error> {
    let (view, info) = {
        let mut current = project.project.lock().await;
        let project = current.as_mut().context("no project is open")?;
        let view = project
            .delete_glossary_entries(expected_revision, ids)
            .await?;
        (view, project.info())
    };
    project_channel.channel.publish(Some(info));
    Ok(view)
}

#[koharu_macros::command]
pub(crate) async fn preview_glossary_import(
    document: String,
    project: CurrentProject,
) -> std::result::Result<GlossaryImportPreview, Error> {
    let current = project.project.lock().await;
    let project = current.as_ref().context("no project is open")?;
    Ok(project.preview_glossary_import(&document)?)
}

#[koharu_macros::command]
pub(crate) async fn apply_glossary_import(
    expected_revision: Revision,
    document: String,
    strategy: GlossaryImportStrategy,
    confirm_language_mismatch: bool,
    project: CurrentProject,
    project_channel: ProjectChannel,
) -> std::result::Result<GlossaryView, Error> {
    let (view, info) = {
        let mut current = project.project.lock().await;
        let project = current.as_mut().context("no project is open")?;
        let view = project
            .apply_glossary_import(
                expected_revision,
                &document,
                strategy,
                confirm_language_mismatch,
            )
            .await?;
        (view, project.info())
    };
    project_channel.channel.publish(Some(info));
    Ok(view)
}

#[koharu_macros::command]
pub(crate) async fn export_glossary(
    project: CurrentProject,
) -> std::result::Result<HttpFile, Error> {
    let (name, json) = {
        let current = project.project.lock().await;
        let project = current.as_ref().context("no project is open")?;
        (project.name.clone(), project.export_glossary_json()?)
    };
    Ok(HttpFile {
        bytes: json.into_bytes(),
        content_type: "application/json",
        filename: Some(format!("{}.glossary.json", safe_project_filename(&name))),
    })
}

fn safe_project_filename(name: &str) -> String {
    let safe = name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | '"' | ':' | '*' | '?' | '<' | '>' | '|'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let safe = safe.trim_matches([' ', '.']);
    if safe.is_empty() {
        "project".to_owned()
    } else {
        safe.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::safe_project_filename;

    #[test]
    fn glossary_export_filename_is_safe() {
        assert_eq!(safe_project_filename("demo"), "demo");
        assert_eq!(safe_project_filename("../bad\r\nname"), "_bad__name");
        assert_eq!(safe_project_filename("..."), "project");
    }
}
