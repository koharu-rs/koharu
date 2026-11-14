//! Shows a project's page on the canvas in a winit window, playing the
//! host's part and React's: it applies the canvas's edits, renders frames,
//! and picks tools from the keyboard. `--script` instead drives the canvas
//! offscreen through a fixed set of gestures, writes a snapshot, and prints
//! its draw times, as the canvas's done check.
//!
//! `cargo run -p koharu-canvas --example view -- pages.khr page1.png page2.png`
//!
//! Keys: V select, T text, B draw, E eraser, I colour picker, J remove,
//! H pan, Space held pans, F fits, Escape cancels, Ctrl+Z undoes and
//! Ctrl+Shift+Z redoes, Delete removes the selection.

mod host;
mod script;
mod window;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::{Edit, Hash, Op, Session};
use parking_lot::Mutex;
use tracing_subscriber::EnvFilter;
use winit::event_loop::EventLoop;

use crate::{host::Host, window::App};

#[derive(Debug, Parser)]
struct Cli {
    /// The project file, opened, or made of `images` when it doesn't exist.
    project: PathBuf,

    /// Image files as a new project's pages, in order.
    images: Vec<PathBuf>,

    /// The page to show, from 1.
    #[arg(long, default_value_t = 1)]
    page: usize,

    /// Run the scripted gestures offscreen and write the canvas to this PNG.
    #[arg(long)]
    script: Option<PathBuf>,

    /// With `--script`, first add 150 text layers, a dense translated page.
    #[arg(long, requires = "script")]
    dense: bool,
}

fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let cli = Cli::parse();
    let session = match (cli.project.exists(), cli.images.is_empty()) {
        (true, true) => Session::open(&cli.project)?,
        (true, false) => bail!("{} exists; it takes no images", cli.project.display()),
        (false, _) => create(&cli.project, &cli.images)?,
    };
    let page = session
        .project()
        .pages()
        .nth(cli.page.saturating_sub(1))
        .map(|page| page.id)
        .with_context(|| format!("the project has no page {}", cli.page))?;
    let renderer = Renderer::new(koharu_config::get::<RendererConfig>()?);
    let host = Host::new(renderer, Arc::new(Mutex::new(session)), page)?;
    match cli.script {
        Some(snapshot) => script::run(host, &snapshot, cli.dense),
        None => {
            let event_loop = EventLoop::new()?;
            event_loop.run_app(&mut App::new(host))?;
            Ok(())
        }
    }
}

/// A project at `path` with `images` as its pages, as the pipeline's example
/// imports them.
fn create(path: &Path, images: &[PathBuf]) -> Result<Session> {
    let mut session = Session::create(path)?;
    let mut ops = Vec::new();
    for (index, file) in images.iter().enumerate() {
        let bytes = fs_err::read(file)?;
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
