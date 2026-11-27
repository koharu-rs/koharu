//! YuzuMarker.FontDetection: which of 6,150 CJK fonts set a crop of text,
//! as a torchvision ResNet-50 names it, read here as how likely each broad
//! style of font, its kernel, is: the fonts' probabilities summed by the
//! kernel table published beside the weights.
//!
//! Upstream:
//! https://github.com/JeffersonQin/YuzuMarker.FontDetection/tree/0a94e165fe2b08d2800b723290eabd120b2d3d58
//! https://huggingface.co/mayocream/yuzumarker-fontdetection/tree/c58f09e7151904f5b6d3f1e6c5678558a16d5ea9

mod model;

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg};
use image::RgbImage;
use koharu_runtime::{Library, PinnedFile, Repository, Turn};
use koharu_torch::{Device, Kind, Tensor};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use self::model::Model;

const REPOSITORY: Repository = Repository::model(
    "mayocream/yuzumarker-fontdetection",
    "c58f09e7151904f5b6d3f1e6c5678558a16d5ea9",
);

pub const FILES: &[PinnedFile] = &[
    PinnedFile::new(REPOSITORY, "model.safetensors", 144_778_040),
    PinnedFile::new(REPOSITORY, "kernels.json", 64_414),
];

/// `config.FONT_COUNT`: the first outputs, one per font.
const FONTS: usize = 6150;
/// demo.py's `transforms.Resize((512, 512))`, which stretches a crop to a
/// square.
const SIZE: u32 = 512;
/// Pillow's bilinear filter, which torchvision's `Resize` calls on a PIL
/// image.
const RESIZE: ResizeAlg = ResizeAlg::Convolution(FilterType::Bilinear);
/// The crops in one pass. Upstream's demo passes one; a batch gives the same
/// outputs, since batch normalization reads its running statistics.
const BATCH: usize = 8;

/// A broad style of CJK font, as the kernel table names it, following
/// Morisawa's, Founder's, and DynaComware's classifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kernel {
    Antique,
    Mincho,
    Fangsong,
    Gothic,
    Rounded,
    Kaisho,
    Gyokai,
    Gyosho,
    Sosho,
    Reisho,
    Tensho,
    Kointai,
    Weibei,
    Textbook,
    Brush,
    Handwriting,
    Pop,
    Design,
    Decorative,
}

#[derive(Debug)]
pub struct YuzuMarkerFontDetection {
    device: Device,
    model: Model,
    /// Each font's kernel, by its output.
    kernels: Vec<Kernel>,
}

impl YuzuMarkerFontDetection {
    pub async fn load(device: &crate::Device) -> Result<Self> {
        let (device, kind) = crate::torch::device(device);
        let [weights, kernels] = REPOSITORY
            .resolve([FILES[0].filename, FILES[1].filename])
            .await
            .context("failed to resolve YuzuMarker's files")?;
        let kernels: Vec<Kernel> = serde_json::from_slice(&fs_err::read(&kernels)?)
            .context("failed to read YuzuMarker's kernel table")?;
        ensure!(
            kernels.len() == FONTS,
            "the kernel table has {} fonts, but the network names {FONTS}",
            kernels.len()
        );
        let _turn = Turn::take(Library::Torch);
        let mut model = Model::new(device, kind)?;
        model.load(&weights)?;
        Ok(Self {
            device,
            model,
            kernels,
        })
    }

    /// Each crop's kernels, with their probabilities: demo.py's
    /// `recognize_font` softmax over the font logits, summed by kernel.
    pub fn inference(&self, crops: &[RgbImage]) -> Result<Vec<BTreeMap<Kernel, f32>>> {
        let pixels = crops
            .par_iter()
            .map(|crop| {
                ensure!(
                    crop.width() > 0 && crop.height() > 0,
                    "cannot read the font of an empty crop"
                );
                Ok(crate::image::resize(crop, SIZE, SIZE, RESIZE)?.into_raw())
            })
            .collect::<Result<Vec<_>>>()?;
        let probabilities = crate::torch::inference(|| -> Result<Vec<f32>> {
            let mut probabilities = Vec::with_capacity(crops.len() * FONTS);
            for batch in pixels.chunks(BATCH) {
                // `ToTensor`: channels first, scaled to 0–1.
                let input = Tensor::from_slice(&batch.concat())?
                    .view([batch.len() as i64, i64::from(SIZE), i64::from(SIZE), 3])?
                    .to(self.device)?
                    .permute([0, 3, 1, 2])?
                    .to_kind(Kind::Float)?
                    .div_scalar(255.0)?;
                let fonts = self
                    .model
                    .forward(&input)?
                    .narrow(1, 0, FONTS as i64)?
                    .to_kind(Kind::Float)?
                    .softmax_int(1, None)?
                    .flatten_using_ints(0, -1)?
                    .to(Device::Cpu)?;
                probabilities.extend(Vec::<f32>::try_from(&fonts)?);
            }
            Ok(probabilities)
        })?;
        Ok(probabilities
            .chunks(FONTS)
            .map(|fonts| {
                let mut kernels = BTreeMap::new();
                for (&kernel, &probability) in self.kernels.iter().zip(fonts) {
                    *kernels.entry(kernel).or_insert(0.0) += probability;
                }
                kernels
            })
            .collect())
    }
}
