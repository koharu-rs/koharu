use anyhow::{Context as _, Result};
use tauri::{Manager as _, WebviewUrl, WindowEvent};
use tauri_runtime_cef::{Cef, CefRuntime};

use crate::host::Host;

pub fn http_router(host: Host, frontend: axum::Router) -> axum::Router {
    crate::commands::router()
        .with_state(host)
        .fallback_service(frontend)
}

pub fn run(
    context: tauri::Context<CefRuntime>,
    cpu: bool,
    listener: std::net::TcpListener,
    frontend: axum::Router,
) -> Result<()> {
    let cef = Cef::default();
    #[cfg(debug_assertions)]
    let cef = cef.remote_debugging(tauri_runtime_cef::RemoteDebugging::Port {
        port: 4000,
        allowed_origins: Vec::new(),
    });
    #[cfg(target_os = "linux")]
    let cef = cef
        .enable_features(["Vulkan", "VulkanFromANGLE"])
        .command_line_args([
            ("--enable-unsafe-webgpu", None),
            ("use-angle", Some("vulkan")),
            ("--ozone-platform", Some("x11")),
        ]);
    tauri::Builder::<CefRuntime>::new()
        .runtime(cef)
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .max_file_size(1_000_000)
                .clear_targets()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir { file_name: None },
                ))
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|handle, _, _| {
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |application| {
            #[cfg(all(target_os = "windows", not(debug_assertions)))]
            crate::host::configure_packaged_store(Some(
                application
                    .path()
                    .resource_dir()
                    .context("failed to locate Koharu's installation directory")?,
            ))?;

            let host = Host::new()?;
            listener.set_nonblocking(true)?;
            let addr = listener.local_addr()?;
            let tokio_listener = tokio::net::TcpListener::from_std(listener)?;
            host.install(application);
            host.spawn_download_events();

            let server_host = host.clone();
            let shutdown = host.server_shutdown();
            tauri::async_runtime::spawn(async move {
                if let Err(error) =
                    axum::serve(tokio_listener, crate::http_router(server_host, frontend))
                        .with_graceful_shutdown(async move {
                            shutdown.notified().await;
                        })
                        .await
                {
                    tracing::error!(%error, "local HTTP server stopped");
                }
            });

            let mut window_config = application
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .context("the main Tauri window configuration is unavailable")?
                .clone();
            let url: url::Url = format!("http://{addr}/")
                .parse()
                .context("invalid local HTTP origin")?;
            window_config.url = WebviewUrl::External(url);
            let window = tauri::WebviewWindowBuilder::from_config(application, &window_config)?
                .build()
                .context("failed to create the main window")?;
            host.set_window(window.clone());
            window.show().context("failed to show the main window")?;
            window
                .set_focus()
                .context("failed to focus the main window")?;
            let initialization_host = host.clone();
            drop(tauri::async_runtime::spawn(async move {
                initialization_host
                    .initialize(cpu)
                    .await
                    .expect("failed to initialize the desktop runtime");
            }));

            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(
                event,
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
            ) {
                window.state::<Host>().shutdown();
            }
            if matches!(event, WindowEvent::Destroyed) {
                tracing::info!(
                    target: "koharu_metrics",
                    metric = "app_closed",
                    phase = "shutdown",
                );
                koharu_metrics::shutdown();
            }
        })
        .run(context)?;
    Ok(())
}
