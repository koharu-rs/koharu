#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod mallinfo;

use clap::Parser as _;
use koharu::panic;
use koharu::sentry;
use koharu_app as app;
use tracing_subscriber::{Layer as _, filter::filter_fn, layer::SubscriberExt as _};

#[derive(clap::Parser)]
#[command(version, about)]
struct Cli {}

#[tokio::main]
#[tauri_runtime_cef::cef_entry_point]
async fn main() {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: This only requests the existing parent console. It does not allocate one.
        let _ = unsafe {
            windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            )
        };
    }

    let _cli = Cli::parse();
    let _guard = sentry::initialize();
    panic::install();
    let filter = filter_fn(|metadata| metadata.target() != "koharu_metrics");
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::filter::EnvFilter::builder()
                    .with_default_directive(tracing::Level::INFO.into())
                    .from_env_lossy(),
            )
            .with(sentry::tracing_layer().with_filter(filter.clone()))
            .with(koharu_metrics::layer())
            .with(koharu::tracing::TimingLayer::new().with_filter(filter)),
    )
    .expect("failed to set the global tracing subscriber");
    tokio::task::block_in_place(|| app::run(tauri::generate_context!()))
        .expect("failed to run the desktop application");
}
