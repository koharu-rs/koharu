use clap::Parser as _;
use tauri_runtime_cef::CefRuntime;
use tracing_subscriber::{Layer as _, filter::filter_fn, layer::SubscriberExt as _};

use crate::cli::Cli;

pub async fn run(context: tauri::Context<CefRuntime>) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    let cli = {
        // SAFETY: This only requests the existing parent console. It does not allocate one.
        let attached = unsafe {
            windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            )
        };
        let cli = Cli::parse();
        if attached.is_err() && (cli.headless || cli.debug) {
            // SAFETY: Allocates a console for headless/debug output when no parent console exists.
            let _ = unsafe { windows::Win32::System::Console::AllocConsole() };
        }
        cli
    };
    #[cfg(not(target_os = "windows"))]
    let cli = Cli::parse();

    let _guard = crate::sentry::initialize();
    crate::panic::install();
    let filter = filter_fn(|metadata| metadata.target() != "koharu_metrics");
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::filter::EnvFilter::builder()
                    .with_default_directive(cli.tracing_level().into())
                    .from_env_lossy(),
            )
            .with(crate::sentry::tracing_layer().with_filter(filter.clone()))
            .with(koharu_metrics::layer())
            .with(crate::tracing::TimingLayer::new().with_filter(filter)),
    )
    .expect("failed to set the global tracing subscriber");

    if cli.download {
        koharu_app::configure_packaged_store(None)?;
        koharu_app::prepare_runtime(cli.cpu).await?;
        return Ok(());
    }

    if cli.headless {
        return crate::server::run(&cli, context).await;
    }

    let frontend = crate::assets::load_frontend(&context)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let listener = listener.into_std()?;
    listener.set_nonblocking(true)?;
    tokio::task::block_in_place(|| {
        koharu_app::run(context, cli.cpu, listener, frontend.into_router())
    })
}
