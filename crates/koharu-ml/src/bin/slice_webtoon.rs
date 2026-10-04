use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use koharu_ml::webtoon::{SliceParams, plan_slices, row_profile};

#[derive(Debug, Parser)]
struct Cli {
    /// Webtoon strip to slice.
    #[arg(value_name = "FILE")]
    input: PathBuf,

    /// Directory that receives the sliced pages.
    #[arg(value_name = "DIRECTORY")]
    output: PathBuf,

    #[arg(long, value_name = "PIXELS")]
    target_height: Option<u32>,

    #[arg(long, value_name = "PIXELS")]
    min_height: Option<u32>,

    #[arg(long, value_name = "PIXELS")]
    max_height: Option<u32>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut params = SliceParams::default();
    if let Some(target_height) = cli.target_height {
        params.target_height = target_height;
    }
    if let Some(min_height) = cli.min_height {
        params.min_height = min_height;
    }
    if let Some(max_height) = cli.max_height {
        params.max_height = max_height;
    }

    let image = image::open(&cli.input)
        .with_context(|| format!("failed to open {}", cli.input.display()))?;
    let profile = row_profile(&image.to_luma8());

    std::fs::create_dir_all(&cli.output)
        .with_context(|| format!("failed to create {}", cli.output.display()))?;

    let Some(plan) = plan_slices(image.width(), image.height(), &profile, &params) else {
        println!(
            "no slicing needed: {}x{} does not exceed the webtoon trigger",
            image.width(),
            image.height()
        );
        let copy = cli.output.join(
            cli.input
                .file_name()
                .context("input image has no file name")?,
        );
        std::fs::copy(&cli.input, &copy).with_context(|| {
            format!(
                "failed to copy {} to {}",
                cli.input.display(),
                copy.display()
            )
        })?;
        return Ok(());
    };

    println!(
        "plan: {}x{} -> {} pages",
        plan.width,
        plan.height,
        plan.page_count()
    );
    for (index, (start, height)) in plan.page_ranges().iter().enumerate() {
        println!("  page {index}: y={start} height={height}");
        image
            .crop_imm(0, *start, plan.width, *height)
            .save(cli.output.join(format!("{index:03}.png")))?;
    }
    Ok(())
}
