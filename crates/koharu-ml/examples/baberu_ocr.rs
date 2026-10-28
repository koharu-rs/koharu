use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::baberu_ocr::BaberuOcr;

#[derive(Debug, Parser)]
#[command(about = "Run Baberu OCR speech-bubble recognition")]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(cli.input)?;

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = BaberuOcr::load(runtime.device()).await?;
    for text in model.inference(&[image])? {
        println!("{text}");
    }
    Ok(())
}
