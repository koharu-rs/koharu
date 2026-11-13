use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use koharu_ml::moss_image::{GenerateOptions, InpaintOptions, MossImage};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE", requires = "mask")]
    input: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE", requires = "input")]
    mask: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(short, long, value_name = "TEXT", default_value = "")]
    prompt: String,

    #[arg(long, default_value_t = 50)]
    steps: usize,

    #[arg(long, default_value_t = 0.8)]
    strength: f64,

    #[arg(long, default_value_t = -1)]
    seed: i64,

    #[arg(long)]
    padding_mask_crop: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let input = cli.input.as_ref().map(image::open).transpose()?;
    let mask = cli.mask.as_ref().map(image::open).transpose()?;

    let device = koharu_runtime::device().await?;
    let pipeline = MossImage::load(device).await?;

    let output = match (input, mask) {
        (Some(input), Some(mask)) => {
            let options = InpaintOptions {
                num_inference_steps: cli.steps,
                strength: cli.strength,
                seed: cli.seed,
                padding_mask_crop: cli.padding_mask_crop,
                ..InpaintOptions::default()
            };
            pipeline.inpaint(&cli.prompt, &input, &mask, &options)?
        }
        _ => {
            let options = GenerateOptions {
                num_inference_steps: u32::try_from(cli.steps)?,
                seed: cli.seed,
                ..GenerateOptions::default()
            };
            pipeline
                .generate(&cli.prompt, &options)?
                .into_iter()
                .next()
                .context("MOSS-Image returned no image")?
        }
    };
    output.save(cli.output)?;

    Ok(())
}
