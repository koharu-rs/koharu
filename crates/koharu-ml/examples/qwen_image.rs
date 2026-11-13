use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use koharu_ml::qwen_image::{InpaintOptions, Options, QwenImage};

#[derive(Debug, Parser)]
struct Cli {
    /// A condition image, read in the order given; repeat for several. With
    /// `--mask`, the one image to inpaint.
    #[arg(short, long, value_name = "FILE")]
    image: Vec<PathBuf>,

    /// Inpaint the image where this is white.
    #[arg(short, long, value_name = "FILE")]
    mask: Option<PathBuf>,

    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    #[arg(short, long, value_name = "TEXT")]
    prompt: String,

    #[arg(long, value_name = "TEXT")]
    negative_prompt: Option<String>,

    #[arg(long, default_value_t = 1.0)]
    true_cfg_scale: f32,

    #[arg(long)]
    width: Option<u32>,

    #[arg(long)]
    height: Option<u32>,

    #[arg(long, default_value_t = 40)]
    steps: u32,

    #[arg(long, default_value_t = 0.6)]
    strength: f64,

    #[arg(long)]
    padding_mask_crop: Option<u32>,

    #[arg(long, default_value_t = -1)]
    seed: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let images = cli
        .image
        .iter()
        .map(image::open)
        .collect::<Result<Vec<_>, _>>()?;

    let inpainted = match (&cli.mask, images.as_slice()) {
        (Some(mask), [input]) => Some((input, image::open(mask)?)),
        (Some(_), _) => bail!("inpainting takes one image"),
        (None, _) => None,
    };

    let device = koharu_runtime::device().await?;
    let pipeline = QwenImage::load(device).await?;
    if let Some((input, mask)) = inpainted {
        let options = InpaintOptions {
            padding_mask_crop: cli.padding_mask_crop,
            strength: cli.strength,
            num_inference_steps: usize::try_from(cli.steps)?,
            seed: cli.seed,
        };
        pipeline
            .inpaint(&cli.prompt, input, &mask, &options)?
            .save(cli.output)?;
        return Ok(());
    }
    let options = Options {
        negative_prompt: cli.negative_prompt,
        true_cfg_scale: cli.true_cfg_scale,
        width: cli.width,
        height: cli.height,
        num_inference_steps: cli.steps,
        seed: cli.seed,
        ..Options::default()
    };
    pipeline
        .generate(&cli.prompt, &images, &options)?
        .into_iter()
        .next()
        .context("Qwen-Image 2.1 returned no image")?
        .save(cli.output)?;

    Ok(())
}
