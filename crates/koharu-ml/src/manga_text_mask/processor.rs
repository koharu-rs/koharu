//! Preprocessing and output handling from the original Gradio inference script.
//!
//! https://huggingface.co/a-b-c-x-y-z/Manga-Text-Segmentation-2025/blob/2dde9eeb03e81692c1562059451c2bf30e1a13da/inference.py

use std::collections::VecDeque;

use anyhow::{Context, Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GrayImage, Luma, RgbImage};
use imageproc::{
    distance_transform::Norm,
    morphology::{Mask, dilate, grayscale_dilate, grayscale_erode},
};
use koharu_torch::{Device, Kind, Tensor};

use super::model::Model;

#[derive(Debug)]
pub(super) struct Processor {
    device: Device,
    mean: Tensor,
    std: Tensor,
}

impl Processor {
    pub(super) fn new(device: Device) -> Self {
        let mean = Tensor::from_slice(&[0.485f32, 0.456, 0.406])
            .view([1, 3, 1, 1])
            .to_device(device);
        let std = Tensor::from_slice(&[0.229f32, 0.224, 0.225])
            .view([1, 3, 1, 1])
            .to_device(device);
        Self { device, mean, std }
    }

    pub(super) fn call(
        &self,
        model: &Model,
        image: &DynamicImage,
        horizontal_flip: bool,
        vertical_flip: bool,
        max_side: Option<u32>,
    ) -> Result<MangaTextMask> {
        let (input, geometry) = self.preprocess(image, max_side)?;
        let infer = |input: &Tensor| model.forward(input).sigmoid();

        let mut probabilities = infer(&input);
        let mut steps = 1;
        if horizontal_flip {
            probabilities += infer(&input.flip([3])).flip([3]);
            steps += 1;
        }
        if vertical_flip {
            probabilities += infer(&input.flip([2])).flip([2]);
            steps += 1;
        }
        probabilities /= steps as f64;

        let mut probabilities = probabilities
            .narrow(2, 0, i64::from(geometry.input_height))
            .narrow(3, 0, i64::from(geometry.input_width));
        if geometry.input_width != geometry.original_width
            || geometry.input_height != geometry.original_height
        {
            // Keep the probability resize on the target device and transfer only
            // the final caller-facing map. The default path never enters here.
            probabilities = probabilities.upsample_bilinear2d(
                [
                    i64::from(geometry.original_height),
                    i64::from(geometry.original_width),
                ],
                false,
                None::<f64>,
                None::<f64>,
            );
        }

        let probabilities = probabilities
            .to_kind(Kind::Float)
            .contiguous()
            .to_device(Device::Cpu)
            .view([-1]);
        let probabilities = Vec::<f32>::try_from(&probabilities)?;
        Ok(MangaTextMask {
            width: geometry.original_width,
            height: geometry.original_height,
            probabilities,
        })
    }

    fn preprocess(
        &self,
        image: &DynamicImage,
        max_side: Option<u32>,
    ) -> Result<(Tensor, InputGeometry)> {
        let original_width = image.width();
        let original_height = image.height();
        ensure!(
            original_width > 0 && original_height > 0,
            "manga text segmentation input must be non-empty"
        );

        let mut image = image.to_rgb8();
        if let Some(max_side) = max_side {
            ensure!(
                max_side > 0,
                "manga text segmentation max side must be positive"
            );
            if original_width.max(original_height) > max_side {
                let (width, height) = resize_dimensions(original_width, original_height, max_side);
                image = resize_rgb(&image, width, height)?;
            }
        }
        let input_width = image.width();
        let input_height = image.height();

        let input = Tensor::from_slice(image.as_raw())
            .view([1, i64::from(input_height), i64::from(input_width), 3])
            .permute([0, 3, 1, 2])
            .to_device(self.device)
            .to_kind(Kind::Float)
            / 255.0;
        let input = (input - &self.mean) / &self.std;

        // F.pad(tensor, (0, pad_w, 0, pad_h), mode="constant", value=0)
        // pads normalized zeros only on the bottom and right edges.
        let pad_height = (32 - input_height % 32) % 32;
        let pad_width = (32 - input_width % 32) % 32;
        let input = if pad_height == 0 && pad_width == 0 {
            input
        } else {
            input.constant_pad_nd([0, i64::from(pad_width), 0, i64::from(pad_height)])
        };

        Ok((
            input,
            InputGeometry {
                original_width,
                original_height,
                input_width,
                input_height,
            },
        ))
    }
}

#[derive(Debug, Clone, Copy)]
struct InputGeometry {
    original_width: u32,
    original_height: u32,
    input_width: u32,
    input_height: u32,
}

#[derive(Debug, Clone)]
pub struct MangaTextMask {
    width: u32,
    height: u32,
    probabilities: Vec<f32>,
}

impl MangaTextMask {
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn probabilities(&self) -> &[f32] {
        &self.probabilities
    }

    #[must_use]
    pub fn into_probabilities(self) -> Vec<f32> {
        self.probabilities
    }

    pub fn binary_mask(&self, threshold: f32) -> Result<GrayImage> {
        ensure!(
            (0.0..=1.0).contains(&threshold),
            "manga text segmentation threshold must be between 0 and 1"
        );
        GrayImage::from_raw(
            self.width,
            self.height,
            self.probabilities
                .iter()
                // Upstream deliberately uses `>` rather than `>=`.
                .map(|&probability| if probability > threshold { 255 } else { 0 })
                .collect(),
        )
        .context("failed to construct manga text segmentation mask")
    }

    pub fn process(&self, options: &MangaTextMaskCleaningOptions) -> Result<GrayImage> {
        ensure!(
            options.padding_iterations <= 10,
            "manga text cleaning padding iterations must be at most 10"
        );
        ensure!(
            options.close_gaps_kernel <= 10,
            "manga text cleaning gap-closing kernel must be at most 10"
        );

        let mut mask = self.binary_mask(options.threshold)?;
        if options.close_gaps_kernel > 0 {
            let kernel_size = options.close_gaps_kernel | 1;
            let kernel = opencv_ellipse(kernel_size as u8);
            mask = grayscale_erode(&grayscale_dilate(&mask, &kernel), &kernel);
        }
        if options.fill_holes {
            fill_holes(&mut mask);
        }

        if options.padding_iterations > 0 {
            // Repeating OpenCV dilation with a 3x3 all-ones kernel is one
            // Chebyshev-radius dilation with the iteration count as radius.
            mask = dilate(&mask, Norm::LInf, options.padding_iterations as u8);
        }

        Ok(mask)
    }
}

#[derive(Debug, Clone)]
pub struct MangaTextMaskCleaningOptions {
    pub threshold: f32,
    pub padding_iterations: u32,
    pub fill_holes: bool,
    pub close_gaps_kernel: u32,
}

impl Default for MangaTextMaskCleaningOptions {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            padding_iterations: 2,
            fill_holes: true,
            close_gaps_kernel: 2,
        }
    }
}

fn resize_dimensions(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let scale = max_side as f64 / width.max(height) as f64;
    (
        (width as f64 * scale).round_ties_even().max(1.0) as u32,
        (height as f64 * scale).round_ties_even().max(1.0) as u32,
    )
}

fn resize_rgb(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    let mut resized = RgbImage::new(width, height);
    Resizer::new().resize(
        image,
        &mut resized,
        &ResizeOptions::new()
            .resize_alg(ResizeAlg::Interpolation(FilterType::Bilinear))
            .use_alpha(false),
    )?;
    Ok(resized)
}

fn opencv_ellipse(kernel_size: u8) -> Mask {
    let mut image = GrayImage::new(u32::from(kernel_size), u32::from(kernel_size));
    let radius = i32::from(kernel_size / 2);
    let inverse_radius_squared = if radius == 0 {
        0.0
    } else {
        1.0 / f64::from(radius * radius)
    };
    for y in 0..i32::from(kernel_size) {
        let dy = y - radius;
        if dy.abs() > radius {
            continue;
        }
        let dx = (f64::from(radius)
            * (f64::from(radius * radius - dy * dy) * inverse_radius_squared).sqrt())
        .round_ties_even() as i32;
        for x in radius - dx..=radius + dx {
            image.put_pixel(x as u32, y as u32, Luma([255]));
        }
    }
    Mask::from_image(&image, kernel_size / 2, kernel_size / 2)
}

fn fill_holes(mask: &mut GrayImage) {
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut outside = vec![false; width * height];
    let mut queue = VecDeque::new();

    let push_background =
        |x: usize, y: usize, outside: &mut [bool], queue: &mut VecDeque<(usize, usize)>| {
            let index = y * width + x;
            if mask.as_raw()[index] == 0 && !outside[index] {
                outside[index] = true;
                queue.push_back((x, y));
            }
        };
    for x in 0..width {
        push_background(x, 0, &mut outside, &mut queue);
        if height > 1 {
            push_background(x, height - 1, &mut outside, &mut queue);
        }
    }
    for y in 0..height {
        push_background(0, y, &mut outside, &mut queue);
        if width > 1 {
            push_background(width - 1, y, &mut outside, &mut queue);
        }
    }

    while let Some((x, y)) = queue.pop_front() {
        let min_x = x.saturating_sub(1);
        let max_x = (x + 1).min(width - 1);
        let min_y = y.saturating_sub(1);
        let max_y = (y + 1).min(height - 1);
        for neighbor_y in min_y..=max_y {
            for neighbor_x in min_x..=max_x {
                push_background(neighbor_x, neighbor_y, &mut outside, &mut queue);
            }
        }
    }

    for (pixel, is_outside) in mask.iter_mut().zip(outside) {
        if *pixel == 0 && !is_outside {
            *pixel = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{GrayImage, Luma};

    use super::{
        MangaTextMask, MangaTextMaskCleaningOptions, fill_holes, opencv_ellipse, resize_dimensions,
    };

    #[test]
    fn resize_preserves_aspect_ratio() {
        assert_eq!(resize_dimensions(1000, 500, 640), (640, 320));
        assert_eq!(resize_dimensions(500, 1000, 640), (320, 640));
    }

    #[test]
    fn threshold_is_strictly_greater_than() -> anyhow::Result<()> {
        let output = MangaTextMask {
            width: 3,
            height: 1,
            probabilities: vec![0.49, 0.5, 0.51],
        };
        assert_eq!(output.binary_mask(0.5)?.as_raw(), &[0, 0, 255]);
        Ok(())
    }

    #[test]
    fn hole_filling_keeps_exterior_background() {
        let mut mask = GrayImage::from_pixel(5, 5, Luma([255]));
        mask.put_pixel(2, 2, Luma([0]));
        mask.put_pixel(0, 0, Luma([0]));
        fill_holes(&mut mask);
        assert_eq!(mask.get_pixel(2, 2), &Luma([255]));
        assert_eq!(mask.get_pixel(0, 0), &Luma([0]));
    }

    #[test]
    fn ellipse_matches_opencv_five_by_five_shape() {
        let source = GrayImage::from_fn(5, 5, |x, y| {
            if x == 2 && y == 2 {
                Luma([255])
            } else {
                Luma([0])
            }
        });
        let dilated = imageproc::morphology::grayscale_dilate(&source, &opencv_ellipse(5));
        let rows = dilated
            .as_raw()
            .chunks(5)
            .map(|row| row.iter().filter(|&&value| value != 0).count())
            .collect::<Vec<_>>();
        assert_eq!(rows, [1, 5, 5, 5, 1]);
    }

    #[test]
    fn process_returns_the_padded_mask() -> anyhow::Result<()> {
        let output = MangaTextMask {
            width: 3,
            height: 3,
            probabilities: vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        };
        let mask = output.process(&MangaTextMaskCleaningOptions::default())?;
        assert!(mask.pixels().all(|pixel| pixel.0 == [255]));
        Ok(())
    }
}
