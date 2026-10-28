use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use image::{GrayImage, Luma, Rgba, RgbaImage};
use imageproc::{drawing::draw_hollow_rect_mut, rect::Rect};
use koharu_ml::comic_layout_yolo26s::{ComicLayoutYolo26sInstance, ComicLayoutYolo26sSegmenter};

#[derive(Debug, Parser)]
struct Cli {
    #[arg(short, long, value_name = "FILE_OR_DIRECTORY")]
    input: PathBuf,

    #[arg(short, long, value_name = "FILE_OR_DIRECTORY")]
    output: Option<PathBuf>,

    #[arg(long, value_name = "FILE_OR_DIRECTORY")]
    mask_output: Option<PathBuf>,

    #[arg(long, value_name = "FILE_OR_DIRECTORY")]
    annotated_output: Option<PathBuf>,

    #[arg(long)]
    confidence_threshold: Option<f32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let runtime = koharu_runtime::Runtime::start().await?;
    let model = ComicLayoutYolo26sSegmenter::load(runtime.device()).await?;
    if cli.input.is_dir() {
        process_directory(&cli, &model)?;
    } else {
        process_file(&cli, &model)?;
    }
    Ok(())
}

fn infer(
    model: &ComicLayoutYolo26sSegmenter,
    image: &image::DynamicImage,
    confidence_threshold: Option<f32>,
) -> Result<koharu_ml::comic_layout_yolo26s::ComicLayoutYolo26sInstances> {
    let result = match confidence_threshold {
        Some(threshold) => model.inference_with_threshold(image, threshold)?,
        None => model.inference(image)?,
    };
    Ok(result)
}

fn process_file(cli: &Cli, model: &ComicLayoutYolo26sSegmenter) -> Result<()> {
    let image = image::open(&cli.input)
        .with_context(|| format!("failed to open {}", cli.input.display()))?;
    let result = infer(model, &image, cli.confidence_threshold)?;

    if let Some(path) = &cli.mask_output {
        mask_image(result.image_width, result.image_height, &result.instances).save(path)?;
    }
    if let Some(path) = &cli.annotated_output {
        annotated_image(&image, &result.instances).save(path)?;
    }

    let json = serde_json::to_string_pretty(&result.instances)?;
    if let Some(path) = &cli.output {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn process_directory(cli: &Cli, model: &ComicLayoutYolo26sSegmenter) -> Result<()> {
    ensure!(
        cli.annotated_output.is_some() || cli.mask_output.is_some() || cli.output.is_some(),
        "directory input requires at least one output directory"
    );
    for directory in [
        cli.annotated_output.as_ref(),
        cli.mask_output.as_ref(),
        cli.output.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        std::fs::create_dir_all(directory)?;
    }

    let mut inputs = std::fs::read_dir(&cli.input)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file() && is_supported_image(path))
        .collect::<Vec<_>>();
    inputs.sort();
    ensure!(
        !inputs.is_empty(),
        "no supported images found in {}",
        cli.input.display()
    );

    for (index, input) in inputs.iter().enumerate() {
        let image =
            image::open(input).with_context(|| format!("failed to open {}", input.display()))?;
        let result = infer(model, &image, cli.confidence_threshold)?;
        let stem = input.file_stem().context("input image has no file stem")?;
        if let Some(directory) = &cli.annotated_output {
            annotated_image(&image, &result.instances)
                .save(directory.join(stem).with_extension("png"))?;
        }
        if let Some(directory) = &cli.mask_output {
            mask_image(result.image_width, result.image_height, &result.instances)
                .save(directory.join(stem).with_extension("png"))?;
        }
        if let Some(directory) = &cli.output {
            std::fs::write(
                directory.join(stem).with_extension("json"),
                serde_json::to_string_pretty(&result.instances)?,
            )?;
        }
        eprintln!(
            "[{}/{}] {}: {} instances",
            index + 1,
            inputs.len(),
            input
                .file_name()
                .map_or_else(|| input.as_os_str(), |file_name| file_name)
                .to_string_lossy(),
            result.instances.len()
        );
    }
    Ok(())
}

fn is_supported_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff"
            )
        })
}

fn mask_image(width: u32, height: u32, instances: &[ComicLayoutYolo26sInstance]) -> GrayImage {
    let mut image = GrayImage::new(width, height);
    for instance in instances {
        let max_y = instance
            .mask
            .height
            .min(height.saturating_sub(instance.mask.y));
        let max_x = instance
            .mask
            .width
            .min(width.saturating_sub(instance.mask.x));
        for y in 0..max_y {
            let offset = y as usize * instance.mask.width as usize;
            for x in 0..max_x {
                if instance.mask.pixels[offset + x as usize] != 0 {
                    image.put_pixel(instance.mask.x + x, instance.mask.y + y, Luma([u8::MAX]));
                }
            }
        }
    }
    image
}

fn annotated_image(
    image: &image::DynamicImage,
    instances: &[ComicLayoutYolo26sInstance],
) -> RgbaImage {
    let colors = [
        Rgba([30, 144, 255, 255]),
        Rgba([40, 220, 90, 255]),
        Rgba([255, 190, 40, 255]),
        Rgba([220, 40, 190, 255]),
    ];
    let mut image = image.to_rgba8();
    for instance in instances {
        let color = colors[instance.label_id % colors.len()];
        let (color_weight, denominator) = if instance.label_id == 0 {
            (1u16, 8u16)
        } else {
            (3u16, 10u16)
        };
        let max_y = instance
            .mask
            .height
            .min(image.height().saturating_sub(instance.mask.y));
        let max_x = instance
            .mask
            .width
            .min(image.width().saturating_sub(instance.mask.x));
        for y in 0..max_y {
            let offset = y as usize * instance.mask.width as usize;
            for x in 0..max_x {
                if instance.mask.pixels[offset + x as usize] == 0 {
                    continue;
                }
                let pixel = image.get_pixel_mut(instance.mask.x + x, instance.mask.y + y);
                for channel in 0..3 {
                    pixel.0[channel] = ((denominator - color_weight) * u16::from(pixel.0[channel])
                        + color_weight * u16::from(color.0[channel]))
                    .div_ceil(denominator) as u8;
                }
            }
        }

        let x1 = instance.bbox[0].floor().max(0.0) as i32;
        let y1 = instance.bbox[1].floor().max(0.0) as i32;
        let x2 = instance.bbox[2].ceil().min(image.width() as f32) as i32;
        let y2 = instance.bbox[3].ceil().min(image.height() as f32) as i32;
        if x2 > x1 && y2 > y1 {
            for inset in 0..3 {
                let width = x2 - x1 - 2 * inset;
                let height = y2 - y1 - 2 * inset;
                if width > 0 && height > 0 {
                    draw_hollow_rect_mut(
                        &mut image,
                        Rect::at(x1 + inset, y1 + inset).of_size(width as u32, height as u32),
                        color,
                    );
                }
            }
        }
    }
    image
}
