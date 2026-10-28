use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_line_segment_mut;
use koharu_ml::pp_ocr_v6::{
    det::{PPOCRV6MediumDet, TextDetections},
    rec::PPOCRV6MediumRec,
};

#[derive(Debug, Parser)]
#[command(about = "Run PP-OCRv6 text detection or recognition")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Detect text regions and return their rotated polygons.
    Det(DetArgs),
    /// Recognize text in a cropped text-line image.
    Rec(RecArgs),
}

#[derive(Debug, Args)]
struct DetArgs {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    annotated_output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RecArgs {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let runtime = koharu_runtime::Runtime::start().await?;
    let device = runtime.device();
    match cli.command {
        Command::Det(args) => detect(args, device).await,
        Command::Rec(args) => recognize(args, device).await,
    }
}

async fn detect(args: DetArgs, device: &koharu_ml::Device) -> Result<()> {
    let image = image::open(&args.input)?;
    let model = PPOCRV6MediumDet::load(device).await?;
    let detections = model.inference(&image)?;

    if let Some(path) = args.annotated_output {
        let mut annotated = image.to_rgba8();
        draw_detections(&mut annotated, &detections);
        annotated.save(path)?;
    }
    write_json(args.output.as_deref(), &detections)
}

async fn recognize(args: RecArgs, device: &koharu_ml::Device) -> Result<()> {
    let image = image::open(&args.input)?;
    let model = PPOCRV6MediumRec::load(device).await?;
    let recognition = model.inference(&image)?;
    write_json(args.output.as_deref(), &recognition)
}

fn write_json(path: Option<&Path>, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(path) = path {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn draw_detections(image: &mut RgbaImage, detections: &TextDetections) {
    let color = Rgba([255, 32, 32, 255]);
    for detection in &detections.detections {
        for index in 0..4 {
            let from = detection.polygon[index];
            let to = detection.polygon[(index + 1) % 4];
            draw_line_segment_mut(image, (from[0], from[1]), (to[0], to[1]), color);
        }
    }
}
