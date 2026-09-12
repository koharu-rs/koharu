pub(crate) mod agent;
pub(crate) mod canvas;
pub(crate) mod cbz;
pub(crate) mod editing;
pub(crate) mod fonts;
pub(crate) mod glossary;
pub(crate) mod import;
pub(crate) mod lifecycle;
pub(crate) mod output;
pub(crate) mod preferences;
pub(crate) mod processing;
pub(crate) mod project;
pub(crate) mod workflow;

use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tauri::ipc::{Channel, IpcResponse};

#[derive(Debug, Type)]
#[specta(transparent)]
pub(crate) struct Error(#[specta(type = String)] anyhow::Error);

impl<E> From<E> for Error
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{:#}", self.0))
    }
}

pub(crate) trait ChannelExt<T> {
    fn publish(&self, value: T);
}

impl<T: IpcResponse> ChannelExt<T> for Mutex<Option<Channel<T>>> {
    fn publish(&self, value: T) {
        let mut channel = self.lock();
        if channel
            .as_ref()
            .is_some_and(|channel| channel.send(value).is_err())
        {
            channel.take();
        }
    }
}

pub fn bindings() -> tauri_specta::Builder<tauri_runtime_cef::CefRuntime> {
    use tauri_specta::{Builder, ErrorHandlingMode, collect_commands};

    Builder::new()
        .commands(collect_commands![
            agent::get_agent_status,
            agent::login_agent,
            agent::logout_agent,
            agent::save_agent_config,
            agent::run_agent,
            agent::cancel_agent,
            lifecycle::subscribe,
            lifecycle::get_project,
            lifecycle::get_pages,
            lifecycle::get_page,
            lifecycle::list_projects,
            lifecycle::create_project,
            lifecycle::open_project,
            lifecycle::delete_project,
            lifecycle::close_project,
            lifecycle::import_pages,
            lifecycle::select_page,
            editing::rename_page,
            editing::delete_pages,
            editing::move_page,
            editing::set_source_text,
            editing::set_translation,
            editing::set_typography,
            editing::set_geometry,
            editing::set_visibility,
            editing::delete_layers,
            editing::move_layer,
            editing::undo,
            editing::redo,
            glossary::get_project_glossary,
            glossary::suggest_glossary_translations,
            glossary::save_project_glossary,
            glossary::import_project_glossary,
            glossary::export_project_glossary,
            workflow::get_workflow_settings,
            workflow::configure_project_workflow,
            workflow::get_workflow_presets,
            workflow::save_workflow_presets,
            workflow::start_workflow,
            workflow::resume_workflow,
            processing::process,
            processing::stop_job,
            output::export_pages,
            cbz::export_cbz,
            cbz::get_export_config,
            cbz::save_export_config,
            output::get_thumbnail,
            fonts::get_fonts,
            fonts::get_font_preview,
            preferences::save_preferences,
            preferences::get_preferences,
            preferences::get_translation_models,
            canvas::get_canvas_manifest,
            canvas::get_canvas_resource,
            canvas::prepare_canvas_page,
            canvas::get_canvas_page_manifest,
            canvas::get_canvas_page_resource,
            canvas::add_point_text,
            canvas::add_text_box,
            canvas::commit_paint,
            canvas::commit_erase,
            canvas::commit_transform,
            canvas::commit_inpaint,
        ])
        .disable_serde_phases()
        .error_handling(ErrorHandlingMode::Throw)
}
