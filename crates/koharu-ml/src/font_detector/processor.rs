//! YuzuMarker input and output processing.
//!
//! Original preprocessing and regression targets:
//! - https://github.com/JeffersonQin/YuzuMarker.FontDetection/blob/0a94e165fe2b08d2800b723290eabd120b2d3d58/demo.py
//! - https://github.com/JeffersonQin/YuzuMarker.FontDetection/blob/0a94e165fe2b08d2800b723290eabd120b2d3d58/detector/data.py

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_torch::{Device, Kind, Tensor};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub(super) const FONT_COUNT: i64 = 6_150;
pub(super) const REGRESSION_DIM: i64 = 10;
pub(super) const OUTPUT_DIM: i64 = FONT_COUNT + 2 + REGRESSION_DIM;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TextDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopFont {
    pub index: usize,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedFontPrediction {
    pub index: usize,
    pub name: String,
    pub language: Option<String>,
    pub probability: f32,
    pub serif: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FontPrediction {
    pub top_fonts: Vec<TopFont>,
    pub named_fonts: Vec<NamedFontPrediction>,
    pub direction: TextDirection,
    pub text_color: [u8; 3],
    pub stroke_color: [u8; 3],
    pub font_size_px: f32,
    pub stroke_width_px: f32,
    pub line_height: f32,
    pub angle_deg: f32,
}

impl Default for FontPrediction {
    fn default() -> Self {
        Self {
            top_fonts: Vec::new(),
            named_fonts: Vec::new(),
            direction: TextDirection::Horizontal,
            text_color: [0; 3],
            stroke_color: [0; 3],
            font_size_px: 0.0,
            stroke_width_px: 0.0,
            line_height: 1.0,
            angle_deg: 0.0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FontLabel {
    path: String,
    language: Option<String>,
    serif: bool,
}

#[derive(Debug)]
pub(super) struct Processor {
    labels: Vec<FontLabel>,
}

impl Processor {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let labels = serde_json::from_str::<Vec<FontLabel>>(
            &fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", path.display()))?;
        ensure!(
            labels.len() == FONT_COUNT as usize,
            "YuzuMarker label count {} does not match model font count {FONT_COUNT}",
            labels.len()
        );
        Ok(Self { labels })
    }

    pub fn preprocess(
        &self,
        images: &[DynamicImage],
        device: Device,
    ) -> Result<(Tensor, Vec<u32>)> {
        let images = images
            .par_iter()
            .map(|image| {
                let image = image.to_rgb8();
                (image.width(), image.height(), image.into_raw())
            })
            .collect::<Vec<_>>();

        // Torchvision applies antialiased bilinear resize to the PIL input,
        // followed by `ToTensor`. LibTorch's uint8 antialiased operator keeps
        // the same interpolation and quantization order. CUDA does not support
        // that operator for bytes, so resize on CPU, batch the packed results,
        // and perform one transfer before the on-device float conversion.
        let pixel_values = (Tensor::cat(
            &images
                .iter()
                .map(|&(width, height, ref pixels)| {
                    Tensor::from_slice(pixels)?
                        .view([height as i64, width as i64, 3])?
                        .permute([2, 0, 1])?
                        .unsqueeze(0)?
                        .internal_upsample_bilinear2d_aa(
                            [512, 512],
                            false,
                            None::<f64>,
                            None::<f64>,
                        )
                })
                .collect::<koharu_torch::Result<Vec<_>>>()?,
            0,
        )?
        .to_device(device)?
        .to_kind(Kind::Float)?
            / 255.0)?;
        let image_widths = images.iter().map(|&(width, _, _)| width).collect();
        Ok((pixel_values, image_widths))
    }

    pub fn postprocess(
        &self,
        output: &Tensor,
        image_widths: &[u32],
        top_k: usize,
    ) -> Result<Vec<FontPrediction>> {
        let batch_size = output.size()[0] as usize;
        ensure!(
            output.size() == [batch_size as i64, OUTPUT_DIM],
            "unexpected YuzuMarker output shape {:?}",
            output.size()
        );
        ensure!(
            image_widths.len() == batch_size,
            "image width count {} does not match model batch size {batch_size}",
            image_widths.len()
        );

        let top_k = top_k.min(FONT_COUNT as usize) as i64;
        let probabilities = output
            .narrow(-1, 0, FONT_COUNT)?
            .softmax(-1, None::<Kind>)?;
        let (scores, indices) = probabilities.topk(top_k, -1, true, true)?;
        let directions = output
            .narrow(-1, FONT_COUNT, 2)?
            .argmax(-1, false)?
            .unsqueeze(-1)?;
        let regression = output.narrow(-1, FONT_COUNT + 2, REGRESSION_DIM)?;

        // Copy only top-k scores and the compact style output. Grouping the
        // floating-point and integer results keeps GPU synchronization to two
        // host transfers, independent of the batch size.
        let floats = Tensor::cat(&[scores, regression], -1)?.to_device(Device::Cpu)?;
        let integers = Tensor::cat(&[indices, directions], -1)?.to_device(Device::Cpu)?;
        let floats = Vec::<Vec<f32>>::try_from(&floats)?;
        let integers = Vec::<Vec<i64>>::try_from(&integers)?;

        Ok(floats
            .into_iter()
            .zip(integers)
            .zip(image_widths)
            .map(|((floats, integers), &image_width)| {
                let top_fonts = (0..top_k as usize)
                    .map(|rank| TopFont {
                        index: integers[rank] as usize,
                        score: floats[rank],
                    })
                    .collect::<Vec<_>>();
                let named_fonts = top_fonts
                    .iter()
                    .map(|font| {
                        let label = &self.labels[font.index];
                        NamedFontPrediction {
                            index: font.index,
                            name: label.path.clone(),
                            language: label.language.clone(),
                            probability: font.score,
                            serif: label.serif,
                        }
                    })
                    .collect();
                let regression = &floats[top_k as usize..];
                let font_size_px = regression[3] * image_width as f32;
                let line_spacing_px = regression[8] * image_width as f32;

                FontPrediction {
                    top_fonts,
                    named_fonts,
                    direction: if integers[top_k as usize] == 0 {
                        TextDirection::Horizontal
                    } else {
                        TextDirection::Vertical
                    },
                    text_color: [
                        normalized_channel(regression[0]),
                        normalized_channel(regression[1]),
                        normalized_channel(regression[2]),
                    ],
                    font_size_px,
                    stroke_width_px: regression[4] * image_width as f32,
                    stroke_color: [
                        normalized_channel(regression[5]),
                        normalized_channel(regression[6]),
                        normalized_channel(regression[7]),
                    ],
                    // Upstream stores extra inter-line spacing, while Koharu's
                    // renderer consumes the conventional baseline multiplier.
                    line_height: if font_size_px > 0.0 {
                        1.0 + line_spacing_px / font_size_px
                    } else {
                        1.2
                    },
                    angle_deg: (regression[9] - 0.5) * 180.0,
                }
            })
            .collect())
    }
}

fn normalized_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_runtime::Runtime;

    async fn preload_libtorch() -> anyhow::Result<()> {
        Runtime::start().await?;
        Ok(())
    }

    fn processor() -> Processor {
        Processor {
            labels: (0..FONT_COUNT)
                .map(|index| FontLabel {
                    path: format!("font-{index}"),
                    language: Some("CJK".into()),
                    serif: index % 2 == 0,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn maps_upstream_regression_layout() -> anyhow::Result<()> {
        preload_libtorch().await?;
        let mut values = vec![0.0; OUTPUT_DIM as usize];
        values[3] = 2.0;
        values[7] = 1.0;
        values[FONT_COUNT as usize + 1] = 1.0;
        values[FONT_COUNT as usize + 2..]
            .copy_from_slice(&[0.25, 0.5, 0.75, 0.2, 0.1, 0.1, 0.2, 0.3, 0.4, 0.75]);
        let output = Tensor::from_slice(&values)?.view([1, OUTPUT_DIM])?;

        let predictions = processor().postprocess(&output, &[200], 2).unwrap();
        let prediction = &predictions[0];
        assert_eq!(
            prediction
                .top_fonts
                .iter()
                .map(|font| font.index)
                .collect::<Vec<_>>(),
            [3, 7]
        );
        assert_eq!(prediction.named_fonts[0].name, "font-3");
        assert_eq!(prediction.direction, TextDirection::Vertical);
        assert_eq!(prediction.text_color, [64, 128, 191]);
        assert_eq!(prediction.stroke_color, [26, 51, 77]);
        assert!((prediction.font_size_px - 40.0).abs() < 1e-5);
        assert!((prediction.stroke_width_px - 20.0).abs() < 1e-5);
        assert!((prediction.line_height - 3.0).abs() < 1e-5);
        assert!((prediction.angle_deg - 45.0).abs() < 1e-5);
        Ok(())
    }

    #[tokio::test]
    async fn preprocesses_a_batch_in_upstream_shape_and_range() -> anyhow::Result<()> {
        preload_libtorch().await?;
        let images = [
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 3, image::Rgb([255, 0, 0]))),
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 5, image::Rgb([0, 255, 0]))),
        ];
        let (pixels, widths) = processor().preprocess(&images, Device::Cpu).unwrap();

        assert_eq!(pixels.size(), [2, 3, 512, 512]);
        assert_eq!(pixels.kind(), Kind::Float);
        assert_eq!(widths, [2, 4]);
        assert_eq!(
            f32::try_from(pixels.get(0)?.get(0)?.get(0)?.get(0)?).unwrap(),
            1.0
        );
        assert_eq!(
            f32::try_from(pixels.get(1)?.get(1)?.get(0)?.get(0)?).unwrap(),
            1.0
        );
        Ok(())
    }
}
