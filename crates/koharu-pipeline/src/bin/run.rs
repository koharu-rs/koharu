//! Puts the pages in one project, runs every step on them with the
//! `[pipeline]` settings, and prints how long each page's step took.

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context as _, Result};
use clap::Parser;
use koharu_pipeline::{Host, Pipeline, Progress, Run, State, Step, Steps, Target};
use koharu_scene::{Author, Change, Edit, Hash, Op, PageId, Ref, Session, Size};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Parser)]
#[command(version, about = "Run Koharu's pipeline on pages")]
struct Arguments {
    #[arg(short, long, value_name = "PAGE", num_args = 1.., required = true)]
    input: Vec<PathBuf>,
}

struct Local {
    session: Mutex<Session>,
    started: Mutex<HashMap<(PageId, Step), Instant>>,
}

impl Host for Local {
    fn read<T>(&self, read: impl FnOnce(&Session) -> T) -> T {
        read(&self.session.lock().expect("the session lock"))
    }

    fn apply(&self, edit: Edit) -> koharu_scene::Result<Change> {
        self.session.lock().expect("the session lock").apply(edit)
    }

    fn report(&self, progress: Progress) {
        let mut started = self.started.lock().expect("the timing lock");
        let key = (progress.page, progress.step);
        let elapsed = || {
            started
                .get(&key)
                .map_or(0.0, |at| at.elapsed().as_secs_f64())
        };
        let (page, step, done, total) =
            (progress.page, progress.step, progress.done, progress.total);
        match progress.state {
            State::Started => {
                started.entry(key).or_insert_with(Instant::now);
            }
            State::Finished => eprintln!("[{done}/{total}] {step} on {page}: {:.2}s", elapsed()),
            State::Skipped(reason) => {
                eprintln!("[{done}/{total}] {step} on {page}: skipped, {reason}")
            }
            State::Failed(reason) => {
                eprintln!("[{done}/{total}] {step} on {page}: failed, {reason}")
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let dir = tempfile::tempdir()?;
    let mut session = Session::create(dir.path().join("run.koharu"))?;
    let mut pages = Vec::new();
    for (index, path) in arguments.input.iter().enumerate() {
        let bytes = fs::read(path).with_context(|| format!("couldn't read {}", path.display()))?;
        let image = image::load_from_memory(&bytes)
            .with_context(|| format!("couldn't decode {}", path.display()))?;
        let media_type = match image::guess_format(&bytes)? {
            image::ImageFormat::Jpeg => "image/jpeg",
            image::ImageFormat::WebP => "image/webp",
            _ => "image/png",
        };
        let file = Hash::of(&bytes);
        let label = |name: &str| Ref::try_from(format!("${name}")).expect("a valid label");
        let change = session.apply(Edit {
            project: session.id(),
            label: "Import".to_owned(),
            author: Author::User { via: None },
            reads: Vec::new(),
            ops: vec![
                Op::AddFile {
                    media_type: media_type.to_owned(),
                    bytes,
                },
                Op::AddPage {
                    id: Some(label("page")),
                    index,
                    size: Size {
                        width: image.width(),
                        height: image.height(),
                    },
                },
                Op::AddLayer {
                    id: None,
                    page: label("page"),
                    parent: None,
                    index: 0,
                    layer: serde_json::from_value(
                        serde_json::json!({"kind": "image", "name": "Scan", "file": file}),
                    )?,
                    ingredients: Vec::new(),
                },
            ],
        })?;
        pages.extend(change.pages.added);
    }

    let host = Arc::new(Local {
        session: Mutex::new(session),
        started: Mutex::new(HashMap::new()),
    });
    let pipeline = Pipeline::new(koharu_runtime::devices().await?);
    let cancel = CancellationToken::new();
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        }
    });
    let started = Instant::now();
    let report = pipeline
        .run(
            Arc::clone(&host),
            Run {
                target: Target::Pages(pages),
                steps: Steps::all(),
                redo: false,
                cancel,
            },
        )
        .await?;
    eprintln!(
        "{} of {} page steps in {:.2}s{}",
        report.done,
        report.total,
        started.elapsed().as_secs_f64(),
        if report.cancelled { ", cancelled" } else { "" },
    );
    for failure in &report.failures {
        eprintln!(
            "{} on {} failed: {}",
            failure.step, failure.page, failure.reason
        );
    }
    Ok(())
}
