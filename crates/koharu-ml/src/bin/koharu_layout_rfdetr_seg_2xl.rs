use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ab_glyph::FontArc;
use anyhow::{Context, Result, bail};
use clap::Parser;
use image::{Rgba, RgbaImage};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_text_mut, text_size},
    rect::Rect,
};
use koharu_ml::koharu_layout_rfdetr_seg_2xl::{
    InputFit, KoharuLayoutDetections, KoharuLayoutRFDetrSeg2XL, KoharuLayoutThresholds,
};

#[derive(Debug, Parser)]
struct Cli {
    /// Page to segment.
    #[arg(
        short,
        long,
        value_name = "FILE",
        required_unless_present = "input_dir",
        conflicts_with = "input_dir"
    )]
    input: Option<PathBuf>,

    /// Directory of pages to segment under a single model load. Comparing fit
    /// strategies means running the same pages twice, and reloading the
    /// checkpoint per page would spend the run on IO.
    #[arg(
        long,
        value_name = "DIR",
        required_unless_present = "input",
        conflicts_with = "input"
    )]
    input_dir: Option<PathBuf>,

    /// Where --input-dir writes one JSON document per page.
    #[arg(long, value_name = "DIR", conflicts_with = "input")]
    output_dir: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE", conflicts_with = "input_dir")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE", conflicts_with = "input_dir")]
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

    /// How each page is mapped onto the fixed square model input. Defaults to
    /// the shipped behaviour, which is also what an absent configuration key
    /// selects, so the command line and `config.toml` cannot disagree.
    #[arg(long, value_enum)]
    input_fit: Option<InputFit>,

    #[arg(long, default_value_t = false)]
    cpu: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    koharu_ml::init().await?;
    let model = KoharuLayoutRFDetrSeg2XL::load(koharu_ml::device(cli.cpu)).await?;
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
    let input_fit = cli.input_fit.unwrap_or_default();

    match (cli.input.as_deref(), cli.input_dir.as_deref()) {
        (Some(path), None) => {
            let image = image::open(path)?;
            let detections = model.inference_with_thresholds(&image, thresholds, input_fit)?;
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
        }
        (None, Some(directory)) => {
            let output = cli
                .output_dir
                .as_deref()
                .context("--output-dir is required with --input-dir")?;
            segment_directory(&model, thresholds, input_fit, directory, output)?;
        }
        _ => bail!("exactly one of --input or --input-dir is required"),
    }
    Ok(())
}

/// Segments every page in a directory under one model load, writing one JSON
/// document per page. Batch mode reports a per-page label count on stdout
/// because a fit strategy is judged by how much text a page yields, not by any
/// single box.
fn segment_directory(
    model: &KoharuLayoutRFDetrSeg2XL,
    thresholds: KoharuLayoutThresholds,
    input_fit: InputFit,
    directory: &Path,
    output: &Path,
) -> Result<()> {
    let pages = pages(directory)?;
    if pages.is_empty() {
        bail!("no page images found in {}", directory.display());
    }
    std::fs::create_dir_all(output)
        .with_context(|| format!("failed to create {}", output.display()))?;
    for page in pages {
        let image = image::open(&page)?;
        let detections = model.inference_with_thresholds(&image, thresholds, input_fit)?;
        let name = page
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        println!("{name}\t{}", label_counts(&detections));
        let document = output.join(format!(
            "{}.json",
            page.file_stem()
                .map(|stem| stem.to_string_lossy())
                .unwrap_or_default()
        ));
        std::fs::write(&document, serde_json::to_string_pretty(&detections)?)
            .with_context(|| format!("failed to write {}", document.display()))?;
    }
    Ok(())
}

/// Page images in a stable order, so two runs over one directory line up.
fn pages(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?;
    let mut pages = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("failed to read {}", directory.display()))?
        .into_iter()
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff"
                    )
                })
        })
        .collect::<Vec<_>>();
    pages.sort();
    Ok(pages)
}

fn label_counts(detections: &KoharuLayoutDetections) -> String {
    let mut counts = BTreeMap::<&str, usize>::new();
    for detection in &detections.detections {
        *counts.entry(detection.label.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(label, count)| format!("{label}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
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
