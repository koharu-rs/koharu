//! COO TRBA preprocessing and SAR decoding.
//!
//! Upstream implementation:
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/dataset.py#L262-L340
//! https://github.com/ku21fan/COO-Comic-Onomatopoeia/blob/d8028f015b8ce99a4dd798427342f97087529357/TRBA/test.py#L245-L388

use anyhow::{Context, Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use tch::{Device, Kind, Tensor};

use super::config::Config;

const SPECIAL_TOKENS: [&str; 5] = ["[PAD]", "[UNK]", "[SOS]", "[EOS]", " "];
const EOS_INDEX: i64 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recognition {
    pub text: String,
    pub confidence: f32,
    pub rotation_degrees: u16,
}

#[derive(Debug)]
pub(super) struct Processor {
    tokens: Vec<String>,
    image_height: u32,
    image_width: u32,
}

impl Processor {
    pub(super) fn new(character_set: &str, config: &Config) -> Result<Self> {
        let mut tokens = SPECIAL_TOKENS
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        tokens.extend(
            character_set
                .trim_start_matches('\u{feff}')
                .trim()
                .chars()
                .map(|c| c.to_string()),
        );
        ensure!(
            tokens.len() == config.num_classes as usize,
            "COO character set produces {} tokens but the model has {} outputs",
            tokens.len(),
            config.num_classes
        );
        Ok(Self {
            tokens,
            image_height: config.image_height as u32,
            image_width: config.image_width as u32,
        })
    }

    pub(super) fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        let rgb = image.to_rgb8();
        let original = resize_normalize(&rgb, self.image_width, self.image_height)?;
        let (rotated_90, rotated_270) = if image.height() > image.width() {
            // PIL's ROTATE_90 is counter-clockwise and ROTATE_270 is clockwise.
            let counter_clockwise = image::imageops::rotate270(&rgb);
            let clockwise = image::imageops::rotate90(&rgb);
            (
                resize_normalize(&counter_clockwise, self.image_width, self.image_height)?,
                resize_normalize(&clockwise, self.image_width, self.image_height)?,
            )
        } else {
            (original.f_alias()?, original.f_alias()?)
        };
        Ok(Tensor::f_stack(&[original, rotated_90, rotated_270], 0)?.f_to_device(device)?)
    }

    pub(super) fn postprocess(&self, logits: &Tensor) -> Result<Recognition> {
        ensure!(
            logits.size().len() == 3 && logits.size()[0] == 3,
            "COO SAR decoder expects logits for three rotations, got {:?}",
            logits.size()
        );
        let probabilities = logits.f_softmax(-1, Kind::Float)?;
        let (maximum_probabilities, indices) = probabilities.f_max_dim(-1, false)?;
        let indices: Vec<Vec<i64>> = Vec::<Vec<i64>>::try_from(&indices.f_to_device(Device::Cpu)?)
            .context("failed to copy COO token indices to CPU")?;
        let maximum_probabilities: Vec<Vec<f32>> =
            Vec::<Vec<f32>>::try_from(&maximum_probabilities.f_to_device(Device::Cpu)?)
                .context("failed to copy COO token probabilities to CPU")?;

        let mut candidates = indices
            .iter()
            .zip(&maximum_probabilities)
            .enumerate()
            .map(|(rotation, (indices, probabilities))| {
                let end = indices
                    .iter()
                    .position(|&index| index == EOS_INDEX)
                    .unwrap_or(indices.len());
                // Upstream catches the empty cumprod indexing error and assigns zero.
                let confidence = if end == 0 {
                    0.0
                } else {
                    probabilities[..end].iter().copied().product::<f32>()
                };
                let text = indices[..end]
                    .iter()
                    .map(|&index| {
                        self.tokens
                            .get(index as usize)
                            .cloned()
                            .with_context(|| format!("COO decoder emitted token index {index}"))
                    })
                    .collect::<Result<String>>()?;
                Ok(Recognition {
                    text,
                    confidence,
                    rotation_degrees: [0, 90, 270][rotation],
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // Python's max/list.index keeps the first item when confidences tie.
        let mut best = 0;
        for index in 1..candidates.len() {
            if candidates[index].confidence > candidates[best].confidence {
                best = index;
            }
        }
        Ok(candidates.swap_remove(best))
    }
}

fn resize_normalize(image: &image::RgbImage, width: u32, height: u32) -> Result<Tensor> {
    let mut resized = image::RgbImage::new(width, height);
    Resizer::new()
        .resize(
            image,
            &mut resized,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::CatmullRom))
                .use_alpha(false),
        )
        .context("failed to resize COO recognition input")?;
    let values = resized
        .pixels()
        .flat_map(|pixel| pixel.0)
        .map(|value| value as f32 / 127.5 - 1.0)
        .collect::<Vec<_>>();
    Ok(Tensor::f_from_slice(&values)?
        .f_view([height as i64, width as i64, 3])?
        .f_permute([2, 0, 1])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bicubic_resize_stays_aligned_with_pillow() -> Result<()> {
        let input = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("benches/fixtures/ocr/title.png");
        let input = image::open(input)?.to_rgb8();
        let mut resized = image::RgbImage::new(100, 100);
        Resizer::new().resize(
            &input,
            &mut resized,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::CatmullRom))
                .use_alpha(false),
        )?;
        let pixels = resized.as_raw();
        assert_eq!(
            [0, 1, 2, 1_000, 5_000, 10_000, 20_000, 29_999].map(|index| pixels[index]),
            [255, 255, 255, 255, 255, 213, 254, 255]
        );
        let sum = pixels.iter().map(|value| u64::from(*value)).sum::<u64>();
        assert!((sum as i64 - 6_561_274).abs() <= 95);
        Ok(())
    }

    #[test]
    fn character_set_matches_checkpoint_outputs() -> Result<()> {
        let processor = Processor::new(include_str!("character_set.txt"), &Config::default())?;
        assert_eq!(processor.tokens.len(), 187);
        assert_eq!(processor.tokens[0], "[PAD]");
        assert_eq!(processor.tokens[2], "[SOS]");
        assert_eq!(processor.tokens[3], "[EOS]");
        assert_eq!(processor.tokens[5], "!");
        Ok(())
    }
}
