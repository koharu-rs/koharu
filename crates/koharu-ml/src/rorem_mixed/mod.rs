//! RORem mixed-resolution SDXL inpainting through stable-diffusion.cpp.
//!
//! Converted model layout and recommended sampling settings:
//! https://huggingface.co/mayocream/RORem-mixed-GGUF/tree/62c75b3e6f078a19e2698b0f677e8a4aa4c9ea56
//! Original preprocessing and inference:
//! https://github.com/leeruibin/RORem/blob/891ab8a17bbcfb16a773c078cb256aae8cb30468/inference_RORem.py

mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GrayImage, RgbImage};
use koharu_diffusion::{
    GuidanceParams, ImageGenerationParams, SampleMethod, SampleParams, Scheduler,
};

use self::{
    model::{Model, ModelPaths},
    processor::Processor,
};

pub use self::processor::RoremMixedOptions;

pub const DEFAULT_PROMPT: &str = "clean manga illustration, crisp black line art, flat colors, seamless original background, clean white speech bubble, no text";
pub const DEFAULT_NEGATIVE_PROMPT: &str = "text, letters, words, symbols, watermark, signature, blurry, smudged, dirty, gray artifacts, extra objects, photorealistic";

crate::model_repository!("mayocream/RORem-mixed-GGUF" @ "62c75b3e6f078a19e2698b0f677e8a4aa4c9ea56" {
    DIFFUSION_MODEL = "rorem-mixed-unet-q4_K.gguf",
    SDXL_VERSION_MARKER = "sdxl-version-marker.safetensors",
});
crate::model_repository!("diffusers/stable-diffusion-xl-1.0-inpainting-0.1" @ "115134f363124c53c7d878647567d04daf26e41e" {
    VAE_MODEL = "vae/diffusion_pytorch_model.fp16.safetensors",
    CLIP_L_MODEL = "text_encoder/model.fp16.safetensors",
    CLIP_G_MODEL = "text_encoder_2/model.fp16.safetensors",
});

#[derive(Debug)]
pub struct RoremMixed {
    model: Model,
}

impl RoremMixed {
    pub async fn load(device: crate::Device) -> Result<Self> {
        let (diffusion_model, version_marker, vae, clip_l, clip_g) = tokio::try_join!(
            DIFFUSION_MODEL.resolve(),
            SDXL_VERSION_MARKER.resolve(),
            VAE_MODEL.resolve(),
            CLIP_L_MODEL.resolve(),
            CLIP_G_MODEL.resolve(),
        )
        .context("failed to resolve RORem mixed model assets")?;
        let model = Model::new(
            &device,
            ModelPaths {
                diffusion_model,
                version_marker,
                vae,
                clip_l,
                clip_g,
            },
        )?;
        Ok(Self { model })
    }

    pub fn inference(
        &self,
        image: &DynamicImage,
        mask: &GrayImage,
        prompt: &str,
        negative_prompt: &str,
        options: &RoremMixedOptions,
    ) -> Result<RgbImage> {
        Processor::validate(image, mask, prompt, negative_prompt, options)?;

        let original = image.to_rgb8();
        let init_image = Processor::resize_image(&original, options.resolution)?;
        let native_mask = Processor::resize_mask(mask, options.resolution, options.mask_dilation)?;
        if native_mask.as_raw().iter().all(|&value| value == 0) {
            return Ok(original);
        }

        let generated = self
            .model
            .forward(&ImageGenerationParams {
                prompt: prompt.to_owned(),
                negative_prompt: negative_prompt.to_owned(),
                init_image: Some(init_image),
                mask_image: Some(native_mask.clone()),
                width: i32::try_from(options.resolution)?,
                height: i32::try_from(options.resolution)?,
                sample: SampleParams {
                    guidance: GuidanceParams {
                        text_cfg: options.guidance_scale,
                        ..GuidanceParams::default()
                    },
                    scheduler: Scheduler::Discrete,
                    sample_method: SampleMethod::Euler,
                    sample_steps: options.num_inference_steps,
                    ..SampleParams::default()
                },
                strength: options.strength,
                seed: options.seed,
                batch_count: 1,
                ..ImageGenerationParams::default()
            })?
            .into_iter()
            .next()
            .context("RORem mixed returned no inpainted image")?;
        ensure!(
            generated.dimensions() == (options.resolution, options.resolution),
            "RORem mixed returned an unexpected image size: {:?}",
            generated.dimensions()
        );

        Processor::composite(&original, &native_mask, &generated)
    }
}
