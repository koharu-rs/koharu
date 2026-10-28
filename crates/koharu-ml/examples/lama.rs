use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::lama::{InpaintRequest, LaMa};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    mask: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(&cli.input)?;
    let mask = image::open(&cli.mask)?.to_luma8();

    let runtime = koharu_runtime::Runtime::start().await?;

    let model = LaMa::load(runtime.device()).await?;
    let inpainted = model.inference(&image, &mask, &InpaintRequest::default())?;
    inpainted.save(cli.output)?;

    Ok(())
}
