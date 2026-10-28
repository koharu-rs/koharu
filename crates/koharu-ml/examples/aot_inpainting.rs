use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::aot_inpainting::AotInpainting;

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    mask: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(long, default_value_t = 2048)]
    max_side: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(&cli.input)?;
    let mask = image::open(&cli.mask)?.to_luma8();

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = AotInpainting::load(runtime.device()).await?;
    let inpainted = model.inference_with_max_side(&image, &mask, cli.max_side)?;
    inpainted.save(cli.output)?;
    Ok(())
}
