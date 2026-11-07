//! LaMa inpainting, run as IOPaint runs it: Big-LaMa's FFC ResNet generator,
//! fine-tuned on manga, given the masked regions of a page.
//!
//! Upstream:
//! https://github.com/advimman/lama/tree/786f5936b27fb3dacd2b1ad799e4de968ea697e7
//! https://github.com/Sanster/IOPaint/blob/61a759fb3f332bacdce8b2813f4837495c9b86e0/iopaint/model/base.py

mod config;
mod model;
mod processor;

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, GrayImage, RgbImage};
use koharu_runtime::Repository;
use koharu_torch::{Device, Kind, Tensor};

use self::{config::FFCResNetGeneratorConfig, model::Model};

const REPOSITORY: Repository = Repository::model(
    "mayocream/lama-manga",
    "f91c85b26913b3e83f9877867b4c336da3675238",
);

#[derive(Debug)]
pub struct Lama {
    device: Device,
    model: Model,
}

impl Lama {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::backend::torch(device);
        let [weights] = REPOSITORY
            .resolve(["lama-manga.safetensors"])
            .await
            .context("failed to resolve LaMa's files")?;

        let mut model = Model::new(&FFCResNetGeneratorConfig::default(), device, kind)?;
        model.load(&weights)?;
        Ok(Self { device, model })
    }

    /// `image` repainted where `mask` is white, IOPaint's
    /// `InpaintModel.__call__` at its defaults: a page over 800 pixels on its
    /// long side goes a crop at a time, with a margin of 128 around each masked
    /// region.
    /// https://github.com/Sanster/IOPaint/blob/61a759fb3f332bacdce8b2813f4837495c9b86e0/iopaint/schema.py#L206-L214
    pub fn inference(&self, image: &DynamicImage, mask: &GrayImage) -> Result<RgbImage> {
        let image = image.to_rgb8();
        ensure!(
            image.dimensions() == mask.dimensions(),
            "image and mask dimensions differ: image={:?}, mask={:?}",
            image.dimensions(),
            mask.dimensions()
        );
        ensure!(
            image.width() > 0 && image.height() > 0,
            "image dimensions must be non-zero"
        );
        koharu_torch::inference_mode(|| {
            if image.width().max(image.height()) <= 800 {
                return self.pad_forward(&image, mask);
            }

            let boxes = processor::boxes_from_mask(mask);
            let mut crop_results = Vec::with_capacity(boxes.len());
            for bounding_box in boxes {
                let crop_box =
                    processor::crop_box(image.width(), image.height(), bounding_box, 128);
                let [left, top, right, bottom] = crop_box;
                let crop_image =
                    image::imageops::crop_imm(&image, left, top, right - left, bottom - top)
                        .to_image();
                let crop_mask =
                    image::imageops::crop_imm(mask, left, top, right - left, bottom - top)
                        .to_image();
                crop_results.push((self.pad_forward(&crop_image, &crop_mask)?, crop_box));
            }

            let mut result = image;
            for (crop_result, [left, top, _, _]) in crop_results {
                image::imageops::replace(&mut result, &crop_result, left.into(), top.into());
            }
            Ok(result)
        })
    }

    /// IOPaint's `_pad_forward`: the image and mask padded to a multiple of 8,
    /// through the network, cropped back, and blended over the image by the
    /// mask, its default `sd_keep_unmasked_area`.
    fn pad_forward(&self, image: &RgbImage, mask: &GrayImage) -> Result<RgbImage> {
        let width = image.width();
        let height = image.height();
        let image_tensor = Tensor::from_slice(image.as_raw())?
            .view([i64::from(height), i64::from(width), 3])?
            .to(self.device)?
            .permute([2, 0, 1])?
            .unsqueeze(0)?
            .contiguous(koharu_torch::MemoryFormat::Contiguous)?;
        let mask_tensor = Tensor::from_slice(mask.as_raw())?
            .view([i64::from(height), i64::from(width)])?
            .to(self.device)?
            .unsqueeze(0)?
            .unsqueeze(0)?
            .contiguous(koharu_torch::MemoryFormat::Contiguous)?;
        let model_image =
            processor::pad_img_to_modulo(image_tensor.to_kind(Kind::Float)?.div_scalar(255.0)?, 8)?;
        let model_mask =
            processor::pad_img_to_modulo(mask_tensor.gt_scalar(0.0)?.to_kind(Kind::Float)?, 8)?;
        let output = (self
            .model
            .forward(&model_image, &model_mask)?
            .narrow(2, 0, i64::from(height))?
            .narrow(3, 0, i64::from(width))?
            .clamp(
                Some(koharu_torch::Scalar::from(0.0)),
                Some(koharu_torch::Scalar::from(1.0)),
            )?)
        .mul_scalar(255.0)?;
        let output = output.to_kind(Kind::Uint8)?;
        let alpha = mask_tensor.to_kind(Kind::Float)?.div_scalar(255.0)?;
        let output = (output.to_kind(Kind::Float)?.mul_tensor(&alpha)?)
            .add_tensor(
                &image_tensor
                    .to_kind(Kind::Float)?
                    .mul_tensor(&alpha.ones_like(None, None)?.sub_tensor(&alpha, 1)?)?,
                1,
            )?
            .to_kind(Kind::Uint8)?;
        processor::post_process(&output, width, height)
    }
}
