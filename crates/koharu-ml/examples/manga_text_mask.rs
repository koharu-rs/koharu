use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::manga_text_mask::{MangaTextMaskCleaningOptions, MangaTextMaskGenerator};

#[derive(Debug, Parser)]
#[command(about = "Segment manga text pixels into a binary mask")]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(long, default_value_t = 0.5)]
    threshold: f32,

    #[arg(long, conflicts_with_all = ["horizontal_flip", "vertical_flip"])]
    max_side: Option<u32>,

    #[arg(long, default_value_t = false)]
    horizontal_flip: bool,

    #[arg(long, default_value_t = false)]
    vertical_flip: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(cli.input)?;

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = MangaTextMaskGenerator::load(runtime.device()).await?;
    let segmentation = if let Some(max_side) = cli.max_side {
        model.inference_with_max_side(&image, max_side)?
    } else if cli.horizontal_flip || cli.vertical_flip {
        model.inference_with_tta(&image, cli.horizontal_flip, cli.vertical_flip)?
    } else {
        model.inference(&image)?
    };
    segmentation
        .process(&MangaTextMaskCleaningOptions {
            threshold: cli.threshold,
            ..Default::default()
        })?
        .save(cli.output)?;
    Ok(())
}
