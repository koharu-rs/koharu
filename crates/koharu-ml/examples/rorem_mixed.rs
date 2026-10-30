use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::rorem_mixed::{
    DEFAULT_NEGATIVE_PROMPT, DEFAULT_PROMPT, RoremMixed, RoremMixedOptions,
};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    mask: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(short, long, default_value = DEFAULT_PROMPT)]
    prompt: String,

    #[arg(long, default_value = DEFAULT_NEGATIVE_PROMPT)]
    negative_prompt: String,

    #[arg(long, default_value_t = 512)]
    resolution: u32,

    #[arg(long, default_value_t = 0)]
    mask_dilation: u8,

    #[arg(long, default_value_t = 30)]
    steps: i32,

    #[arg(long, default_value_t = 8.0)]
    guidance_scale: f32,

    #[arg(long, default_value_t = 0.999)]
    strength: f32,

    #[arg(long, default_value_t = -1)]
    seed: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(cli.input)?;
    let mask = image::open(cli.mask)?.to_luma8();

    let device = koharu_runtime::device().await?;
    let model = RoremMixed::load(device, device.memory().free).await?;
    let output = model.inference(
        &image,
        &mask,
        &cli.prompt,
        &cli.negative_prompt,
        &RoremMixedOptions {
            resolution: cli.resolution,
            mask_dilation: cli.mask_dilation,
            num_inference_steps: cli.steps,
            guidance_scale: cli.guidance_scale,
            strength: cli.strength,
            seed: cli.seed,
        },
    )?;
    output.save(cli.output)?;
    Ok(())
}
