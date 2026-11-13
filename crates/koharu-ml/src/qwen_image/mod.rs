//! Qwen-Image 2.1 generation, editing, and inpainting.
//!
//! Follows diffusers' `QwenImage21Pipeline` at its defaults: 40 flow-matching
//! Euler steps on the resolution-dependent shift, no guidance, and a
//! 1024-pixel side, the schedule of which stable-diffusion.cpp's Qwen-Image
//! 2.1 applies itself.
//! https://github.com/huggingface/diffusers/blob/74fb49b5506a6960d55de0f7e6f4e0631213dc63/src/diffusers/pipelines/qwenimage21/pipeline_qwenimage21.py
//! Editing is generation conditioned on images, which the model reads in the
//! order given. A mask or a painted annotation is one more condition image
//! the prompt names, as the model card describes local edits.
//! https://huggingface.co/Qwen/Qwen-Image-2.1/blob/d26bb61231c349cf6b7896fa83353113880e1ba3/README.md
//! Qwen-Image 2.1 has no inpainting pipeline, so inpainting follows diffusers'
//! Qwen-Image-Edit one, flow-matching img2img with a mask and the image as the
//! edit's condition, on 2.1's own steps and sizes.
//! https://github.com/huggingface/diffusers/blob/74fb49b5506a6960d55de0f7e6f4e0631213dc63/src/diffusers/pipelines/qwenimage/pipeline_qwenimage_edit_inpaint.py

mod model;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, RgbImage, RgbaImage};
use koharu_diffusion::{ImageParams, SampleMethod};
use koharu_runtime::Repository;

use self::model::{Model, ModelPaths};
use crate::diffusers;

/// leejet's conversion of Qwen-Image 2.1 for stable-diffusion.cpp.
const TRANSFORMER: Repository = Repository::model(
    "leejet/Qwen-Image-2.1-GGUF",
    "cc11433936a06e9765f7c0c0b1f0436cfd2b9856",
);
/// Qwen-Image 2.1's own VAE, which stable-diffusion.cpp's guide names; the
/// earlier Qwen-Image and Wan 2.2 VAEs don't stand in for it.
const VAE: Repository = Repository::model(
    "Comfy-Org/Qwen-Image-2.1",
    "aacf31a6d48e707f352ecba714cee0680da3479b",
);
/// Qwen's own GGUF of the Qwen3-VL 8B text encoder, with the vision projector
/// that reads condition images.
const TEXT_ENCODER: Repository = Repository::model(
    "Qwen/Qwen3-VL-8B-Instruct-GGUF",
    "f982a07559d4a2f6c8744d840bf6fccab30eea96",
);

/// The VAE's 16× downscale by the transformer's 2×2 patches: sizes are
/// multiples of it.
const MULTIPLE: u32 = 32;
/// The area `QwenImageEditInpaintPipeline` sizes an image to inpaint to.
const AREA: u32 = 1024 * 1024;

/// `QwenImage21Pipeline.__call__`'s parameters and defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Steers away with classifier-free guidance, when `true_cfg_scale` is
    /// above 1.
    pub negative_prompt: Option<String>,
    pub true_cfg_scale: f32,
    /// Unset, `output_resolution`'s side, or the last condition image's
    /// aspect ratio at its area; rounded down to a multiple of 32.
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub num_inference_steps: u32,
    pub num_images_per_prompt: u32,
    /// sd.cpp's seed; -1 picks one at random.
    pub seed: i64,
    /// The side the output and every condition image are sized to the area of.
    pub output_resolution: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            negative_prompt: None,
            true_cfg_scale: 1.0,
            height: None,
            width: None,
            num_inference_steps: 40,
            num_images_per_prompt: 1,
            seed: -1,
            output_resolution: 1024,
        }
    }
}

/// `QwenImageEditInpaintPipeline.__call__`'s parameters that apply here,
/// with 2.1's own number of steps in place of Qwen-Image-Edit's 50.
#[derive(Debug, Clone, PartialEq)]
pub struct InpaintOptions {
    /// The margin around the mask an inpainting is cropped to, or the whole
    /// image.
    pub padding_mask_crop: Option<u32>,
    pub strength: f64,
    pub num_inference_steps: usize,
    /// sd.cpp's seed; -1 picks one at random.
    pub seed: i64,
}

impl Default for InpaintOptions {
    fn default() -> Self {
        Self {
            padding_mask_crop: None,
            strength: 0.6,
            num_inference_steps: 40,
            seed: -1,
        }
    }
}

#[derive(Debug)]
pub struct QwenImage {
    model: Model,
}

impl QwenImage {
    /// Loads the model once for generation and editing, its weights placed by
    /// stable-diffusion.cpp's own fit to the device's free memory.
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let paths = async {
            // 4-bit, as Koharu's other diffusion models load.
            let [transformer] = TRANSFORMER.resolve(["qwen_image_2.1-Q4_K.gguf"]).await?;
            let [vae] = VAE
                .resolve(["vae/qwen_image_2.1_vae_bf16.safetensors"])
                .await?;
            let [text_encoder, vision] = TEXT_ENCODER
                .resolve([
                    "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
                    "mmproj-Qwen3VL-8B-Instruct-F16.gguf",
                ])
                .await?;
            anyhow::Ok(ModelPaths {
                transformer,
                text_encoder,
                vision,
                vae,
            })
        }
        .await
        .context("failed to resolve Qwen-Image 2.1's files")?;
        let model = Model::new(device, &paths)?;
        Ok(Self { model })
    }

    /// Images of `prompt`, conditioned on `images` when there are any. They
    /// carry alpha, which the prompt asks for as the model card describes.
    pub fn generate(
        &self,
        prompt: &str,
        images: &[DynamicImage],
        options: &Options,
    ) -> Result<Vec<RgbaImage>> {
        ensure!(
            !prompt.contains('\0'),
            "prompt contains an interior NUL byte"
        );
        ensure!(
            options.num_inference_steps > 0
                && options.num_images_per_prompt > 0
                && options.output_resolution > 0,
            "num_inference_steps, num_images_per_prompt, and output_resolution must be greater than zero"
        );
        ensure!(
            options.true_cfg_scale.is_finite(),
            "true_cfg_scale must be finite"
        );
        let area = options.output_resolution * options.output_resolution;
        let (width, height) = match images.last() {
            Some(last) => {
                let (width, height) = dimensions(area, last.width(), last.height());
                (
                    options.width.unwrap_or(width),
                    options.height.unwrap_or(height),
                )
            }
            None => (
                options.width.unwrap_or(options.output_resolution),
                options.height.unwrap_or(options.output_resolution),
            ),
        };
        let (width, height) = (width / MULTIPLE * MULTIPLE, height / MULTIPLE * MULTIPLE);
        ensure!(
            width > 0 && height > 0,
            "height and width must be at least {MULTIPLE}"
        );

        // One resize feeds both the text encoder and the VAE. The images go as
        // RGB, which sd.cpp gives an opaque alpha, as diffusers' `RGBA`
        // conversion does.
        let references = images
            .iter()
            .map(|image| {
                let (width, height) = dimensions(area, image.width(), image.height());
                crate::image::resize(&image.to_rgb8(), width, height, diffusers::LANCZOS)
            })
            .collect::<Result<Vec<RgbImage>>>()?;
        let negative_prompt = options.negative_prompt.as_deref();
        let guided = options.true_cfg_scale > 1.0 && negative_prompt.is_some();
        self.model.forward(&ImageParams {
            prompt,
            negative_prompt: negative_prompt.unwrap_or_default(),
            width: Some(width),
            height: Some(height),
            references: &references,
            cfg: Some(if guided { options.true_cfg_scale } else { 1.0 }),
            sample_method: Some(SampleMethod::Euler),
            steps: Some(options.num_inference_steps),
            seed: Some(options.seed),
            count: Some(options.num_images_per_prompt),
            ..ImageParams::default()
        })
    }

    /// `image` repainted where `mask` is white, sized to a megapixel at its
    /// aspect ratio, with what's inpainted as the edit's condition. The prompt
    /// may be empty.
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
            "Qwen-Image 2.1 inpaint strength must be greater than zero and at most one"
        );
        ensure!(
            options.num_inference_steps > 0,
            "num_inference_steps must be greater than zero"
        );
        let (width, height) = dimensions(AREA, image.width(), image.height());
        ensure!(
            width > 0 && height > 0,
            "the image is too narrow to inpaint at {MULTIPLE} pixels a side"
        );

        let image = crate::image::resize(&image.to_rgb8(), width, height, diffusers::LANCZOS)?;
        let mask = crate::image::resize(&mask.to_luma8(), width, height, diffusers::LANCZOS)?;
        let region = options
            .padding_mask_crop
            .and_then(|padding| diffusers::crop_region(&mask, width, height, padding));
        let (init_image, mut native_mask) = diffusers::crop_to_region(&image, &mask, region)?;
        diffusers::binarize(&mut native_mask);

        // Upstream conditions the text encoder on the image before its crop
        // and the transformer on the crop; sd.cpp takes one reference for
        // both, the crop, which is the whole image unless one is asked for.
        let generated = self
            .model
            .forward(&ImageParams {
                prompt,
                width: Some(width),
                height: Some(height),
                init: Some(&init_image),
                references: std::slice::from_ref(&init_image),
                mask: Some(&native_mask),
                cfg: Some(1.0),
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
            .context("Qwen-Image 2.1 returned no inpainted image")?;
        let generated = DynamicImage::ImageRgba8(generated).into_rgb8();
        // Upstream lays the result over the image only for a crop.
        match region {
            Some(_) => diffusers::apply_overlay(&mask, &image, generated, region),
            None => Ok(generated),
        }
    }
}

/// `calculate_dimensions`: `target_area` at the image's aspect ratio, each
/// side rounded to a multiple of 32 as Python's `round` rounds, half to even.
fn dimensions(target_area: u32, image_width: u32, image_height: u32) -> (u32, u32) {
    let ratio = f64::from(image_width) / f64::from(image_height);
    let width = (f64::from(target_area) * ratio).sqrt();
    let height = width / ratio;
    let round = |side: f64| ((side / f64::from(MULTIPLE)).round_ties_even() as u32) * MULTIPLE;
    (round(width), round(height))
}
