use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_hollow_rect_mut, rect::Rect};
use koharu_ml::comic_text_bubble_detector::{RTDetrV2Detection, TextBlock};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    annotated_output: Option<PathBuf>,

    #[arg(long, default_value_t = 0.3)]
    confidence_threshold: f32,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(&cli.input)?;

    let runtime = koharu_runtime::Runtime::start().await?;

    let model = RTDetrV2Detection::load(runtime.device()).await?;
    let text_blocks = model.inference(&image, cli.confidence_threshold)?;

    if let Some(path) = cli.annotated_output {
        annotated_image(&image, &text_blocks).save(path)?;
    }

    let json = serde_json::to_string_pretty(&text_blocks)?;
    if let Some(path) = cli.output {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }

    Ok(())
}

fn annotated_image(image: &image::DynamicImage, text_blocks: &[TextBlock]) -> RgbaImage {
    let mut annotated = image.to_rgba8();
    for block in text_blocks {
        draw_box(&mut annotated, block.xyxy, Rgba([40, 220, 90, 255]));
        if let Some(bubble) = block.bubble_xyxy {
            draw_box(&mut annotated, bubble, Rgba([40, 160, 255, 255]));
        }
    }
    annotated
}

fn draw_box(image: &mut RgbaImage, bbox: [i32; 4], color: Rgba<u8>) {
    let x1 = bbox[0].clamp(0, image.width() as i32);
    let y1 = bbox[1].clamp(0, image.height() as i32);
    let x2 = bbox[2].clamp(0, image.width() as i32);
    let y2 = bbox[3].clamp(0, image.height() as i32);
    if x2 > x1 && y2 > y1 {
        draw_hollow_rect_mut(
            image,
            Rect::at(x1, y1).of_size((x2 - x1) as u32, (y2 - y1) as u32),
            color,
        );
    }
}
