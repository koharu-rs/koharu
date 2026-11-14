//! The Koharu app: one window whose React interface renders off-screen in
//! CEF over the native canvas, and the commands the interface calls.

mod app;
mod commands;
mod events;
mod library;
mod runs;
mod screen;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use koharu_pipeline::{Pipeline, PipelineConfig};
use koharu_renderer::{Renderer, RendererConfig};
use sentry::protocol::GpuContext;
use tauri::Manager as _;
use tauri_plugin_log::{Target, TargetKind, log::LevelFilter};
use tauri_plugin_window_state::StateFlags;
use tauri_runtime_cef::{Cef, CefRuntime};

use crate::{app::App, library::Library};
pub use crate::{commands::bindings, events::bindings as event_bindings};

pub fn run(context: tauri::Context<CefRuntime>) -> Result<()> {
    // One Tokio runtime: async commands run on the one the pipeline's
    // scheduler runs on.
    tauri::async_runtime::set(tokio::runtime::Handle::current());

    let library = Library::new(
        dirs::document_dir()
            .context("the user's Documents folder is unknown")?
            .join("Koharu"),
    )?;

    let renderer = Renderer::new(koharu_config::get::<RendererConfig>()?);
    // The window composites with it from its first frame.
    let gpu = renderer.gpu()?;
    let adapter = gpu.adapter.get_info();
    koharu_sentry::set_context(
        "gpu",
        GpuContext {
            name: adapter.name,
            id: Some(format!("{:#06x}", adapter.device)),
            vendor_id: Some(format!("{:#06x}", adapter.vendor)),
            api_type: Some(adapter.backend.to_string()),
            driver_version: Some(adapter.driver_info).filter(|version| !version.is_empty()),
            ..Default::default()
        },
    );

    let pipeline = Pipeline::new(koharu_config::get::<PipelineConfig>()?);
    let app = Arc::new(App::new(library, renderer, gpu, pipeline));
    koharu_sentry::set_screenshot({
        let screen = Arc::clone(&app.screen);
        move || screen.snapshot()
    });

    let cef = Cef::default()
        // Lets the main webview render off-screen; a process-wide setting.
        .with_settings(|settings| settings.windowless_rendering_enabled = 1)
        .on_window_event({
            let app = Arc::clone(&app);
            move |label, _, window, event| screen::on_window_event(&app, label, window, event)
        });
    #[cfg(debug_assertions)]
    let cef = cef
        .remote_debugging(tauri_runtime_cef::RemoteDebugging::Port {
            port: 4000,
            allowed_origins: Vec::new(),
        })
        // A profile apart from the installed app's, which pins remote
        // debugging off in its local state on every launch.
        .root_cache_path(
            dirs::cache_dir()
                .context("the user's cache directory is unknown")?
                .join(&context.config().identifier)
                .join("cef-debug"),
        );

    tauri::Builder::<CefRuntime>::new()
        .runtime(cef)
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(LevelFilter::Info)
                .max_file_size(1_000_000)
                .targets([Target::new(TargetKind::LogDir { file_name: None })])
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_window(screen::MAIN) {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    StateFlags::SIZE
                        | StateFlags::POSITION
                        | StateFlags::MAXIMIZED
                        | StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(koharu_sentry::plugin())
        .on_web_content_process_terminate(koharu_sentry::web_content_process_terminated)
        .manage(app)
        .invoke_handler(commands::handler())
        .setup(|tauri| {
            tauri.state::<Arc<App>>().start(tauri.handle().clone());
            Ok(screen::open(tauri.handle())?)
        })
        .run(context)?;
    Ok(())
}
