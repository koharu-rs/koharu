//! BallonsTranslator AOT preprocessing and postprocessing.
//!
//! https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/inpaint/inpaint_default.py#L124-L176
//! https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/utils/imgproc_utils.py#L132-L164

use anyhow::{Context, Result, bail, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GrayImage, RgbImage};
use koharu_torch::{Device, Kind, Tensor};

use super::model::Model;

#[derive(Debug)]
pub(super) struct Processor {
    device: Device,
}

impl Processor {
    pub(super) fn new(device: Device) -> Self {
        Self { device }
    }

    pub(super) fn call(
        &self,
        model: &Model,
        image: &DynamicImage,
        mask: &GrayImage,
        max_side: u32,
    ) -> Result<RgbImage> {
        ensure!(max_side > 0, "AOT inpainting max side must be positive");
        let original = image.to_rgb8();
        ensure!(
            original.dimensions() == mask.dimensions(),
            "image and mask dimensions differ: image={:?}, mask={:?}",
            original.dimensions(),
            mask.dimensions()
        );
        ensure!(
            original.width() > 0 && original.height() > 0,
            "image dimensions must be non-zero"
        );

        let original_mask = GrayImage::from_raw(
            mask.width(),
            mask.height(),
            mask.as_raw()
                .iter()
                .map(|&value| u8::from(value >= 127))
                .collect(),
        )
        .context("failed to binarize AOT compositing mask")?;

        let (input_width, input_height) = if original.width().max(original.height()) > max_side {
            resize_dimensions(original.width(), original.height(), max_side)
        } else {
            original.dimensions()
        };
        let input_image = resize_rgb(&original, input_width, input_height)?;
        let input_mask = resize_gray(mask, input_width, input_height)?;

        let pad_bottom = 128_u32.saturating_sub(input_height);
        let pad_right = 128_u32.saturating_sub(input_width);
        let image_tensor = Tensor::from_slice(input_image.as_raw())
            .view([i64::from(input_height), i64::from(input_width), 3])
            .permute([2, 0, 1])
            .unsqueeze(0)
            .to_device(self.device)
            .to_kind(Kind::Float)
            / 127.5
            - 1.0;
        let mask_tensor = Tensor::from_slice(input_mask.as_raw())
            .view([i64::from(input_height), i64::from(input_width)])
            .unsqueeze(0)
            .unsqueeze(0)
            .to_device(self.device)
            .to_kind(Kind::Float)
            / 255.0;
        let image_tensor = pad_bottom_right_reflect(image_tensor, pad_bottom, pad_right);
        let mask_tensor = pad_bottom_right_reflect(mask_tensor, pad_bottom, pad_right)
            .ge(0.5)
            .to_kind(Kind::Float);
        let masked_image = image_tensor * (mask_tensor.ones_like() - &mask_tensor);

        let output = model.forward(&masked_image, &mask_tensor);
        let output_height = output.size()[2] - i64::from(pad_bottom);
        let output_width = output.size()[3] - i64::from(pad_right);
        if output_height <= 0 || output_width <= 0 {
            bail!(
                "AOT output is too small after removing padding: {:?}",
                output.size()
            );
        }
        let output = output
            .narrow(2, 0, output_height)
            .narrow(3, 0, output_width);
        let mut output = tensor_to_rgb(&output, output_width as u32, output_height as u32)?;
        if output.dimensions() != original.dimensions() {
            output = resize_rgb(&output, original.width(), original.height())?;
        }

        for (index, &masked) in original_mask.as_raw().iter().enumerate() {
            if masked == 0 {
                let offset = index * 3;
                output.as_flat_samples_mut().samples[offset..offset + 3]
                    .copy_from_slice(&original.as_raw()[offset..offset + 3]);
            }
        }
        Ok(output)
    }
}

fn resize_dimensions(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let ratio = max_side as f64 / width.max(height) as f64;
    (
        python_round(width as f64 * ratio).max(1) as u32,
        python_round(height as f64 * ratio).max(1) as u32,
    )
}

fn resize_rgb(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    if image.dimensions() == (width, height) {
        return Ok(image.clone());
    }
    let mut output = RgbImage::new(width, height);
    Resizer::new().resize(
        image,
        &mut output,
        &ResizeOptions::new()
            .resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear))
            .use_alpha(false),
    )?;
    Ok(output)
}

fn resize_gray(image: &GrayImage, width: u32, height: u32) -> Result<GrayImage> {
    if image.dimensions() == (width, height) {
        return Ok(image.clone());
    }
    let mut output = GrayImage::new(width, height);
    Resizer::new().resize(
        image,
        &mut output,
        &ResizeOptions::new()
            .resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear))
            .use_alpha(false),
    )?;
    Ok(output)
}

fn pad_bottom_right_reflect(tensor: Tensor, pad_bottom: u32, pad_right: u32) -> Tensor {
    if pad_bottom == 0 && pad_right == 0 {
        return tensor;
    }
    let height = tensor.size()[2] as u32;
    let width = tensor.size()[3] as u32;
    let height_indices = (0..height + pad_bottom)
        .map(|index| i64::from(symmetric_index(index, height)))
        .collect::<Vec<_>>();
    let width_indices = (0..width + pad_right)
        .map(|index| i64::from(symmetric_index(index, width)))
        .collect::<Vec<_>>();
    tensor
        .index_select(
            2,
            &Tensor::from_slice(&height_indices).to_device(tensor.device()),
        )
        .index_select(
            3,
            &Tensor::from_slice(&width_indices).to_device(tensor.device()),
        )
}

fn symmetric_index(index: u32, length: u32) -> u32 {
    let index = index % (length * 2);
    if index < length {
        index
    } else {
        length * 2 - index - 1
    }
}

fn tensor_to_rgb(tensor: &Tensor, width: u32, height: u32) -> Result<RgbImage> {
    let tensor = ((tensor.to_kind(Kind::Float) + 1.0) * 127.5)
        .round()
        .clamp(0.0, 255.0)
        .to_kind(Kind::Uint8)
        .squeeze_dim(0)
        .permute([1, 2, 0])
        .contiguous()
        .to_device(Device::Cpu)
        .view([-1]);
    let pixels = Vec::<u8>::try_from(&tensor)?;
    RgbImage::from_raw(width, height, pixels).context("failed to convert AOT output to RGB image")
}

fn python_round(value: f64) -> i64 {
    let floor = value.floor();
    let fraction = value - floor;
    if (fraction - 0.5).abs() < f64::EPSILON {
        if floor as i64 % 2 == 0 {
            floor as i64
        } else {
            floor as i64 + 1
        }
    } else {
        value.round() as i64
    }
}
