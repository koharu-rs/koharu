use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use image::{GrayImage, Luma, Rgba, RgbaImage};
use imageproc::{drawing::draw_hollow_rect_mut, rect::Rect};
use koharu_ml::speech_bubble_yolo11n::{Yolo11nSpeechBubbleInstance, Yolo11nSpeechBubbleSegmenter};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    mask_output: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    annotated_output: Option<PathBuf>,

    #[arg(long)]
    confidence_threshold: Option<f32>,

    #[arg(long)]
    nms_threshold: Option<f32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(&cli.input)?;

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = Yolo11nSpeechBubbleSegmenter::load(runtime.device()).await?;
    let result = match (cli.confidence_threshold, cli.nms_threshold) {
        (None, None) => model.inference(&image)?,
        (confidence, nms) => model.inference_with_thresholds(
            &image,
            confidence.unwrap_or(0.25),
            nms.unwrap_or(0.7),
        )?,
    };

    if let Some(path) = cli.mask_output {
        mask_image(result.image_width, result.image_height, &result.instances).save(path)?;
    }
    if let Some(path) = cli.annotated_output {
        annotated_image(&image, &result.instances).save(path)?;
    }

    let json = serde_json::to_string_pretty(&result.instances)?;
    if let Some(path) = cli.output {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn mask_image(width: u32, height: u32, instances: &[Yolo11nSpeechBubbleInstance]) -> GrayImage {
    let mut image = GrayImage::new(width, height);
    for bubble in instances {
        let max_y = bubble.mask.height.min(height.saturating_sub(bubble.mask.y));
        let max_x = bubble.mask.width.min(width.saturating_sub(bubble.mask.x));
        for y in 0..max_y {
            let offset = y as usize * bubble.mask.width as usize;
            for x in 0..max_x {
                if bubble.mask.pixels[offset + x as usize] != 0 {
                    image.put_pixel(bubble.mask.x + x, bubble.mask.y + y, Luma([u8::MAX]));
                }
            }
        }
    }
    image
}

fn annotated_image(
    image: &image::DynamicImage,
    instances: &[Yolo11nSpeechBubbleInstance],
) -> RgbaImage {
    let mut image = image.to_rgba8();
    for bubble in instances {
        let x1 = bubble.bbox[0].floor().max(0.0) as i32;
        let y1 = bubble.bbox[1].floor().max(0.0) as i32;
        let x2 = bubble.bbox[2].ceil().min(image.width() as f32) as i32;
        let y2 = bubble.bbox[3].ceil().min(image.height() as f32) as i32;
        if x2 > x1 && y2 > y1 {
            draw_hollow_rect_mut(
                &mut image,
                Rect::at(x1, y1).of_size((x2 - x1) as u32, (y2 - y1) as u32),
                Rgba([40, 220, 90, 255]),
            );
        }
    }
    image
}
