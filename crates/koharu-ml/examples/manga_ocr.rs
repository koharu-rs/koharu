use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::manga_ocr::MangaOcr;

#[derive(Debug, Parser)]
#[command(about = "Run Manga OCR text recognition")]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(cli.input)?;

    let device = koharu_runtime::device().await?;
    let model = MangaOcr::load(device).await?;
    for text in model.inference(&[image])? {
        println!("{text}");
    }
    Ok(())
}
