//! Baberu OCR image and character processing.
//!
//! Canonical image and decoding behavior:
//! https://huggingface.co/genshiai-daichi/baberu-ocr/blob/a0e3d4b1f98a8c5d0aba66ded831fe7b52bbf2fa/inference.py

use std::{collections::HashSet, path::Path};

use anyhow::{Result, bail, ensure};
use fast_image_resize::{FilterType, ResizeAlg};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use image::DynamicImage;
use koharu_torch::{Device, Kind, Tensor};
use serde::Deserialize;

/// Pillow's bicubic filter, the same Catmull-Rom convolution.
const RESIZE: ResizeAlg = ResizeAlg::Convolution(FilterType::CatmullRom);

#[derive(Debug, Deserialize)]
pub(super) struct BitImageProcessorConfig {
    do_convert_rgb: bool,
    do_normalize: bool,
    do_rescale: bool,
    image_mean: [f32; 3],
    image_std: [f32; 3],
    resample: u8,
    rescale_factor: f32,
}

#[derive(Debug)]
pub(super) struct BaberuImageProcessor {
    size: u32,
    do_normalize: bool,
    do_rescale: bool,
    rescale_factor: f64,
    mean: Tensor,
    std: Tensor,
    resample: u8,
    device: Device,
}

impl BaberuImageProcessor {
    pub(super) fn new(config: &BitImageProcessorConfig, size: i64, device: Device) -> Result<Self> {
        ensure!(config.do_convert_rgb, "Baberu OCR requires RGB conversion");
        ensure!(
            size > 0 && size <= u32::MAX.into(),
            "invalid image size {size}"
        );
        ensure!(
            config.image_std.iter().all(|value| *value != 0.0),
            "Baberu OCR image standard deviation contains zero"
        );
        let mean = Tensor::f_from_slice(&config.image_mean)?
            .f_view([1, 3, 1, 1])?
            .f_to_device(device)?;
        let std = Tensor::f_from_slice(&config.image_std)?
            .f_view([1, 3, 1, 1])?
            .f_to_device(device)?;
        Ok(Self {
            size: size as u32,
            do_normalize: config.do_normalize,
            do_rescale: config.do_rescale,
            rescale_factor: config.rescale_factor.into(),
            mean,
            std,
            resample: config.resample,
            device,
        })
    }

    pub(super) fn preprocess(&self, image: &DynamicImage) -> Result<Tensor> {
        if image.width() == 0 || image.height() == 0 {
            bail!("cannot recognize an empty image");
        }
        if self.resample != 3 {
            bail!("unsupported DINOv2 PIL resampling filter {}", self.resample);
        }

        // `build_ocr_image_processor` disables center cropping and resizes the
        // complete bubble to a fixed square with Pillow's bicubic filter.
        // `fast_image_resize` uses the same Catmull-Rom convolution but can
        // differ at coefficient rounding; on the 280x53 fixture its byte sum is
        // 355 below Pillow across 150,528 channels, with identical decoded IDs.
        let image = image.to_rgb8();
        let image = if image.width() == self.size && image.height() == self.size {
            image
        } else {
            crate::image::resize(&image, self.size, self.size, RESIZE)?
        };
        let mut pixel_values = Tensor::f_from_slice(image.as_raw())?
            .f_view([1, i64::from(self.size), i64::from(self.size), 3])?
            .f_permute([0, 3, 1, 2])?
            .f_to_device(self.device)?
            .f_to_kind(Kind::Float)?;
        if self.do_rescale {
            pixel_values = pixel_values.f_mul_scalar(self.rescale_factor)?;
        }
        if self.do_normalize {
            pixel_values = (pixel_values.f_sub(&self.mean)?).f_div(&self.std)?;
        }
        Ok(pixel_values)
    }
}

#[derive(Debug)]
pub(super) struct Tokenizer {
    vocabulary: Vec<String>,
    content_ids: HashSet<i64>,
}

impl Tokenizer {
    pub(super) fn from_file(path: &Path) -> Result<Self> {
        let charset: Vec<String> = serde_json::from_slice(&fs_err::read(path)?)?;
        let mut vocabulary = vec![
            "<pad>".to_owned(),
            "<bos>".to_owned(),
            "<eos>".to_owned(),
            "<unk>".to_owned(),
        ];
        let mut seen = vocabulary.iter().cloned().collect::<HashSet<_>>();
        for character in charset {
            if seen.insert(character.clone()) {
                vocabulary.push(character);
            }
        }
        if vocabulary.len() == 4 {
            bail!("Baberu OCR vocabulary is empty");
        }

        let categories = CodePointMapData::<GeneralCategory>::new();
        let content_ids = vocabulary
            .iter()
            .enumerate()
            .filter_map(|(token_id, token)| {
                let mut characters = token.chars();
                let character = characters.next()?;
                (characters.next().is_none()
                    && !matches!(character, 'ー' | 'ｰ' | '〜' | '~')
                    && (GeneralCategoryGroup::Letter.contains(categories.get(character))
                        || GeneralCategoryGroup::Number.contains(categories.get(character))))
                .then_some(token_id as i64)
            })
            .collect();
        Ok(Self {
            vocabulary,
            content_ids,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.vocabulary.len()
    }

    pub(super) fn is_content(&self, token_id: i64) -> bool {
        self.content_ids.contains(&token_id)
    }

    pub(super) fn decode(&self, token_ids: &[i64]) -> Result<String> {
        let mut text = String::new();
        for &token_id in token_ids {
            let token = self
                .vocabulary
                .get(token_id as usize)
                .ok_or_else(|| anyhow::anyhow!("invalid Baberu OCR token id {token_id}"))?;
            if token_id >= 4 {
                text.push_str(token);
            }
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, path::PathBuf};

    use super::{RESIZE, Tokenizer};

    #[test]
    fn bicubic_resize_stays_aligned_with_pillow() {
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/ocr/title.png");
        let input = image::open(input).unwrap().to_rgb8();
        let resized = crate::image::resize(&input, 224, 224, RESIZE).unwrap();
        let pixels = resized.as_raw();

        assert_eq!(resized.dimensions(), (224, 224));
        assert_eq!(
            [0, 1, 2, 1000, 50_000, 100_000, 150_000].map(|index| pixels[index]),
            [255, 255, 255, 255, 255, 114, 254]
        );
        let sum = pixels.iter().map(|value| u64::from(*value)).sum::<u64>();
        assert!((sum as i64 - 32_934_996).abs() <= 355);
    }

    #[test]
    fn tokenizer_matches_upstream_special_tokens_and_content_categories() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(r#"["A","7","。","ー","猫","<pad>"]"#.as_bytes())
            .unwrap();
        let tokenizer = Tokenizer::from_file(file.path()).unwrap();

        assert_eq!(tokenizer.len(), 9);
        assert!(tokenizer.is_content(4));
        assert!(tokenizer.is_content(5));
        assert!(!tokenizer.is_content(6));
        assert!(!tokenizer.is_content(7));
        assert!(tokenizer.is_content(8));
        assert_eq!(tokenizer.decode(&[1, 4, 6, 8, 2]).unwrap(), "A。猫");
    }
}
