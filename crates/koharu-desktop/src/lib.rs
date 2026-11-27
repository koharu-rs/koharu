//! The Koharu app: one window whose React interface renders off-screen in
//! CEF over the native canvas, and the commands the interface calls.

mod app;
mod commands;
mod events;
mod gallery;
mod library;
mod screen;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use koharu_pipeline::{Pipeline, PipelineConfig};
use koharu_renderer::{Gpu, Renderer};
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

    // Beside the store in Koharu's root, and in the app's local data folder
    // otherwise, which is a per-user install's folder too. That is Tauri's
    // `app_local_data_dir`, which needs the app this builds.
    let data = match koharu_runtime::root() {
        Some(folder) => folder.to_path_buf(),
        None => dirs::data_local_dir()
            .context("the user's local data folder is unknown")?
            .join(&context.config().identifier),
    };
    let library = Library::new(data.join("projects"))?;

    // The process's, which the pipeline draws a painted area's page with.
    let renderer = Renderer::get().clone();
    // The window composites with it from its first frame.
    let gpu = Gpu::get()?;
    let adapter = gpu.adapter.get_info();
    // CEF hands its frames over as GPU textures where Koharu's GPU can take
    // them, and copies them through memory everywhere else.
    let sharing = screen::sharing(&gpu);
    let shared = sharing.is_some();
    tracing::info!(
        "CEF's frames: {} on {} ({})",
        if shared { "shared textures" } else { "copied" },
        adapter.name,
        adapter.backend
    );
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
    let app = Arc::new(App::new(library, renderer, pipeline));
    koharu_sentry::set_screenshot({
        let screen = app.screen.clone();
        move || screen.screenshot()
    });

    let cef = Cef::default()
        .with_settings(|settings| {
            // Lets the main webview render off-screen; a process-wide setting.
            settings.windowless_rendering_enabled = 1;
            // The macOS keychain item holding the cookies' key. Every Chromium
            // app shares the default one, and macOS asks for the user's
            // password when another app made it.
            settings.keychain_service_name = "Koharu Safe Storage".into();
            settings.keychain_account_name = "Koharu".into();
        })
        .on_window_event({
            let app = Arc::clone(&app);
            move |label, _, window, event| screen::on_window_event(&app, label, window, event)
        })
        // A second launch on this profile, which CEF ends before any plugin runs.
        .on_relaunch({
            let app = Arc::clone(&app);
            move || app.bring_forward()
        });
    // A shared texture opens only on the GPU that made it.
    let cef = match sharing.flatten() {
        Some(luid) => cef.command_line_arg("use-adapter-luid", Some(luid)),
        None => cef,
    };
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
        // Another build of Koharu, on another CEF profile.
        .plugin(tauri_plugin_single_instance::init(|tauri, _, _| {
            tauri.state::<Arc<App>>().bring_forward()
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Sign-in windows open anew each time.
                .with_filter(|label| label == screen::MAIN)
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
        .on_window_event(|window, event| {
            if window.label() != screen::MAIN {
                return;
            }
            if let tauri::WindowEvent::DragDrop(event) = event {
                let payload = match event {
                    tauri::DragDropEvent::Enter { position, .. } => {
                        let Ok(scale) = window.scale_factor() else {
                            return;
                        };
                        let pos = position.to_logical::<f64>(scale);
                        events::FileDrag::Enter { x: pos.x, y: pos.y }
                    }
                    tauri::DragDropEvent::Over { position } => {
                        let Ok(scale) = window.scale_factor() else {
                            return;
                        };
                        let pos = position.to_logical::<f64>(scale);
                        events::FileDrag::Over { x: pos.x, y: pos.y }
                    }
                    tauri::DragDropEvent::Drop { paths, position } => {
                        let Ok(scale) = window.scale_factor() else {
                            return;
                        };
                        let pos = position.to_logical::<f64>(scale);
                        let paths = paths
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect();
                        events::FileDrag::Drop {
                            x: pos.x,
                            y: pos.y,
                            paths,
                        }
                    }
                    tauri::DragDropEvent::Leave => events::FileDrag::Leave,
                    _ => return,
                };
                events::FILE_DRAG.tell(window.app_handle(), payload);
            }
        })
        .on_web_content_process_terminate(koharu_sentry::web_content_process_terminated)
        .manage(app)
        .invoke_handler(commands::handler())
        .setup(move |tauri| {
            tauri.manage(gallery::client(tauri.handle().clone())?);
            let app = tauri.state::<Arc<App>>();
            app.start(tauri.handle().clone());
            Ok(screen::open(tauri.handle(), &app.screen, gpu, shared)?)
        })
        .run(context)?;
    Ok(())
}
