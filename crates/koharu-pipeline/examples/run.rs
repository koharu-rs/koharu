//! Runs the pipeline with its default settings over image files as a
//! project's pages, every step unless `--steps` names some, tracing each
//! stage's time as its span closes. Then it saves each page rendered as the
//! app draws it, the page with only its clean-ups laid over the scan, and a
//! copy tinting each block's clean-up by what made it: green for one colour,
//! blue for a smooth fill, and red for the inpainting model.
//!
//! `cargo run -p koharu-pipeline --example run -- --out out page1.png page2.png`

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use image::{Rgba, RgbaImage};
use koharu_llm::rig::completion::Effort;
use koharu_pipeline::{Event, Inpainter, Outcome, Pipeline, PipelineConfig, Request, Step, Target};
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::{Content, Edit, Hash, Op, Session, Tile};
use parking_lot::Mutex;
use tracing_subscriber::{
    EnvFilter,
    fmt::{format::FmtSpan, time::uptime},
};

#[derive(Debug, Parser)]
struct Cli {
    /// Image files, or folders of them, as the pages in order.
    #[arg(required = true, value_name = "PAGE")]
    pages: Vec<PathBuf>,

    /// The steps to run: detect, recognize, inpaint, translate.
    #[arg(
        short,
        long,
        value_delimiter = ',',
        value_parser = parse::<Step>,
        default_value = "detect,recognize,inpaint,translate"
    )]
    steps: Vec<Step>,

    /// The inpainting model, by its id in `[pipeline]`.
    #[arg(long, value_parser = parse::<Inpainter>)]
    inpainter: Option<Inpainter>,

    /// The translation model, as a `vendor/model` reference.
    #[arg(long, value_name = "MODEL")]
    translator: Option<String>,

    /// Show the translation model each page beside its texts.
    #[arg(long)]
    vision: bool,

    /// How much the translation model reasons: minimal, low, medium, high.
    #[arg(long, value_parser = parse::<Effort>)]
    reasoning: Option<Effort>,

    /// Where the project and the pages it makes go.
    #[arg(short, long, value_name = "DIR")]
    out: PathBuf,
}

/// A value by its id, as the `[pipeline]` section and requests write it.
fn parse<T: serde::de::DeserializeOwned>(id: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(id.to_owned()))
        .map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .with_timer(uptime())
        .init();
    let cli = Cli::parse();
    let files = pages(&cli.pages)?;
    std::fs::create_dir_all(&cli.out)?;
    let path = cli.out.join("pages.khr");
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let mut session = Session::create(&path)?;
    let mut ops = Vec::new();
    for (index, file) in files.iter().enumerate() {
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
    let session = Arc::new(Mutex::new(session));

    let mut config = PipelineConfig::default();
    if let Some(translator) = cli.translator {
        config.translator.model = translator;
    }
    if let Some(inpainter) = cli.inpainter {
        config.inpainter.model = inpainter;
    }
    config.translator.vision = cli.vision;
    config.translator.reasoning = cli.reasoning;
    let pipeline = Pipeline::new(config);
    let mut events = pipeline.events();
    let request = Request {
        target: Target::Project,
        steps: cli.steps.into_iter().collect(),
        config: None,
    };
    let run = pipeline.run(Arc::clone(&session), request)?;
    let pages: Vec<_> = session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let number = |page| {
        pages
            .iter()
            .position(|found| *found == page)
            .map_or(0, |at| at + 1)
    };
    let started = std::time::Instant::now();
    loop {
        let elapsed = started.elapsed().as_secs_f32();
        match events.recv().await? {
            Event::Started { page, step, .. } => {
                println!("{elapsed:7.1}s page {} {step} started", number(page))
            }
            Event::Finished { page, step, .. } => {
                println!("{elapsed:7.1}s page {} {step} finished", number(page))
            }
            Event::Ended {
                run: ended,
                outcome,
            } if ended == run => {
                println!("{elapsed:7.1}s ended: {outcome:?}");
                if let Outcome::Failed { reason, .. } = outcome {
                    bail!(reason);
                }
                break;
            }
            Event::Ended { .. } => {}
        }
    }

    let session = session.lock();
    let project = session.project();
    let renderer = Renderer::new(RendererConfig::default());
    for (page, file) in project.pages().zip(&files) {
        let background = page
            .background
            .context("a page made from a file has a background")?;
        let mut cleaned = image::load_from_memory(&session.file(&background.file)?)?.to_rgba8();
        let mut marked = cleaned.clone();
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for layer in project.layers_of(page.id)? {
            let Content::Block(block) = &layer.content else {
                continue;
            };
            if !block.source.content.text().trim().is_empty() {
                *counts.entry("read".to_owned()).or_default() += 1;
            }
            if block.translation.is_some() {
                *counts.entry("translated".to_owned()).or_default() += 1;
            }
            let Some(action) = layer
                .attributes
                .get("c2pa")
                .and_then(|c2pa| c2pa.get("inpainting"))
            else {
                continue;
            };
            let made_by = action["parameters"]["fill"]
                .as_str()
                .or(action["softwareAgent"]["name"].as_str())
                .unwrap_or("unknown")
                .to_owned();
            let tint = match made_by.as_str() {
                "color" => Rgba([0, 200, 0, 255]),
                "smooth" => Rgba([0, 120, 255, 255]),
                _ => Rgba([255, 0, 0, 255]),
            };
            *counts.entry(made_by).or_default() += 1;
            let tiles = session.tiles(block.cleanup.id, None)?;
            let size = Tile::SIZE as i64;
            for tile in &tiles.tiles {
                let left = i64::from(tile.x) * size + i64::from(tiles.offset.x);
                let top = i64::from(tile.y) * size + i64::from(tiles.offset.y);
                for (at, pixel) in tile.pixels.as_chunks::<4>().0.iter().enumerate() {
                    let (x, y) = (left + at as i64 % size, top + at as i64 / size);
                    if pixel[3] == 0
                        || x < 0
                        || y < 0
                        || x >= i64::from(cleaned.width())
                        || y >= i64::from(cleaned.height())
                    {
                        continue;
                    }
                    let pixel = Rgba([pixel[0], pixel[1], pixel[2], 255]);
                    cleaned.put_pixel(x as u32, y as u32, pixel);
                    let half = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
                    let mixed = Rgba(std::array::from_fn(|channel| {
                        half(pixel.0[channel], tint.0[channel])
                    }));
                    marked.put_pixel(x as u32, y as u32, mixed);
                }
            }
        }
        let stem = file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("page");
        let rendered = renderer.rasterize(&renderer.render(&session, page.id)?)?;
        save(&rendered.image, &cli.out, &format!("{stem}_final.png"))?;
        save(&cleaned, &cli.out, &format!("{stem}_cleaned.png"))?;
        save(&marked, &cli.out, &format!("{stem}_fills.png"))?;
        if !counts.is_empty() {
            println!("{stem}: {counts:?}");
        }
    }
    Ok(())
}

/// The image files `paths` name, a folder's in name order.
fn pages(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<_>>()?;
            found.retain(|file| image::ImageFormat::from_path(file).is_ok());
            found.sort();
            files.extend(found);
        } else {
            files.push(path.clone());
        }
    }
    Ok(files)
}

fn save(image: &RgbaImage, dir: &std::path::Path, name: &str) -> Result<()> {
    image.save(dir.join(name))?;
    Ok(())
}
