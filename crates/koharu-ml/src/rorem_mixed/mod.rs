//! RORem mixed-resolution SDXL inpainting through stable-diffusion.cpp.
//!
//! Converted model layout and recommended sampling settings:
//! https://huggingface.co/mayocream/RORem-mixed-GGUF/tree/62c75b3e6f078a19e2698b0f677e8a4aa4c9ea56
//! Original preprocessing and inference:
//! https://github.com/leeruibin/RORem/blob/891ab8a17bbcfb16a773c078cb256aae8cb30468/inference_RORem.py

mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GenericImageView, GrayImage, RgbImage};
use koharu_diffusion::{ImageParams, SampleMethod, Scheduler};
use koharu_runtime::Repository;

use self::model::{Model, ModelPaths};

const ROREM: Repository = Repository::model(
    "mayocream/RORem-mixed-GGUF",
    "62c75b3e6f078a19e2698b0f677e8a4aa4c9ea56",
);
const SDXL_INPAINTING: Repository = Repository::model(
    "diffusers/stable-diffusion-xl-1.0-inpainting-0.1",
    "115134f363124c53c7d878647567d04daf26e41e",
);

/// The converted model card's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub prompt: String,
    pub negative_prompt: String,
    /// Square inference resolution. RORem mixed was trained at 512 and 1024.
    pub resolution: u32,
    /// Mask growth in model-space pixels.
    pub mask_dilation: u8,
    pub num_inference_steps: i32,
    pub guidance_scale: f32,
    pub strength: f32,
    /// sd.cpp's seed; -1 picks one at random.
    pub seed: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            prompt: "clean manga illustration, crisp black line art, flat colors, seamless original background, clean white speech bubble, no text".to_owned(),
            negative_prompt: "text, letters, words, symbols, watermark, signature, blurry, smudged, dirty, gray artifacts, extra objects, photorealistic".to_owned(),
            resolution: 512,
            mask_dilation: 0,
            num_inference_steps: 30,
            guidance_scale: 8.0,
            strength: 0.999,
            seed: -1,
        }
    }
}

#[derive(Debug)]
pub struct RoremMixed {
    model: Model,
}

impl RoremMixed {
    /// The files `load` puts on the device, fetched first if the store doesn't
    /// have them.
    pub async fn weights() -> Result<Vec<std::path::PathBuf>> {
        let paths = Self::paths().await?;
        Ok(vec![
            paths.diffusion_model,
            paths.version_marker,
            paths.vae,
            paths.clip_l,
            paths.clip_g,
        ])
    }

    /// Loads the model, its weights placed to fit `max_vram` bytes of the
    /// device.
    pub async fn load(device: &crate::Device, max_vram: u64) -> Result<Self> {
        let paths = Self::paths().await?;
        let model = Model::new(device, max_vram, &paths)?;
        Ok(Self { model })
    }

    async fn paths() -> Result<ModelPaths> {
        let paths = async {
            let [diffusion_model, version_marker] = ROREM
                .resolve([
                    "rorem-mixed-unet-q4_K.gguf",
                    "sdxl-version-marker.safetensors",
                ])
                .await?;
            let [vae, clip_l, clip_g] = SDXL_INPAINTING
                .resolve([
                    "vae/diffusion_pytorch_model.fp16.safetensors",
                    "text_encoder/model.fp16.safetensors",
                    "text_encoder_2/model.fp16.safetensors",
                ])
                .await?;
            anyhow::Ok(ModelPaths {
                diffusion_model,
                version_marker,
                vae,
                clip_l,
                clip_g,
            })
        };
        paths.await.context("failed to resolve RORem mixed's files")
    }

    /// `image` repainted where `mask` is white.
    pub fn inference(
        &self,
        image: &DynamicImage,
        mask: &GrayImage,
        options: &Options,
    ) -> Result<RgbImage> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "image dimensions must be non-zero"
        );
        ensure!(
            image.dimensions() == mask.dimensions(),
            "image and mask dimensions differ: image={:?}, mask={:?}",
            image.dimensions(),
            mask.dimensions()
        );
        ensure!(
            matches!(options.resolution, 512 | 1024),
            "RORem mixed resolution must be 512 or 1024"
        );
        ensure!(
            options.num_inference_steps > 0,
            "num_inference_steps must be greater than zero"
        );
        ensure!(
            options.guidance_scale.is_finite() && options.guidance_scale > 0.0,
            "guidance_scale must be finite and greater than zero"
        );
        ensure!(
            options.strength.is_finite() && options.strength > 0.0 && options.strength < 1.0,
            "RORem mixed strength must be finite, greater than zero, and less than one"
        );
        ensure!(
            !options.prompt.contains('\0'),
            "prompt contains an interior NUL byte"
        );
        ensure!(
            !options.negative_prompt.contains('\0'),
            "negative prompt contains an interior NUL byte"
        );

        let original = image.to_rgb8();
        let init_image = processor::resize_image(&original, options.resolution)?;
        let native_mask = processor::resize_mask(mask, options.resolution, options.mask_dilation)?;
        if native_mask.as_raw().iter().all(|&value| value == 0) {
            return Ok(original);
        }

        let generated = self
            .model
            .forward(&ImageParams {
                prompt: &options.prompt,
                negative_prompt: &options.negative_prompt,
                init: Some(&init_image),
                mask: Some(&native_mask),
                width: Some(options.resolution),
                height: Some(options.resolution),
                cfg: Some(options.guidance_scale),
                scheduler: Some(Scheduler::Discrete),
                sample_method: Some(SampleMethod::Euler),
                steps: Some(u32::try_from(options.num_inference_steps)?),
                strength: Some(options.strength),
                seed: Some(options.seed),
                count: Some(1),
                ..ImageParams::default()
            })?
            .into_iter()
            .next()
            .context("RORem mixed returned no inpainted image")?;
        ensure!(
            generated.dimensions() == (options.resolution, options.resolution),
            "RORem mixed returned an unexpected image size: {:?}",
            generated.dimensions()
        );

        processor::composite(&original, &native_mask, &generated)
    }
}
