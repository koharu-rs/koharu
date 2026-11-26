use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use koharu_ml::yuzumarker_font_detection::YuzuMarkerFontDetection;

#[derive(Debug, Parser)]
#[command(about = "Read the font kernels of text crops with YuzuMarker.FontDetection")]
struct Cli {
    /// Crops of text, each printed as a JSON line of its kernels.
    #[arg(required = true, value_name = "FILE")]
    inputs: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let crops = cli
        .inputs
        .iter()
        .map(|path| Ok(image::open(path)?.to_rgb8()))
        .collect::<Result<Vec<_>>>()?;

    let device = koharu_runtime::device().await?;
    let model = YuzuMarkerFontDetection::load(device).await?;
    for (path, kernels) in cli.inputs.iter().zip(model.inference(&crops)?) {
        println!(
            "{}",
            serde_json::json!({ "file": path.display().to_string(), "kernels": kernels })
        );
    }
    Ok(())
}
