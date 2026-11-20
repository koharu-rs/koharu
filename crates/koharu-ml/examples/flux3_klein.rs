use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use koharu_ml::flux3_klein::{Flux3Klein, GenerateOptions};

#[derive(Debug, Parser)]
struct Cli {
    /// The image to edit, the first reference.
    #[arg(short, long, value_name = "FILE")]
    input: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(short, long, value_name = "TEXT")]
    prompt: String,

    #[arg(long, value_name = "FILE")]
    reference: Option<PathBuf>,

    #[arg(long, default_value_t = 16)]
    steps: i32,

    #[arg(long, default_value_t = -1)]
    seed: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let input = cli.input.as_ref().map(image::open).transpose()?;
    let reference = cli.reference.as_ref().map(image::open).transpose()?;

    let device = koharu_runtime::device().await?;
    let pipeline = Flux3Klein::load(device).await?;

    let options = GenerateOptions {
        num_inference_steps: cli.steps,
        seed: cli.seed,
        ..GenerateOptions::default()
    };
    let references = input.into_iter().chain(reference).collect::<Vec<_>>();
    pipeline
        .generate(&references, &cli.prompt, &options)?
        .into_iter()
        .next()
        .context("FLUX.3 Image Klein returned no image")?
        .save(cli.output)?;

    Ok(())
}
