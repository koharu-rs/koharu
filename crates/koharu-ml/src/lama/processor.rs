//! IOPaint's LaMa preprocessing and postprocessing: crop boxes around the
//! mask, symmetric padding to a multiple of 8, and the tensor back to pixels.
//!
//! Original implementations:
//! https://github.com/Sanster/IOPaint/blob/61a759fb3f332bacdce8b2813f4837495c9b86e0/iopaint/model/base.py#L57-L192
//! https://github.com/Sanster/IOPaint/blob/61a759fb3f332bacdce8b2813f4837495c9b86e0/iopaint/helper.py#L187-L267

use anyhow::{Context, Result};
use image::{GrayImage, RgbImage};
use imageproc::contours::{BorderType, find_contours_with_threshold};
use koharu_torch::{Device, Kind, Tensor};

pub(super) fn boxes_from_mask(mask: &GrayImage) -> Vec<[u32; 4]> {
    let width = mask.width();
    let mut left = width;
    let mut top = mask.height();
    let mut right = 0;
    let mut bottom = 0;
    for y in 0..mask.height() {
        let row = &mask.as_raw()[y as usize * width as usize..(y + 1) as usize * width as usize];
        let Some(row_left) = row.iter().position(|value| *value > 127) else {
            continue;
        };
        let row_right = row
            .iter()
            .rposition(|value| *value > 127)
            .expect("masked row must have a right edge");
        left = left.min(row_left as u32);
        top = top.min(y);
        right = right.max(row_right as u32 + 1);
        bottom = y + 1;
    }
    if right <= left || bottom <= top {
        return Vec::new();
    }

    let cropped_width = right - left;
    let cropped_height = bottom - top;
    let padded_width = cropped_width + 2;
    let mut padded = GrayImage::new(padded_width, cropped_height + 2);
    for y in 0..cropped_height as usize {
        let source_start = (top as usize + y) * width as usize + left as usize;
        let target_start = (y + 1) * padded_width as usize + 1;
        padded.as_flat_samples_mut().samples[target_start..target_start + cropped_width as usize]
            .copy_from_slice(&mask.as_raw()[source_start..source_start + cropped_width as usize]);
    }

    find_contours_with_threshold::<u32>(&padded, 127)
        .into_iter()
        .filter(|contour| contour.border_type == BorderType::Outer && contour.parent.is_none())
        .filter_map(|contour| {
            let mut points = contour.points.into_iter();
            let first = points.next()?;
            let mut contour_left = first.x;
            let mut contour_top = first.y;
            let mut contour_right = first.x;
            let mut contour_bottom = first.y;
            for point in points {
                contour_left = contour_left.min(point.x);
                contour_top = contour_top.min(point.y);
                contour_right = contour_right.max(point.x);
                contour_bottom = contour_bottom.max(point.y);
            }
            Some([
                left + contour_left.saturating_sub(1),
                top + contour_top.saturating_sub(1),
                (left + contour_right).min(mask.width()),
                (top + contour_bottom).min(mask.height()),
            ])
        })
        .filter(|[left, top, right, bottom]| right > left && bottom > top)
        .collect()
}

pub(super) fn crop_box(
    image_width: u32,
    image_height: u32,
    [left, top, right, bottom]: [u32; 4],
    margin: u32,
) -> [u32; 4] {
    let image_width = i64::from(image_width);
    let image_height = i64::from(image_height);
    let crop_width = i64::from(right - left) + i64::from(margin) * 2;
    let crop_height = i64::from(bottom - top) + i64::from(margin) * 2;
    let center_x = (i64::from(left) + i64::from(right)) / 2;
    let center_y = (i64::from(top) + i64::from(bottom)) / 2;

    let raw_left = center_x - crop_width / 2;
    let raw_right = center_x + crop_width / 2;
    let raw_top = center_y - crop_height / 2;
    let raw_bottom = center_y + crop_height / 2;

    let mut left = raw_left.max(0);
    let mut right = raw_right.min(image_width);
    let mut top = raw_top.max(0);
    let mut bottom = raw_bottom.min(image_height);

    if raw_left < 0 {
        right += -raw_left;
    }
    if raw_right > image_width {
        left -= raw_right - image_width;
    }
    if raw_top < 0 {
        bottom += -raw_top;
    }
    if raw_bottom > image_height {
        top -= raw_bottom - image_height;
    }

    [
        left.clamp(0, image_width) as u32,
        top.clamp(0, image_height) as u32,
        right.clamp(0, image_width) as u32,
        bottom.clamp(0, image_height) as u32,
    ]
}

pub(super) fn post_process(tensor: &Tensor, width: u32, height: u32) -> Result<RgbImage> {
    let tensor = tensor
        .squeeze_dim(0)?
        .permute([1, 2, 0])?
        .contiguous(koharu_torch::MemoryFormat::Contiguous)?
        .to(Device::Cpu)?
        .view([-1])?;
    let rgb = Vec::<u8>::try_from(&tensor)?;
    RgbImage::from_raw(width, height, rgb).context("failed to convert LaMa tensor to RGB image")
}

pub(super) fn pad_img_to_modulo(tensor: Tensor, modulo: u32) -> Result<Tensor> {
    let height = tensor.size()[2] as u32;
    let width = tensor.size()[3] as u32;
    let out_height = ceil_modulo(height, modulo);
    let out_width = ceil_modulo(width, modulo);
    let height_indices = symmetric_indices(height, out_height, tensor.device())?;
    let width_indices = symmetric_indices(width, out_width, tensor.device())?;
    Ok(tensor
        .index_select(2, &height_indices)?
        .index_select(3, &width_indices)?)
}

fn ceil_modulo(value: u32, modulo: u32) -> u32 {
    if value.is_multiple_of(modulo) {
        value
    } else {
        (value / modulo + 1) * modulo
    }
}

fn symmetric_indices(length: u32, output_length: u32, device: Device) -> Result<Tensor> {
    if !device.is_cuda() {
        let indices = (0..output_length)
            .map(|index| i64::from(crate::image::symmetric_index(index, length)))
            .collect::<Vec<_>>();
        return Ok(Tensor::from_slice(&indices)?.to(device)?);
    }
    let length = i64::from(length);
    let period = length * 2;
    let indices = Tensor::arange(i64::from(output_length), (Kind::Int64, device))?;
    let indices =
        indices.sub_tensor(&indices.floor_divide_scalar(period)?.mul_scalar(period)?, 1)?;
    Ok(indices.where_self(
        &indices.lt_scalar(length)?,
        &indices.sub_scalar(period, 1)?.neg()?.sub_scalar(1, 1)?,
    )?)
}
