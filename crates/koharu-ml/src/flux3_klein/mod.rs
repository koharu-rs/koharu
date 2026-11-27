//! FLUX.3 Image Klein generation, which edits when it's given references,
//! following Black Forest Labs' reference implementation:
//! https://github.com/black-forest-labs/flux-3-image-klein/blob/62e90b9278b73ecc52927770c6965dc6b1966498/src/flux3/generate.py
//! https://github.com/black-forest-labs/flux-3-image-klein/blob/62e90b9278b73ecc52927770c6965dc6b1966498/src/flux3/pipeline.py

mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, RgbImage, imageops};
use koharu_diffusion::{Canceller, ImageParams, SampleMethod};
use koharu_runtime::{PinnedFile, Repository};

use self::model::{Model, ModelPaths};

const TRANSFORMER: Repository = Repository::model(
    "unsloth/flux-3-image-klein-GGUF",
    "12ef13ce920d53f45fe805093910c899668b9e76",
);
/// FLUX.3 encodes and decodes with the FLUX.2 autoencoder.
const DECODER: Repository = Repository::model(
    "black-forest-labs/FLUX.2-small-decoder",
    "a3efc24f613ef42d9428af62fdbd6f5fd8856c4a",
);
const TEXT_ENCODER: Repository = Repository::model(
    "Qwen/Qwen3-VL-4B-Instruct-GGUF",
    "1cd86afb9a95c410a6038ab3b40d8b578c892266",
);

pub const FILES: &[PinnedFile] = &[
    PinnedFile::new(
        TRANSFORMER,
        "flux-3-image-klein-UD-Q4_K_XL.gguf",
        3_929_944_992,
    ),
    PinnedFile::new(
        TEXT_ENCODER,
        "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
        2_497_281_664,
    ),
    PinnedFile::new(
        DECODER,
        "full_encoder_small_decoder.safetensors",
        249_519_092,
    ),
];

/// The flow map's time shift, `alpha` in the reference's `Pipeline.generate`.
/// sd.cpp's default for this model, 6.93, is the reference's at release
/// (45e57c9fb06f9751cdced29a65c5d2171003be05); it has been 16 since 62e90b9.
const FLOW_SHIFT: f32 = 16.0;
/// `MAX_REFERENCES` upstream.
const MAX_REFERENCES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateOptions {
    /// The output's size, or the reference's, or 1024.
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub num_inference_steps: i32,
    /// sd.cpp's seed; -1 picks one at random.
    pub seed: i64,
    pub num_images_per_prompt: i32,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            height: None,
            width: None,
            // The distilled checkpoint's steps, `MODELS["distilled"]` upstream.
            num_inference_steps: 16,
            seed: -1,
            num_images_per_prompt: 1,
        }
    }
}

#[derive(Debug)]
pub struct Flux3Klein {
    model: Model,
}

impl Flux3Klein {
    /// What stops a generation under way, from another thread.
    pub fn canceller(&self) -> Canceller {
        self.model.canceller.clone()
    }

    /// Loads the model once for generation and editing, its weights
    /// placed by stable-diffusion.cpp's own fit to the device's free memory.
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let paths = async {
            // The model card's recommended file.
            let [transformer] = TRANSFORMER.resolve([FILES[0].filename]).await?;
            let [text_encoder] = TEXT_ENCODER.resolve([FILES[1].filename]).await?;
            let [vae] = DECODER.resolve([FILES[2].filename]).await?;
            anyhow::Ok(ModelPaths {
                transformer,
                text_encoder,
                vae,
            })
        }
        .await
        .context("failed to resolve FLUX.3 Image Klein's files")?;
        let model = Model::new(device, &paths)?;
        Ok(Self { model })
    }

    /// Images of `prompt`, conditioned on `references`.
    pub fn generate(
        &self,
        references: &[DynamicImage],
        prompt: &str,
        options: &GenerateOptions,
    ) -> Result<Vec<RgbImage>> {
        ensure!(
            !prompt.contains('\0'),
            "prompt contains an interior NUL byte"
        );
        ensure!(
            options.num_inference_steps > 0,
            "num_inference_steps must be greater than zero"
        );
        ensure!(
            options.num_images_per_prompt > 0,
            "num_images_per_prompt must be greater than zero"
        );
        ensure!(
            references.len() <= MAX_REFERENCES,
            "FLUX.3 Image Klein takes at most {MAX_REFERENCES} reference images"
        );

        let (references, region) = processor::references(references)?;
        let height = options
            .height
            .or_else(|| references.first().map(RgbImage::height))
            .unwrap_or(1024);
        let width = options
            .width
            .or_else(|| references.first().map(RgbImage::width))
            .unwrap_or(1024);
        let height = (height / 16) * 16;
        let width = (width / 16) * 16;
        ensure!(width > 0 && height > 0);

        let images = self.model.forward(&ImageParams {
            prompt,
            width: Some(width),
            height: Some(height),
            references: &references,
            reference_args: Some("resize_before_vae=0"),
            cfg: Some(1.0),
            flow_shift: Some(FLOW_SHIFT),
            sample_method: Some(SampleMethod::Euler),
            steps: Some(u32::try_from(options.num_inference_steps)?),
            seed: Some(options.seed),
            count: Some(u32::try_from(options.num_images_per_prompt)?),
            ..ImageParams::default()
        })?;
        // Upstream crops back only when the size is the padded reference's.
        Ok(match region {
            Some((left, top, right, bottom))
                if options.width.is_none() && options.height.is_none() =>
            {
                images
                    .iter()
                    .map(|image| {
                        imageops::crop_imm(image, left, top, right - left, bottom - top).to_image()
                    })
                    .collect()
            }
            _ => images,
        })
    }
}
