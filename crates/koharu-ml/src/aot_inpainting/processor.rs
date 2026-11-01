//! BallonsTranslator's AOT preprocessing and postprocessing.
//!
//! https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/modules/inpaint/inpaint_default.py#L124-L176
//! https://github.com/dmMaze/BallonsTranslator/blob/4bcc635c19f6c63a902872cf77b3d554e14ed1b7/ballontranslator/utils/imgproc_utils.py#L132-L164

use anyhow::{Context, Result, bail};
use fast_image_resize::{FilterType, ResizeAlg};
use image::{GrayImage, RgbImage};
use koharu_torch::{Device, Kind, Tensor};

/// BallonsTranslator resizes with OpenCV's `INTER_LINEAR`.
const RESIZE: ResizeAlg = ResizeAlg::Interpolation(FilterType::Bilinear);

/// The network's input: the image in [-1, 1] and the binary mask, each at
/// least 128 on a side.
pub(super) struct Input {
    pub(super) image: Tensor,
    pub(super) mask: Tensor,
    /// What reflection padding added below and to the right.
    padding: (u32, u32),
}

/// The image fitted under `max_side`, padded with its reflection to at least
/// 128 on each side, the masked area blanked.
pub(super) fn preprocess(
    image: &RgbImage,
    mask: &GrayImage,
    max_side: u32,
    device: Device,
) -> Result<Input> {
    let (input_width, input_height) = if image.width().max(image.height()) > max_side {
        resize_dimensions(image.width(), image.height(), max_side)
    } else {
        image.dimensions()
    };
    let input_image = crate::image::resize(image, input_width, input_height, RESIZE)?;
    let input_mask = crate::image::resize(mask, input_width, input_height, RESIZE)?;

    let pad_bottom = 128_u32.saturating_sub(input_height);
    let pad_right = 128_u32.saturating_sub(input_width);
    let image_tensor = ((Tensor::f_from_slice(input_image.as_raw())?
        .f_view([i64::from(input_height), i64::from(input_width), 3])?
        .f_permute([2, 0, 1])?
        .f_unsqueeze(0)?
        .f_to_device(device)?
        .f_to_kind(Kind::Float)?)
    .f_div_scalar(127.5)?)
    .f_sub_scalar(1.0)?;
    let mask_tensor = (Tensor::f_from_slice(input_mask.as_raw())?
        .f_view([i64::from(input_height), i64::from(input_width)])?
        .f_unsqueeze(0)?
        .f_unsqueeze(0)?
        .f_to_device(device)?
        .f_to_kind(Kind::Float)?)
    .f_div_scalar(255.0)?;
    let image_tensor = pad_bottom_right_reflect(image_tensor, pad_bottom, pad_right)?;
    let mask_tensor = pad_bottom_right_reflect(mask_tensor, pad_bottom, pad_right)?
        .f_ge(0.5)?
        .f_to_kind(Kind::Float)?;
    Ok(Input {
        image: image_tensor.f_mul(&mask_tensor.f_ones_like()?.f_sub(&mask_tensor)?)?,
        mask: mask_tensor,
        padding: (pad_bottom, pad_right),
    })
}

/// The network's `output` without the padding, at `image`'s size, with
/// `image`'s pixels wherever `mask` is below half.
pub(super) fn postprocess(
    output: &Tensor,
    input: &Input,
    image: &RgbImage,
    mask: &GrayImage,
) -> Result<RgbImage> {
    let (pad_bottom, pad_right) = input.padding;
    let output_height = output.size()[2] - i64::from(pad_bottom);
    let output_width = output.size()[3] - i64::from(pad_right);
    if output_height <= 0 || output_width <= 0 {
        bail!(
            "AOT output is too small after removing padding: {:?}",
            output.size()
        );
    }
    let output = output
        .f_narrow(2, 0, output_height)?
        .f_narrow(3, 0, output_width)?;
    let output = tensor_to_rgb(&output, output_width as u32, output_height as u32)?;
    let mut output = crate::image::resize(&output, image.width(), image.height(), RESIZE)?;
    for (index, &value) in mask.as_raw().iter().enumerate() {
        if value < 127 {
            let offset = index * 3;
            output.as_flat_samples_mut().samples[offset..offset + 3]
                .copy_from_slice(&image.as_raw()[offset..offset + 3]);
        }
    }
    Ok(output)
}

fn resize_dimensions(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let ratio = max_side as f64 / width.max(height) as f64;
    (
        ((width as f64 * ratio).round_ties_even() as i64).max(1) as u32,
        ((height as f64 * ratio).round_ties_even() as i64).max(1) as u32,
    )
}

fn pad_bottom_right_reflect(tensor: Tensor, pad_bottom: u32, pad_right: u32) -> Result<Tensor> {
    if pad_bottom == 0 && pad_right == 0 {
        return Ok(tensor);
    }
    let height = tensor.size()[2] as u32;
    let width = tensor.size()[3] as u32;
    let height_indices = (0..height + pad_bottom)
        .map(|index| i64::from(crate::image::symmetric_index(index, height)))
        .collect::<Vec<_>>();
    let width_indices = (0..width + pad_right)
        .map(|index| i64::from(crate::image::symmetric_index(index, width)))
        .collect::<Vec<_>>();
    Ok(tensor
        .f_index_select(
            2,
            &Tensor::f_from_slice(&height_indices)?.f_to_device(tensor.device())?,
        )?
        .f_index_select(
            3,
            &Tensor::f_from_slice(&width_indices)?.f_to_device(tensor.device())?,
        )?)
}

fn tensor_to_rgb(tensor: &Tensor, width: u32, height: u32) -> Result<RgbImage> {
    let tensor = tensor
        .f_to_kind(Kind::Float)?
        .f_add_scalar(1.0)?
        .f_mul_scalar(127.5)?
        .f_round()?
        .f_clamp(0.0, 255.0)?
        .f_to_kind(Kind::Uint8)?
        .f_squeeze_dim(0)?
        .f_permute([1, 2, 0])?
        .f_contiguous()?
        .f_to_device(Device::Cpu)?
        .f_view([-1])?;
    let pixels = Vec::<u8>::try_from(&tensor)?;
    RgbImage::from_raw(width, height, pixels).context("failed to convert AOT output to RGB image")
}
