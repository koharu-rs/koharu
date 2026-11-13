#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod mallinfo;

use std::io::IsTerminal as _;

use clap::Parser as _;
use koharu::panic;
use koharu::sentry;
use tracing_subscriber::layer::SubscriberExt as _;

#[derive(clap::Parser)]
#[command(version, about)]
struct Cli {}

#[tauri_runtime_cef::cef_entry_point]
fn main() {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: This only requests the existing parent console. It does not allocate one.
        let _ = unsafe {
            windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            )
        };
    }

    // First, before the runtime and the arguments: see `sentry::initialize`.
    let _guard = sentry::initialize();
    let _cli = Cli::parse();
    panic::install();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::filter::EnvFilter::builder()
                    .with_default_directive(tracing::Level::INFO.into())
                    .from_env_lossy(),
            )
            .with(sentry::tracing_layer())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(std::io::stderr().is_terminal()),
            ),
    )
    .expect("failed to set the global tracing subscriber");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start the async runtime");
    let _runtime = runtime.enter();
    koharu_desktop::run(tauri::generate_context!()).expect("failed to run the desktop application");
}
