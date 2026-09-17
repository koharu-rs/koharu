use anyhow::{Context as _, Result};
use tauri_runtime_cef::CefRuntime;

use crate::{
    cli::Cli,
    listen::{self, BindMode},
};

pub(crate) async fn run(cli: &Cli, context: tauri::Context<CefRuntime>) -> Result<()> {
    let bind_host = cli.host.as_deref().unwrap_or("127.0.0.1");
    let bind_port = cli.port.unwrap_or(4000);
    let mode = bind_mode(cli);
    let listener = listen::bind(bind_host, bind_port, mode).await?;
    let addr = listener
        .local_addr()
        .context("failed to read the bound headless address")?;

    koharu_app::configure_packaged_store(None)?;
    let host = koharu_app::Host::new().context("failed to construct the headless host")?;
    host.spawn_download_events();
    host.initialize(cli.cpu)
        .await
        .context("failed to initialize the headless runtime")?;

    tracing::info!("headless: open {} in a browser", listen::bound_url(addr));

    let router =
        koharu_app::http_router(host, crate::assets::load_frontend(&context)?.into_router());
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("headless HTTP server failed")?;
    Ok(())
}

pub(crate) fn bind_mode(cli: &Cli) -> BindMode {
    listen::bind_mode(cfg!(debug_assertions), cli.port.is_some())
}
