use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use image::{Rgba, RgbaImage};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_line_segment_mut},
    rect::Rect,
};
use koharu_ml::pp_doclayout_v3::{PPDocLayoutV3, PPDocLayoutV3Detections};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    annotated_output: Option<PathBuf>,

    #[arg(long, default_value_t = 0.3)]
    threshold: f32,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let image = image::open(&cli.input)?;

    let runtime = koharu_runtime::Runtime::start().await?;

    let model = PPDocLayoutV3::load(runtime.device()).await?;
    let result = model.inference(&image, cli.threshold)?;

    if let Some(path) = cli.annotated_output {
        let mut annotated = image.to_rgba8();
        draw_regions(&mut annotated, &result);
        annotated.save(path)?;
    }

    let json = serde_json::to_string_pretty(&result)?;
    if let Some(path) = cli.output {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }

    Ok(())
}

fn draw_regions(image: &mut RgbaImage, detections: &PPDocLayoutV3Detections) {
    let rect_color = Rgba([255, 32, 32, 255]);
    let polygon_color = Rgba([32, 220, 80, 255]);
    let label_background = Rgba([0, 0, 0, 210]);
    let label_color = Rgba([255, 255, 255, 255]);
    for region in &detections.regions {
        let x1 = region.bbox[0].min(region.bbox[2]).max(0.0);
        let y1 = region.bbox[1].min(region.bbox[3]).max(0.0);
        let x2 = region.bbox[0].max(region.bbox[2]).min(image.width() as f32);
        let y2 = region.bbox[1]
            .max(region.bbox[3])
            .min(image.height() as f32);
        let width = (x2 - x1).max(1.0) as u32;
        let height = (y2 - y1).max(1.0) as u32;
        draw_hollow_rect_mut(
            image,
            Rect::at(x1 as i32, y1 as i32).of_size(width, height),
            rect_color,
        );

        if let Some(polygon_points) = &region.polygon_points {
            draw_polygon(image, polygon_points, polygon_color);
        }

        let label = format!(
            "{} {} {:.2}",
            region.order_seq,
            region.label.to_ascii_uppercase(),
            region.score
        );
        draw_label(
            image,
            x1 as i32,
            y1 as i32,
            &label,
            label_color,
            label_background,
        );
    }
}

fn draw_polygon(image: &mut RgbaImage, points: &[[f32; 2]], color: Rgba<u8>) {
    if points.len() < 2 {
        return;
    }

    for pair in points.windows(2) {
        draw_line_segment_mut(
            image,
            (pair[0][0], pair[0][1]),
            (pair[1][0], pair[1][1]),
            color,
        );
    }
    let first = points[0];
    let last = points[points.len() - 1];
    draw_line_segment_mut(image, (last[0], last[1]), (first[0], first[1]), color);
}

fn draw_label(
    image: &mut RgbaImage,
    x: i32,
    y: i32,
    text: &str,
    color: Rgba<u8>,
    background: Rgba<u8>,
) {
    const SCALE: i32 = 2;
    const GLYPH_WIDTH: i32 = 5;
    const GLYPH_HEIGHT: i32 = 7;
    const GAP: i32 = 1;
    const PADDING: i32 = 3;

    let text = text
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '?' })
        .collect::<String>();
    let width =
        (text.chars().count() as i32 * (GLYPH_WIDTH + GAP) - GAP).max(1) * SCALE + PADDING * 2;
    let height = GLYPH_HEIGHT * SCALE + PADDING * 2;
    let x = x.clamp(0, image.width().saturating_sub(1) as i32);
    let y = if y - height >= 0 { y - height } else { y };
    let y = y.clamp(0, image.height().saturating_sub(1) as i32);

    draw_filled_rect_mut(
        image,
        Rect::at(x, y).of_size(width as u32, height as u32),
        background,
    );

    let mut cursor_x = x + PADDING;
    let baseline_y = y + PADDING;
    for ch in text.chars() {
        draw_glyph(image, cursor_x, baseline_y, ch, SCALE, color);
        cursor_x += (GLYPH_WIDTH + GAP) * SCALE;
    }
}

fn draw_glyph(image: &mut RgbaImage, x: i32, y: i32, ch: char, scale: i32, color: Rgba<u8>) {
    for (row, bits) in glyph_bits(ch).iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) == 0 {
                continue;
            }
            let px = x + col * scale;
            let py = y + row as i32 * scale;
            draw_filled_rect_mut(
                image,
                Rect::at(px, py).of_size(scale as u32, scale as u32),
                color,
            );
        }
    }
}

fn glyph_bits(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b01010, 0b01010, 0b00100, 0b01010, 0b01010, 0b10001,
        ],
        'Y' => [
            0b10001, 0b01010, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        '_' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        ' ' => [0; 7],
        _ => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
    }
}
