//! Manga OCR image, token, and text processing.
//!
//! Image preprocessing follows Transformers 4.15's `ViTFeatureExtractor` and the
//! grayscale conversion performed by manga-ocr:
//! https://github.com/huggingface/transformers/blob/05fa1a7ac17bb7aa07b9e0c1e138ecb31a28bbfe/src/transformers/models/vit/feature_extraction_vit.py
//! https://github.com/kha-white/manga-ocr/blob/847e0939fcb391ad63584dcf50ba6c6533ec9ee8/manga_ocr/ocr.py

use std::{collections::HashSet, fs, path::Path};

use anyhow::{Result, bail};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, RgbImage};
use serde::Deserialize;
use koharu_torch::{Device, Kind, Tensor};

#[derive(Debug, Clone, Deserialize)]
pub struct ViTImageProcessor {
    do_normalize: bool,
    do_resize: bool,
    image_mean: [f32; 3],
    image_std: [f32; 3],
    resample: u8,
    size: u32,
}

impl ViTImageProcessor {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub(crate) fn preprocess(&self, image: &DynamicImage, device: Device) -> Result<Tensor> {
        if image.width() == 0 || image.height() == 0 {
            bail!("cannot recognize an empty image");
        }
        if self.resample != 2 {
            bail!(
                "unsupported Manga OCR PIL resampling filter {}",
                self.resample
            );
        }

        // `MangaOcr.__call__` uses Pillow's ITU-R 601-2 L conversion, then RGB.
        let image = pillow_grayscale(image);
        let image = if self.do_resize && (image.width() != self.size || image.height() != self.size)
        {
            resize_bilinear(&image, self.size, self.size)?
        } else {
            image
        };
        let height = image.height() as i64;
        let width = image.width() as i64;
        let mut pixel_values = (Tensor::f_from_slice(image.as_raw())?
            .f_view([1, height, width, 3])?
            .f_permute([0, 3, 1, 2])?
            .f_to_device(device)?
            .f_to_kind(Kind::Float)?)
        .f_div_scalar(255.0)?;
        if self.do_normalize {
            let mean = Tensor::f_from_slice(&self.image_mean)?
                .f_view([1, 3, 1, 1])?
                .f_to_device(device)?;
            let std = Tensor::f_from_slice(&self.image_std)?
                .f_view([1, 3, 1, 1])?
                .f_to_device(device)?;
            pixel_values = pixel_values.f_sub(&mean)?.f_div(&std)?;
        }
        Ok(pixel_values)
    }
}

fn pillow_grayscale(image: &DynamicImage) -> RgbImage {
    let mut image = image.to_rgb8();
    for pixel in image.pixels_mut() {
        let luma = ((19_595 * u32::from(pixel[0])
            + 38_470 * u32::from(pixel[1])
            + 7_471 * u32::from(pixel[2])
            + 32_768)
            >> 16) as u8;
        *pixel = image::Rgb([luma; 3]);
    }
    image
}

fn resize_bilinear(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    let mut resized = RgbImage::new(width, height);
    Resizer::new().resize(
        image,
        &mut resized,
        &ResizeOptions::new()
            // Convolution widens the bilinear kernel like Pillow while downscaling.
            // fast_image_resize can still differ from Pillow at coefficient rounding.
            .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
            .use_alpha(false),
    )?;
    Ok(resized)
}

#[derive(Debug)]
pub(crate) struct Tokenizer {
    vocabulary: Vec<String>,
    special_tokens: HashSet<&'static str>,
}

impl Tokenizer {
    pub(crate) fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let vocabulary = fs::read_to_string(path)?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if vocabulary.is_empty() {
            bail!("Manga OCR vocabulary is empty");
        }
        Ok(Self {
            vocabulary,
            special_tokens: HashSet::from(["[UNK]", "[SEP]", "[PAD]", "[CLS]", "[MASK]"]),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.vocabulary.len()
    }

    pub(crate) fn decode(&self, token_ids: &[i64]) -> Result<String> {
        let mut text = String::new();
        for &token_id in token_ids {
            let token = self
                .vocabulary
                .get(token_id as usize)
                .ok_or_else(|| anyhow::anyhow!("invalid Manga OCR token id {token_id}"))?;
            if !self.special_tokens.contains(token.as_str()) {
                // BertJapaneseTokenizer's character subword tokenizer joins without spaces.
                text.push_str(token.strip_prefix("##").unwrap_or(token));
            }
        }
        Ok(post_process(&text))
    }
}

fn post_process(text: &str) -> String {
    let text = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let text = text.replace('\u{2026}', "...");
    let mut dots = String::with_capacity(text.len());
    let mut dot_run = Vec::new();
    for character in text.chars() {
        if character == '.' || character == '\u{30fb}' {
            dot_run.push(character);
            continue;
        }
        flush_dot_run(&mut dots, &mut dot_run);
        dots.push(character);
    }
    flush_dot_run(&mut dots, &mut dot_run);
    halfwidth_to_fullwidth(&dots)
}

fn flush_dot_run(output: &mut String, run: &mut Vec<char>) {
    if run.len() >= 2 {
        output.extend(std::iter::repeat_n('.', run.len()));
    } else {
        output.extend(run.iter());
    }
    run.clear();
}

fn halfwidth_to_fullwidth(text: &str) -> String {
    // jaconv.h2z(kana=True, ascii=True, digit=True). jaconv first replaces
    // supported two-codepoint voiced kana, then applies its one-to-one table.
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '!'..='~' => {
                output.push(char::from_u32(character as u32 + 0xfee0).unwrap_or(character))
            }
            ' ' => output.push('\u{3000}'),
            '\u{ff9e}' | '\u{ff9f}' => {
                let previous = output.pop();
                if let Some(composed) = previous.and_then(|base| compose_katakana(base, character))
                {
                    output.push(composed);
                } else {
                    output.extend(previous);
                    output.push(character);
                }
            }
            '\u{ff61}'..='\u{ff9d}' => output.push_str(halfwidth_katakana(character)),
            _ => output.push(character),
        }
    }
    output
}

fn halfwidth_katakana(character: char) -> &'static str {
    const FULLWIDTH: [&str; 63] = [
        "。", "「", "」", "、", "・", "ヲ", "ァ", "ィ", "ゥ", "ェ", "ォ", "ャ", "ュ", "ョ", "ッ",
        "ー", "ア", "イ", "ウ", "エ", "オ", "カ", "キ", "ク", "ケ", "コ", "サ", "シ", "ス", "セ",
        "ソ", "タ", "チ", "ツ", "テ", "ト", "ナ", "ニ", "ヌ", "ネ", "ノ", "ハ", "ヒ", "フ", "ヘ",
        "ホ", "マ", "ミ", "ム", "メ", "モ", "ヤ", "ユ", "ヨ", "ラ", "リ", "ル", "レ", "ロ", "ワ",
        "ン", "ﾞ", "ﾟ",
    ];
    FULLWIDTH[character as usize - '\u{ff61}' as usize]
}

fn compose_katakana(base: char, mark: char) -> Option<char> {
    let voiced = "ウカキクケコサシスセソタチツテトハヒフヘホ";
    let voiced_result = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボ";
    let semi_voiced = "ハヒフヘホ";
    let semi_voiced_result = "パピプペポ";
    let (bases, results) = if mark == '\u{ff9e}' {
        (voiced, voiced_result)
    } else {
        (semi_voiced, semi_voiced_result)
    };
    bases
        .chars()
        .position(|candidate| candidate == base)
        .and_then(|index| results.chars().nth(index))
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgb, RgbImage};

    use super::{pillow_grayscale, post_process, resize_bilinear};

    #[test]
    fn pillow_image_processing_matches_reference() {
        let image = RgbImage::from_raw(
            3,
            2,
            vec![
                255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 128, 64, 32,
            ],
        )
        .unwrap();
        let grayscale = pillow_grayscale(&DynamicImage::ImageRgb8(image));
        assert_eq!(
            grayscale.pixels().copied().collect::<Vec<_>>(),
            [76, 150, 29, 255, 0, 79].map(|value| Rgb([value; 3]))
        );
        assert_eq!(
            resize_bilinear(&grayscale, 2, 2)
                .unwrap()
                .pixels()
                .copied()
                .collect::<Vec<_>>(),
            [104, 74, 159, 49].map(|value| Rgb([value; 3]))
        );
    }

    #[test]
    fn post_processing_matches_manga_ocr() {
        assert_eq!(post_process(" A 12…・・. ｶﾞﾊﾟ "), "Ａ１２．．．．．．ガパ");
        assert_eq!(post_process("・ ﾞ"), "・ﾞ");
    }
}
