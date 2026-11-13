use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use ab_glyph::FontArc;
use anyhow::{Context, Result};
use clap::Parser;
use image::{DynamicImage, GrayImage, Rgba, RgbaImage};
use imageproc::{
    contours::{BorderType, find_contours},
    drawing::{draw_filled_circle_mut, draw_text_mut, text_size},
};
use koharu_ml::koharu_layout_highres::{
    Bubble, Kind, KoharuLayoutHighres, Layout, Lettering, Mask, Options, Typography,
};
use serde_json::{Value, json};

#[derive(Debug, Parser)]
struct Cli {
    /// A page, or a folder of pages.
    #[arg(short, long, value_name = "PATH")]
    input: PathBuf,

    /// Where to write the detections as JSON; a folder when the input is one.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Where to write the annotated page; a folder when the input is one.
    #[arg(long, value_name = "PATH")]
    annotated_output: Option<PathBuf>,

    /// Font used for detection labels when writing an annotated image.
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,

    /// Label text size on a 1500-pixel page; labels scale with the page.
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

/// The layout with the typography measured in each lettering.
struct Page<'a> {
    layout: &'a Layout,
    typography: Vec<Option<Typography>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let folder = cli.input.is_dir();
    let pages = if folder {
        let mut pages: Vec<PathBuf> = std::fs::read_dir(&cli.input)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()?;
        pages.retain(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    ["jpg", "jpeg", "png", "webp"]
                        .contains(&extension.to_ascii_lowercase().as_str())
                })
        });
        pages.sort();
        pages
    } else {
        vec![cli.input.clone()]
    };
    let font = match (&cli.annotated_output, &cli.font) {
        (Some(_), Some(path)) => Some(load_font(path)?),
        (Some(_), None) => anyhow::bail!("--font is required when --annotated-output is used"),
        (None, _) => None,
    };
    for path in [&cli.output, &cli.annotated_output].into_iter().flatten() {
        if folder {
            std::fs::create_dir_all(path)?;
        }
    }

    let device = koharu_runtime::device().await?;
    let model = KoharuLayoutHighres::load(device).await?;
    let options = Options {
        text: cli.text_threshold,
        onomatopoeia: cli.onomatopoeia_threshold,
        bubble: cli.bubble_threshold,
        panel: cli.panel_threshold,
    };
    for path in &pages {
        // Some pages' extensions don't match their format.
        let image = image::ImageReader::open(path)?
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("failed to open {}", path.display()))?;
        let started = Instant::now();
        let layout = model.inference(&image, &options)?.layout(&image.to_rgb8());
        let inferred = started.elapsed();
        let page = measure(&image, &layout);
        let measured = started.elapsed() - inferred;
        eprintln!(
            "{}: {} lettering, {} bubbles, {} panels, {} measured; inference {:.0} ms, measurement {:.0} ms",
            path.display(),
            layout.lettering.len(),
            layout.bubbles.len(),
            layout.panels.len(),
            page.typography.iter().flatten().count(),
            inferred.as_secs_f64() * 1000.0,
            measured.as_secs_f64() * 1000.0,
        );

        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("page");
        let target = |output: &Path, extension: &str| {
            if folder {
                output.join(format!("{stem}.{extension}"))
            } else {
                output.to_path_buf()
            }
        };
        if let (Some(output), Some(font)) = (cli.annotated_output.as_deref(), &font) {
            let mut annotated = image.to_rgba8();
            annotate(&mut annotated, &page, font, cli.font_size);
            let output = target(output, "jpg");
            DynamicImage::ImageRgba8(annotated)
                .to_rgb8()
                .save(&output)
                .with_context(|| format!("failed to save {}", output.display()))?;
        }
        let json = serde_json::to_string_pretty(&page.json())?;
        match cli.output.as_deref() {
            Some(output) => std::fs::write(target(output, "json"), json)?,
            None if !folder => println!("{json}"),
            None => {}
        }
    }
    Ok(())
}

/// The typography of each lettering.
fn measure<'a>(image: &DynamicImage, layout: &'a Layout) -> Page<'a> {
    let page = image.to_rgb8();
    Page {
        layout,
        typography: layout
            .lettering
            .iter()
            .map(|lettering| lettering.typography(&page))
            .collect(),
    }
}

impl Page<'_> {
    /// The layout as JSON: boxes, relations by index, and outlines and rooms
    /// as SVG path data in page pixels.
    fn json(&self) -> Value {
        let layout = self.layout;
        json!({
            "width": layout.width,
            "height": layout.height,
            "lettering": layout.lettering.iter().zip(&self.typography).map(|(lettering, typography)| json!({
                "kind": match lettering.kind {
                    Kind::Text => "text",
                    Kind::Onomatopoeia => "onomatopoeia",
                },
                "score": lettering.score,
                "bbox": lettering.bbox,
                "bubble": lettering.bubble,
                "panel": lettering.panel,
                "room": lettering.room.as_ref().map(|room| room.to_svg()),
                "typography": typography,
            })).collect::<Vec<_>>(),
            "bubbles": layout.bubbles.iter().map(|bubble| json!({
                "score": bubble.score,
                "bbox": bubble.bbox,
                "outline": bubble.outline.to_svg(),
                "lettering": bubble.lettering,
                "panel": bubble.panel,
            })).collect::<Vec<_>>(),
            "panels": layout.panels.iter().map(|panel| json!({
                "score": panel.score,
                "bbox": panel.bbox,
            })).collect::<Vec<_>>(),
        })
    }
}

fn load_font(path: &Path) -> Result<FontArc> {
    let data =
        std::fs::read(path).with_context(|| format!("failed to read font {}", path.display()))?;
    FontArc::try_from_vec(data).with_context(|| format!("failed to parse font {}", path.display()))
}

const TEXT: [u8; 3] = [45, 212, 191];
const ONOMATOPOEIA: [u8; 3] = [251, 146, 60];
const BUBBLE: [u8; 3] = [236, 72, 153];
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

/// Lettering masks inverted under a see-through box, bubbles outlined in
/// dots, and under each measured mask a label: its ink in a rounded block,
/// ↑ for vertical lines or → for horizontal, its tilt as a slope, and a bar
/// as thick as its strokes, in the page's own pixels. Panels aren't drawn.
fn annotate(image: &mut RgbaImage, page: &Page<'_>, font: &FontArc, font_size: f32) {
    // Sizes are for a 1500-pixel page and scale with it.
    let scale = (image.width().max(image.height()) as f32 / 1500.0).max(0.5);
    let lettering = &page.layout.lettering;

    // One union, so overlapping masks invert once.
    let width = image.width();
    let mut inverted = vec![false; (width * image.height()) as usize];
    for lettering in lettering {
        let mask = &lettering.mask;
        for (index, _) in mask
            .pixels
            .iter()
            .enumerate()
            .filter(|(_, value)| **value != 0)
        {
            let (x, y) = (
                mask.x + index as u32 % mask.width,
                mask.y + index as u32 / mask.width,
            );
            if x < width && y < image.height() {
                inverted[(y * width + x) as usize] = true;
            }
        }
    }
    for (pixel, inverted) in image.pixels_mut().zip(inverted) {
        if inverted {
            for channel in 0..3 {
                pixel[channel] = u8::MAX - pixel[channel];
            }
        }
    }

    for lettering in lettering {
        let [x1, y1, x2, y2] = lettering.bbox;
        let color = match lettering.kind {
            Kind::Text => TEXT,
            Kind::Onomatopoeia => ONOMATOPOEIA,
        };
        fill(image, (x1, y1, x2 - x1, y2 - y1), 0.0, color, 0.25);
    }

    for Bubble { mask, .. } in &page.layout.bubbles {
        dot_edge(image, mask, scale);
    }

    for (lettering, typography) in lettering.iter().zip(&page.typography) {
        if let Some(typography) = typography {
            label(image, lettering, typography, font, font_size * scale, scale);
        }
    }
}

/// Dots every few pixels along the outer edge of `mask`.
fn dot_edge(image: &mut RgbaImage, mask: &Mask, scale: f32) {
    let shape = GrayImage::from_raw(mask.width, mask.height, mask.pixels.clone())
        .expect("a mask holds a byte a pixel");
    let (spacing, radius) = (7.0 * scale, (1.6 * scale).round().max(1.0) as i32);
    for contour in find_contours::<i32>(&shape) {
        if !matches!(contour.border_type, BorderType::Outer) {
            continue;
        }
        let mut travelled = spacing;
        let mut previous: Option<(i32, i32)> = None;
        for point in contour.points {
            if let Some((x, y)) = previous {
                travelled += ((point.x - x) as f32).hypot((point.y - y) as f32);
            }
            previous = Some((point.x, point.y));
            if travelled >= spacing {
                travelled = 0.0;
                let center = (mask.x as i32 + point.x, mask.y as i32 + point.y);
                draw_filled_circle_mut(image, center, radius, rgba(BUBBLE));
            }
        }
    }
}

/// The label under a lettering mask's box.
fn label(
    image: &mut RgbaImage,
    lettering: &Lettering,
    typography: &Typography,
    font: &FontArc,
    size: f32,
    scale: f32,
) {
    let arrow = if typography.vertical { "↑" } else { "→" };
    let slope = format!("{:+.1}%", typography.tilt.to_radians().tan() * 100.0);
    let text = format!("{arrow} {slope}");
    let (text_width, text_height) = text_size(size, font, &text);
    let (text_width, text_height) = (text_width as f32, text_height as f32);
    let padding = 5.0 * scale;
    let swatch = text_height;
    let stroke = typography.stroke_width.map(|width| width.round().max(1.0));
    let bar = 24.0 * scale;
    let width =
        padding + swatch + padding + text_width + stroke.map_or(0.0, |_| padding + bar) + padding;
    let height = swatch.max(stroke.unwrap_or(0.0)) + 2.0 * padding;
    let [x1, _, _, y2] = lettering.bbox;
    let x = x1.clamp(0.0, (image.width() as f32 - width).max(0.0));
    let y = (y2 + 2.0 * scale).clamp(0.0, (image.height() as f32 - height).max(0.0));
    let middle = y + height / 2.0;

    fill(image, (x, y, width, height), 6.0 * scale, BLACK, 0.75);
    let block = (x + padding, middle - swatch / 2.0, swatch, swatch);
    fill(image, block, swatch / 3.0, WHITE, 1.0);
    if let Some(ink) = typography.ink {
        let ring = scale.max(1.0);
        let inner = (
            block.0 + ring,
            block.1 + ring,
            swatch - 2.0 * ring,
            swatch - 2.0 * ring,
        );
        fill(image, inner, swatch / 3.0 - ring, ink, 1.0);
    }
    let text_x = x + padding + swatch + padding;
    draw_text_mut(
        image,
        rgba(WHITE),
        text_x as i32,
        (middle - text_height / 2.0) as i32,
        size,
        font,
        &text,
    );
    if let Some(stroke) = stroke {
        let bar_x = text_x + text_width + padding;
        fill(
            image,
            (bar_x, middle - stroke / 2.0, bar, stroke),
            0.0,
            WHITE,
            1.0,
        );
    }
}

/// Blends `color` at `opacity` over the pixels whose centres fall in the
/// rectangle `(x, y, width, height)` with corners rounded to `radius`.
fn fill(
    image: &mut RgbaImage,
    (x, y, width, height): (f32, f32, f32, f32),
    radius: f32,
    color: [u8; 3],
    opacity: f32,
) {
    let radius = radius.clamp(0.0, width.min(height) / 2.0);
    let (left, top) = (x.floor().max(0.0) as u32, y.floor().max(0.0) as u32);
    let right = ((x + width).ceil().max(0.0) as u32).min(image.width());
    let bottom = ((y + height).ceil().max(0.0) as u32).min(image.height());
    for py in top..bottom {
        for px in left..right {
            let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
            if cx < x || cx > x + width || cy < y || cy > y + height {
                continue;
            }
            let nearest_x = cx.clamp(x + radius, x + width - radius);
            let nearest_y = cy.clamp(y + radius, y + height - radius);
            if (cx - nearest_x).hypot(cy - nearest_y) > radius {
                continue;
            }
            let pixel = image.get_pixel_mut(px, py);
            for channel in 0..3 {
                let blended = f32::from(pixel[channel]) * (1.0 - opacity)
                    + f32::from(color[channel]) * opacity;
                pixel[channel] = blended.round() as u8;
            }
        }
    }
}

fn rgba([r, g, b]: [u8; 3]) -> Rgba<u8> {
    Rgba([r, g, b, u8::MAX])
}
