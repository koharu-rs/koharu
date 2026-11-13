use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use koharu_ml::qwen_image::{Options, QwenImage};

#[derive(Debug, Parser)]
struct Cli {
    /// A condition image, read in the order given; repeat for several.
    #[arg(short, long, value_name = "FILE")]
    image: Vec<PathBuf>,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(short, long, value_name = "TEXT")]
    prompt: String,

    #[arg(long, value_name = "TEXT")]
    negative_prompt: Option<String>,

    #[arg(long, default_value_t = 1.0)]
    true_cfg_scale: f32,

    #[arg(long)]
    width: Option<u32>,

    #[arg(long)]
    height: Option<u32>,

    #[arg(long, default_value_t = 40)]
    steps: u32,

    #[arg(long, default_value_t = -1)]
    seed: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let images = cli
        .image
        .iter()
        .map(image::open)
        .collect::<Result<Vec<_>, _>>()?;

    let device = koharu_runtime::device().await?;
    let pipeline = QwenImage::load(device).await?;
    let options = Options {
        negative_prompt: cli.negative_prompt,
        true_cfg_scale: cli.true_cfg_scale,
        width: cli.width,
        height: cli.height,
        num_inference_steps: cli.steps,
        seed: cli.seed,
        ..Options::default()
    };
    pipeline
        .generate(&cli.prompt, &images, &options)?
        .into_iter()
        .next()
        .context("Qwen-Image 2.1 returned no image")?
        .save(cli.output)?;

    Ok(())
}
