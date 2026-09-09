use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::hayai_ocr::HayaiOcr;

#[derive(Debug, Parser)]
#[command(about = "Run Hayai OCR text recognition")]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(long, default_value_t = false)]
    cpu: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(cli.input)?;

    koharu_ml::init().await?;
    let model = HayaiOcr::load(koharu_ml::device(cli.cpu)).await?;
    println!("{}", model.inference(&image)?);
    Ok(())
}
