//! manga-ocr-ar image preprocessing and text decoding, upstream's `preprocess`
//! and `MangaOCR.decode`:
//! https://huggingface.co/muscgab/manga-ocr-ar-experimental/blob/7873d8492307a021d6087b2c2b5f576a30b4acae/infer.py

use anyhow::{Result, ensure};
use fast_image_resize::{FilterType, ResizeAlg};
use image::{DynamicImage, Rgb, RgbImage, imageops};
use koharu_torch::{Device, Kind, Tensor};
use serde::Deserialize;

const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
// Upstream's digits, more than an F32 holds.
#[allow(clippy::excessive_precision)]
const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];
pub(super) const PATCH: i64 = 14;
/// Pillow's bicubic filter, whose `a = -0.5` is Catmull-Rom, as convolution.
const RESIZE: ResizeAlg = ResizeAlg::Convolution(FilterType::CatmullRom);

/// One crop as patches of a grid, padded to the patch budget, on the host so
/// a batch moves to the device once.
#[derive(Debug)]
pub(super) struct Crop {
    /// `(budget, 3 * PATCH^2)`, in patch order row, column, and in each patch
    /// channel, pixel row, pixel column.
    patches: Tensor,
    valid: Tensor,
    rows: Tensor,
    cols: Tensor,
    grid: (i64, i64),
}

/// A batch of crops on the device.
#[derive(Debug)]
pub(super) struct Batch {
    pub(super) patches: Tensor,
    pub(super) valid: Tensor,
    pub(super) rows: Tensor,
    pub(super) cols: Tensor,
    pub(super) heights: Tensor,
    pub(super) widths: Tensor,
    /// The longest grid side, upstream's `max(rows.max(), cols.max()) + 1`,
    /// read without a sync from the device.
    pub(super) side: i64,
}

impl Batch {
    pub(super) fn new(crops: &[Crop], device: Device) -> koharu_torch::Result<Self> {
        let stack = |field: fn(&Crop) -> &Tensor| {
            Tensor::stack(&crops.iter().map(field).collect::<Vec<_>>(), 0)?.to(device)
        };
        let side = |field: fn(&Crop) -> i64| {
            Tensor::from_slice(&crops.iter().map(field).collect::<Vec<_>>())?.to(device)
        };
        Ok(Self {
            patches: stack(|crop| &crop.patches)?,
            valid: stack(|crop| &crop.valid)?,
            rows: stack(|crop| &crop.rows)?,
            cols: stack(|crop| &crop.cols)?,
            heights: side(|crop| crop.grid.0)?,
            widths: side(|crop| crop.grid.1)?,
            side: crops
                .iter()
                .map(|crop| crop.grid.0.max(crop.grid.1))
                .fold(0, i64::max),
        })
    }
}

/// The largest uniform scale whose patch grid fits `budget`, and that grid.
fn pad_grid(height: i64, width: i64, budget: i64) -> (f64, i64, i64) {
    let cells = |size: i64, scale: f64| (size as f64 * scale / PATCH as f64).ceil() as i64;
    let (mut lo, mut hi) = (1e-4, 1000.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if cells(height, mid) * cells(width, mid) <= budget {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo, cells(height, lo), cells(width, lo))
}

/// The aspect-kept image at the top left of its patch grid, the rest filled
/// with its border's median color.
pub(super) fn preprocess(image: &DynamicImage, budget: i64) -> Result<Crop> {
    ensure!(
        image.width() > 0 && image.height() > 0,
        "cannot recognize an empty image"
    );
    let image = image.to_rgb8();
    let (scale, height, width) =
        pad_grid(i64::from(image.height()), i64::from(image.width()), budget);
    // Python's `round` takes ties to even.
    let scaled = |size: u32, limit: i64| {
        ((f64::from(size) * scale).round_ties_even() as i64)
            .min(limit)
            .max(1) as u32
    };
    let small = crate::image::resize(
        &image,
        scaled(image.width(), width * PATCH),
        scaled(image.height(), height * PATCH),
        RESIZE,
    )?;

    let mut canvas = RgbImage::from_pixel(
        (width * PATCH) as u32,
        (height * PATCH) as u32,
        Rgb(border_median(&small)),
    );
    imageops::replace(&mut canvas, &small, 0, 0);

    let cells = height * width;
    let pixels = Tensor::from_slice(canvas.as_raw())?
        .view([height * PATCH, width * PATCH, 3])?
        .to_kind(Kind::Float)?
        .div_scalar(255.0)?
        .sub_tensor(&Tensor::from_slice(&MEAN)?, 1)?
        .div_tensor(&Tensor::from_slice(&STD)?)?;
    let patches = pixels
        .reshape([height, PATCH, width, PATCH, 3])?
        .permute([0, 2, 4, 1, 3])?
        .reshape([cells, 3 * PATCH * PATCH])?;
    let padding = Tensor::zeros(
        [budget - cells, 3 * PATCH * PATCH],
        (Kind::Float, Device::Cpu),
    )?;

    let valid = (0..budget).map(|i| i < cells).collect::<Vec<_>>();
    let rows = (0..budget)
        .map(|i| if i < cells { i / width } else { 0 })
        .collect::<Vec<_>>();
    let cols = (0..budget)
        .map(|i| if i < cells { i % width } else { 0 })
        .collect::<Vec<_>>();
    Ok(Crop {
        patches: Tensor::cat(&[patches, padding], 0)?,
        valid: Tensor::from_slice(&valid)?,
        rows: Tensor::from_slice(&rows)?,
        cols: Tensor::from_slice(&cols)?,
        grid: (height, width),
    })
}

/// Each channel's median over the edge rows and columns, corners counted
/// twice as upstream's concatenation does. The count is always even, and
/// numpy's midpoint truncates to `u8`.
fn border_median(image: &RgbImage) -> [u8; 3] {
    let (width, height) = image.dimensions();
    let border = (0..width)
        .flat_map(|x| [(x, 0), (x, height - 1)])
        .chain((0..height).flat_map(|y| [(0, y), (width - 1, y)]))
        .map(|(x, y)| image.get_pixel(x, y).0)
        .collect::<Vec<_>>();
    std::array::from_fn(|channel| {
        let mut values = border
            .iter()
            .map(|pixel| pixel[channel])
            .collect::<Vec<_>>();
        values.sort_unstable();
        let middle = values.len() / 2;
        ((u16::from(values[middle - 1]) + u16::from(values[middle])) / 2) as u8
    })
}

/// The characters the decoder writes: ids past them are the end token and,
/// input only, the start token.
#[derive(Debug, Deserialize)]
pub(super) struct Vocabulary {
    characters: Vec<String>,
}

impl Vocabulary {
    pub(super) fn len(&self) -> i64 {
        self.characters.len() as i64
    }

    /// The text up to the first end token.
    pub(super) fn decode(&self, token_ids: &[i64]) -> String {
        token_ids
            .iter()
            .take_while(|&&id| id < self.len())
            .map(|&id| self.characters[id as usize].as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_matches_upstream_pad_grid() {
        // Heights and widths with the grids upstream's `pad_grid` gives them.
        for (height, width, grid) in [
            (53, 280, (8, 40)),
            (4, 6, (14, 21)),
            (20, 2000, (2, 162)),
            (2000, 20, (162, 2)),
            (1, 1, (18, 18)),
        ] {
            let (_, rows, cols) = pad_grid(height, width, 324);
            assert_eq!((rows, cols), grid);
        }
    }
}
