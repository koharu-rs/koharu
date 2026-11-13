//! Serves a project's agent tools over MCP, with no app: it opens the project,
//! or makes one of image files as its pages, starts the pipeline and the
//! renderer with their settings, and serves until Ctrl+C.
//!
//! `cargo run -p koharu-agent --example serve -- --port 7878 pages.khr page1.png page2.png`

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use koharu_agent::{McpServer, Tools};
use koharu_pipeline::{Pipeline, PipelineConfig};
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::{Edit, Hash, Op, Session};
use parking_lot::Mutex;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
struct Cli {
    /// The port on 127.0.0.1 to serve MCP at.
    #[arg(long, default_value_t = 7878)]
    port: u16,

    /// The project file, opened, or made of `pages` when it doesn't exist.
    project: PathBuf,

    /// Image files as a new project's pages, in order.
    pages: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let cli = Cli::parse();
    let session = match (cli.project.exists(), cli.pages.is_empty()) {
        (true, true) => Session::open(&cli.project)?,
        (true, false) => bail!("{} exists; it takes no pages", cli.project.display()),
        (false, _) => create(&cli.project, &cli.pages)?,
    };
    let session = Arc::new(Mutex::new(session));
    let pipeline = Pipeline::new(koharu_config::get::<PipelineConfig>()?);
    let renderer = Renderer::new(koharu_config::get::<RendererConfig>()?);
    let server = McpServer::start(cli.port).await?;
    server.open(Some(Tools::new(session, pipeline, renderer)));
    println!("http://{}/mcp", server.address());
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// A project at `path` with `pages` as its pages, as the pipeline's example
/// imports them.
fn create(path: &PathBuf, pages: &[PathBuf]) -> Result<Session> {
    let mut session = Session::create(path)?;
    let mut ops = Vec::new();
    for (index, file) in pages.iter().enumerate() {
        let bytes =
            std::fs::read(file).with_context(|| format!("couldn't read {}", file.display()))?;
        let background = Some(Hash::of(&bytes));
        ops.push(Op::AddFile { bytes });
        ops.push(Op::AddPage {
            id: None,
            index,
            size: None,
            background,
        });
    }
    session.apply(Edit {
        label: "Import".to_owned(),
        ops,
    })?;
    Ok(session)
}
