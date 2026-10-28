use std::path::{Path, PathBuf};

use ab_glyph::FontArc;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_line_segment_mut, draw_text_mut, text_size},
    rect::Rect,
};
use koharu_ml::comic_onomatopoeia::{
    ComicOnomatopoeiaDetector, ComicOnomatopoeiaRecognizer, Detection, Recognition,
};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(about = "Detect and recognize comic onomatopoeia, then mark it on the input image")]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Marked image containing detection polygons and OCR labels.
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    /// Write structured detection and OCR results to this file instead of stdout.
    #[arg(long, value_name = "FILE")]
    json_output: Option<PathBuf>,

    /// Font used for OCR labels.
    #[arg(long, value_name = "FILE")]
    font: PathBuf,

    #[arg(long, default_value_t = 28.0)]
    font_size: f32,

    /// Locally fine-tuned TRBA Safetensors; upstream weights are used when omitted.
    #[arg(long, value_name = "FILE")]
    recognizer_weights: Option<PathBuf>,

    #[arg(long, default_value_t = 0.48)]
    detection_threshold: f32,

    #[arg(long, default_value_t = 0.47)]
    recognition_threshold: f32,
}

#[derive(Debug, Serialize)]
struct Region {
    detection: Detection,
    recognition: Recognition,
    accepted: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    ensure!(
        cli.font_size.is_finite() && cli.font_size > 0.0,
        "font size must be positive"
    );
    ensure!(
        (0.0..=1.0).contains(&cli.detection_threshold)
            && (0.0..=1.0).contains(&cli.recognition_threshold),
        "detection and recognition thresholds must be between zero and one"
    );
    let image_data = std::fs::read(&cli.input)
        .with_context(|| format!("failed to read {}", cli.input.display()))?;
    let image = image::load_from_memory(&image_data)
        .with_context(|| format!("failed to decode {}", cli.input.display()))?;

    let runtime = koharu_runtime::Runtime::start().await?;
    let device = runtime.device();
    let detector = ComicOnomatopoeiaDetector::load(device).await?;
    let recognizer = if let Some(path) = cli.recognizer_weights.as_deref() {
        ComicOnomatopoeiaRecognizer::load_from_path(device, path)?
    } else {
        ComicOnomatopoeiaRecognizer::load(device).await?
    };
    let font = load_font(&cli.font)?;

    let regions = detector
        .inference(&image)?
        .into_iter()
        .map(|detection| {
            let crop = crop_detection(&image, &detection)?;
            let recognition = recognizer.inference(&crop)?;
            let accepted = detection.score >= cli.detection_threshold
                && recognition.confidence >= cli.recognition_threshold;
            Ok(Region {
                detection,
                recognition,
                accepted,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut marked = image.to_rgba8();
    draw_regions(&mut marked, &regions, &font, cli.font_size);
    save_marked(marked, &cli.output)?;
    write_json(cli.json_output.as_deref(), &regions)?;
    Ok(())
}

fn save_marked(image: RgbaImage, path: &Path) -> Result<()> {
    let is_jpeg = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg")
        });
    if is_jpeg {
        DynamicImage::ImageRgba8(image).to_rgb8().save(path)
    } else {
        image.save(path)
    }
    .with_context(|| format!("failed to save {}", path.display()))
}

fn load_font(path: &Path) -> Result<FontArc> {
    let data =
        std::fs::read(path).with_context(|| format!("failed to read font {}", path.display()))?;
    FontArc::try_from_vec(data).with_context(|| format!("failed to parse font {}", path.display()))
}

// COO's TRBA evaluation data crops the axis-aligned polygon bounds with an
// exclusive upper coordinate.
// https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/COO-data/data_for_TRBA.ipynb
fn crop_detection(image: &DynamicImage, detection: &Detection) -> Result<DynamicImage> {
    let bounds = detection.polygon.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |mut bounds, point| {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
            bounds
        },
    );
    let left = bounds[0].floor().clamp(0.0, image.width() as f32) as u32;
    let top = bounds[1].floor().clamp(0.0, image.height() as f32) as u32;
    let right = bounds[2].ceil().clamp(0.0, image.width() as f32) as u32;
    let bottom = bounds[3].ceil().clamp(0.0, image.height() as f32) as u32;
    ensure!(
        right > left && bottom > top,
        "MTSv3 produced an empty crop [{left}, {top}, {right}, {bottom}]"
    );
    Ok(image.crop_imm(left, top, right - left, bottom - top))
}

fn draw_regions(image: &mut RgbaImage, regions: &[Region], font: &FontArc, font_size: f32) {
    let label_color = Rgba([255, 255, 255, 255]);
    let label_background = Rgba([0, 0, 0, 220]);
    for (index, region) in regions.iter().enumerate() {
        let polygon_color = if region.accepted {
            Rgba([48, 220, 96, 255])
        } else {
            Rgba([255, 128, 32, 255])
        };
        draw_polygon(image, &region.detection.polygon, polygon_color);

        let label = format!(
            "#{} {} D:{:.2} OCR:{:.2}",
            index + 1,
            region.recognition.text,
            region.detection.score,
            region.recognition.confidence,
        );
        let (text_width, text_height) = text_size(font_size, font, &label);
        let padding = 4;
        let bounds = region.detection.bounding_box;
        let mut x = bounds[0].floor().max(0.0) as i32;
        let box_width = text_width.saturating_add(padding * 2).max(1);
        let box_height = text_height.saturating_add(padding * 2).max(1);
        x = x.min(image.width().saturating_sub(box_width) as i32).max(0);
        let detection_top = bounds[1].floor().max(0.0) as i32;
        let y = if detection_top >= box_height as i32 {
            detection_top - box_height as i32
        } else {
            detection_top
        }
        .min(image.height().saturating_sub(box_height) as i32)
        .max(0);
        draw_filled_rect_mut(
            image,
            Rect::at(x, y).of_size(box_width, box_height),
            label_background,
        );
        draw_text_mut(
            image,
            label_color,
            x + padding as i32,
            y + padding as i32,
            font_size,
            font,
            &label,
        );
    }
}

fn draw_polygon(image: &mut RgbaImage, points: &[[f32; 2]], color: Rgba<u8>) {
    if points.len() < 2 {
        return;
    }
    for index in 0..points.len() {
        let from = points[index];
        let to = points[(index + 1) % points.len()];
        draw_line_segment_mut(image, (from[0], from[1]), (to[0], to[1]), color);
    }
}

fn write_json(path: Option<&Path>, value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(path) = path {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}
