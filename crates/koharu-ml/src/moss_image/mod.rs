//! MOSS-Image generation and inpainting.
//!
//! Generation follows OpenMOSS's pipeline at its defaults: 50 flow-matching
//! Euler steps with a time shift of 5, guidance 3, an empty negative prompt,
//! and the negative branch skipping DiT layer 22, all of which
//! stable-diffusion.cpp's MOSS-Image applies itself.
//! https://huggingface.co/OpenMOSS-Team/MOSS-Image-SFT/blob/f972bd4b4b9e3852133cc5da954d87bc32cd8fb9/pipeline_moss_image.py
//! MOSS-Image doesn't inpaint, so inpainting is diffusers' flow-matching
//! img2img with a mask, as FLUX.3's here is.

mod model;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, RgbImage};
use koharu_diffusion::{ImageParams, SampleMethod};
use koharu_runtime::Repository;

use self::model::{Model, ModelPaths};
use crate::diffusers;

/// leejet's conversion of MOSS-Image SFT, the checkpoint fine-tuned on
/// curated data, for stable-diffusion.cpp.
const TRANSFORMER: Repository = Repository::model(
    "leejet/MOSS-Image-SFT-GGUF",
    "238daf423f0de65316207453591f275407aac9a7",
);
/// MOSS-Image's own VAE and tokenizer. Its VAE is FLUX.2's small decoder with
/// an encoder and latent statistics of its own, so FLUX.2's files, which
/// stable-diffusion.cpp's guide names, don't stand in for it.
const MOSS: Repository = Repository::model(
    "OpenMOSS-Team/MOSS-Image-SFT",
    "f972bd4b4b9e3852133cc5da954d87bc32cd8fb9",
);
/// Qwen3.5 9B at the revision koharu-llm's catalog pins, so one download
/// serves both.
const TEXT_ENCODER: Repository = Repository::model(
    "unsloth/Qwen3.5-9B-GGUF",
    "3885219b6810b007914f3a7950a8d1b469d598a5",
);

/// The pipeline's defaults, `MOSSImagePipeline.__call__`.
const STEPS: u32 = 50;
const GUIDANCE: f32 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateOptions {
    /// Multiples of 16.
    pub width: u32,
    pub height: u32,
    pub num_inference_steps: u32,
    pub guidance_scale: f32,
    /// sd.cpp's seed; -1 picks one at random.
    pub seed: i64,
    pub num_images_per_prompt: u32,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 1024,
            num_inference_steps: STEPS,
            guidance_scale: GUIDANCE,
            seed: -1,
            num_images_per_prompt: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InpaintOptions {
    /// The margin around the mask an inpainting is cropped to, or the whole
    /// image.
    pub padding_mask_crop: Option<u32>,
    /// FLUX.3's here, as MOSS-Image has none of its own.
    pub strength: f64,
    pub num_inference_steps: usize,
    pub guidance_scale: f32,
    /// sd.cpp's seed; -1 picks one at random.
    pub seed: i64,
}

impl Default for InpaintOptions {
    fn default() -> Self {
        Self {
            padding_mask_crop: None,
            strength: 0.8,
            num_inference_steps: STEPS as usize,
            guidance_scale: GUIDANCE,
            seed: -1,
        }
    }
}

#[derive(Debug)]
pub struct MossImage {
    model: Model,
}

impl MossImage {
    /// Loads the model once for generation and inpainting, its weights
    /// placed by stable-diffusion.cpp's own fit to the device's free memory.
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let paths = async {
            // 4-bit, as Koharu's other diffusion models load, and a k-quant,
            // as RORem's is.
            let [transformer] = TRANSFORMER.resolve(["MOSS-Image-SFT-Q4_K.gguf"]).await?;
            // koharu-llm's default quantization of the same model.
            let [text_encoder] = TEXT_ENCODER.resolve(["Qwen3.5-9B-UD-Q4_K_XL.gguf"]).await?;
            let [vae, tokenizer] = MOSS
                .resolve([
                    "vae/diffusion_pytorch_model.safetensors",
                    "tokenizer/tokenizer.json",
                ])
                .await?;
            anyhow::Ok(ModelPaths {
                transformer,
                text_encoder,
                vae,
                tokenizer,
            })
        }
        .await
        .context("failed to resolve MOSS-Image's files")?;
        let model = Model::new(device, &paths)?;
        Ok(Self { model })
    }

    /// Images of `prompt`: a caption, tags, or a JSON layout, as the
    /// pipeline takes them.
    pub fn generate(&self, prompt: &str, options: &GenerateOptions) -> Result<Vec<RgbImage>> {
        ensure!(
            !prompt.contains('\0'),
            "prompt contains an interior NUL byte"
        );
        ensure!(
            options.width > 0
                && options.height > 0
                && options.width.is_multiple_of(diffusers::TOKEN)
                && options.height.is_multiple_of(diffusers::TOKEN),
            "height and width must be positive multiples of 16"
        );
        ensure!(
            options.num_inference_steps > 0 && options.num_images_per_prompt > 0,
            "num_inference_steps and num_images_per_prompt must be greater than zero"
        );
        ensure!(
            options.guidance_scale.is_finite() && options.guidance_scale >= 0.0,
            "guidance_scale must be finite and nonnegative"
        );
        self.model.forward(&ImageParams {
            prompt,
            width: Some(options.width),
            height: Some(options.height),
            cfg: Some(options.guidance_scale),
            sample_method: Some(SampleMethod::Euler),
            steps: Some(options.num_inference_steps),
            seed: Some(options.seed),
            count: Some(options.num_images_per_prompt),
            ..ImageParams::default()
        })
    }

    /// `image` repainted where `mask` is white, at the size it's fitted to
    /// under a megapixel, as FLUX.3's inpainting here does. The prompt may be
    /// empty, to fill from what's around.
    pub fn inpaint(
        &self,
        prompt: &str,
        image: &DynamicImage,
        mask: &DynamicImage,
        options: &InpaintOptions,
    ) -> Result<RgbImage> {
        ensure!(
            image.width() == mask.width() && image.height() == mask.height(),
            "image/mask dimensions differ: image={}x{}, mask={}x{}",
            image.width(),
            image.height(),
            mask.width(),
            mask.height()
        );
        ensure!(
            !prompt.contains('\0'),
            "prompt contains an interior NUL byte"
        );
        ensure!(
            options.strength > 0.0 && options.strength <= 1.0,
            "MOSS-Image inpaint strength must be greater than zero and at most one"
        );
        ensure!(
            options.num_inference_steps > 0,
            "num_inference_steps must be greater than zero"
        );

        let (image, mask) = diffusers::fit_inpaint(image, mask)?;
        let (width, height) = image.dimensions();
        let region = options
            .padding_mask_crop
            .and_then(|padding| diffusers::crop_region(&mask, width, height, padding));
        let (init_image, mut native_mask) = diffusers::crop_to_region(&image, &mask, region)?;
        diffusers::binarize(&mut native_mask);

        let generated = self
            .model
            .forward(&ImageParams {
                prompt,
                width: Some(width),
                height: Some(height),
                init: Some(&init_image),
                mask: Some(&native_mask),
                cfg: Some(options.guidance_scale),
                sample_method: Some(SampleMethod::Euler),
                steps: Some(u32::try_from(options.num_inference_steps)?),
                seed: Some(options.seed),
                count: Some(1),
                strength: Some(diffusers::strength(
                    options.strength,
                    options.num_inference_steps,
                )),
                ..ImageParams::default()
            })?
            .into_iter()
            .next()
            .context("MOSS-Image returned no inpainted image")?;
        diffusers::apply_overlay(&mask, &image, generated, region)
    }
}
