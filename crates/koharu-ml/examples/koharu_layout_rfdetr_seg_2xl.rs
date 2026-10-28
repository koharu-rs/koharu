use std::path::{Path, PathBuf};

use ab_glyph::FontArc;
use anyhow::{Context, Result};
use clap::Parser;
use image::{Rgba, RgbaImage};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut, text_size},
    rect::Rect,
};
use koharu_ml::koharu_layout_rfdetr_seg_2xl::{KoharuLayoutDetections, KoharuLayoutRFDetrSeg2XL};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    annotated_output: Option<PathBuf>,

    /// Font used for detection labels when writing an annotated image.
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,

    #[arg(long, default_value_t = 22.0)]
    font_size: f32,

    #[arg(long)]
    text_threshold: Option<f32>,

    #[arg(long)]
    onomatopoeia_threshold: Option<f32>,

    #[arg(long)]
    bubble_threshold: Option<f32>,

    #[arg(long)]
    panel_threshold: Option<f32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(&cli.input)?;

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = KoharuLayoutRFDetrSeg2XL::load(runtime.device()).await?;
    let mut thresholds = model.recommended_thresholds();
    if let Some(threshold) = cli.text_threshold {
        thresholds.text = threshold;
    }
    if let Some(threshold) = cli.onomatopoeia_threshold {
        thresholds.onomatopoeia = threshold;
    }
    if let Some(threshold) = cli.bubble_threshold {
        thresholds.bubble = threshold;
    }
    if let Some(threshold) = cli.panel_threshold {
        thresholds.panel = threshold;
    }
    let detections = model.inference_with_thresholds(&image, thresholds)?;

    if let Some(path) = cli.annotated_output.as_deref() {
        let font_path = cli
            .font
            .as_deref()
            .context("--font is required when --annotated-output is used")?;
        let font = load_font(font_path)?;
        let mut annotated = image.to_rgba8();
        draw_detections(&mut annotated, &detections, &font, cli.font_size);
        annotated
            .save(path)
            .with_context(|| format!("failed to save {}", path.display()))?;
    }

    let json = serde_json::to_string_pretty(&detections)?;
    if let Some(path) = cli.output {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn load_font(path: &Path) -> Result<FontArc> {
    let data =
        std::fs::read(path).with_context(|| format!("failed to read font {}", path.display()))?;
    FontArc::try_from_vec(data).with_context(|| format!("failed to parse font {}", path.display()))
}

fn draw_detections(
    image: &mut RgbaImage,
    detections: &KoharuLayoutDetections,
    font: &FontArc,
    font_size: f32,
) {
    let colors = [
        Rgba([45, 212, 191, 255]),
        Rgba([251, 146, 60, 255]),
        Rgba([96, 165, 250, 255]),
        Rgba([244, 63, 94, 255]),
        Rgba([168, 85, 247, 255]),
    ];

    // Draw in separate passes so overlapping masks never obscure boxes or labels.
    for detection in &detections.detections {
        if detection.label == "panel" {
            continue;
        }
        let color = colors[detection.label_id.min(colors.len() - 1)];
        for y in 0..detection.mask.height {
            for x in 0..detection.mask.width {
                let index = y as usize * detection.mask.width as usize + x as usize;
                let Some(&mask) = detection.mask.pixels.get(index) else {
                    continue;
                };
                let Some(pixel) =
                    image.get_pixel_mut_checked(detection.mask.x + x, detection.mask.y + y)
                else {
                    continue;
                };
                if mask == 0 {
                    continue;
                }
                for channel in 0..3 {
                    pixel[channel] =
                        ((u16::from(pixel[channel]) * 2 + u16::from(color[channel])) / 3) as u8;
                }
            }
        }
    }

    for detection in &detections.detections {
        let color = colors[detection.label_id.min(colors.len() - 1)];
        let x1 = detection.bbox[0].floor().max(0.0) as i32;
        let y1 = detection.bbox[1].floor().max(0.0) as i32;
        let x2 = detection.bbox[2].ceil().min(image.width() as f32) as i32;
        let y2 = detection.bbox[3].ceil().min(image.height() as f32) as i32;
        if x2 > x1 && y2 > y1 {
            draw_hollow_rect_mut(
                image,
                Rect::at(x1, y1).of_size((x2 - x1) as u32, (y2 - y1) as u32),
                color,
            );
        }
    }

    for detection in &detections.detections {
        let x1 = detection.bbox[0].floor().max(0.0) as i32;
        let y1 = detection.bbox[1].floor().max(0.0) as i32;
        let label = format!("{} {:.2}", detection.label, detection.score);
        let (text_width, text_height) = text_size(font_size, font, &label);
        let padding = 4;
        let label_width = text_width
            .saturating_add(padding * 2)
            .min(image.width())
            .max(1);
        let label_height = text_height
            .saturating_add(padding * 2)
            .min(image.height())
            .max(1);
        let label_x = x1
            .min(image.width().saturating_sub(label_width) as i32)
            .max(0);
        let label_y = if y1 >= label_height as i32 {
            y1 - label_height as i32
        } else {
            y1
        }
        .min(image.height().saturating_sub(label_height) as i32)
        .max(0);
        draw_filled_rect_mut(
            image,
            Rect::at(label_x, label_y).of_size(label_width, label_height),
            Rgba([0, 0, 0, 220]),
        );
        draw_text_mut(
            image,
            Rgba([255, 255, 255, 255]),
            label_x + padding as i32,
            label_y + padding as i32,
            font_size,
            font,
            &label,
        );
    }
}
