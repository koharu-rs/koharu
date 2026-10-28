use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::font_detector::FontDetector;

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, required = true, value_name = "FILE")]
    input: Vec<PathBuf>,

    #[arg(short = 'k', long, default_value_t = 5)]
    top_k: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let runtime = koharu_runtime::Runtime::start().await?;

    let images = cli
        .input
        .iter()
        .map(image::open)
        .collect::<Result<Vec<_>, _>>()?;
    let model = FontDetector::load(runtime.device()).await?;
    let predictions = model.inference(&images, cli.top_k)?;
    println!("{}", serde_json::to_string_pretty(&predictions)?);
    Ok(())
}
