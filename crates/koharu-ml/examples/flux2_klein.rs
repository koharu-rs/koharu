use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use koharu_ml::flux2_klein::{Flux2Klein, Flux2KleinInpaintOptions, Flux2KleinOptions};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE", requires = "input")]
    mask: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(short, long, value_name = "TEXT")]
    prompt: String,

    #[arg(long, value_name = "FILE")]
    reference: Option<PathBuf>,

    #[arg(long, default_value_t = 4)]
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
    let reference = cli.reference.as_ref().map(image::open).transpose()?;

    let device = koharu_runtime::device().await?;
    let pipeline = Flux2Klein::load(device, device.memory().free).await?;

    let output = if let Some(mask) = mask.as_ref() {
        let input = input
            .as_ref()
            .context("--input is required when --mask is provided")?;
        let options = Flux2KleinInpaintOptions {
            num_inference_steps: cli.steps,
            strength: cli.strength,
            seed: cli.seed,
            padding_mask_crop: cli.padding_mask_crop,
        };
        pipeline.inpaint(&cli.prompt, input, reference.as_ref(), mask, &options)?
    } else {
        let options = Flux2KleinOptions {
            num_inference_steps: i32::try_from(cli.steps)?,
            seed: cli.seed,
            ..Flux2KleinOptions::default()
        };
        let references = input.into_iter().chain(reference).collect::<Vec<_>>();
        let generated = pipeline
            .inference(&references, &cli.prompt, &options)?
            .into_iter()
            .next()
            .context("FLUX.2 Klein returned no image")?;
        image::DynamicImage::ImageRgb8(generated)
    };
    output.save(cli.output)?;

    Ok(())
}
