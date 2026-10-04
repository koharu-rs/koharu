//! RF-DETR image preprocessing and instance segmentation postprocessing.
//!
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/detr.py
//! https://github.com/roboflow/rf-detr/blob/4ab7c18729de9d02ffd0495795d0831b5630f01b/src/rfdetr/models/postprocess.py

use anyhow::{Result, bail, ensure};
use clap::ValueEnum;
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, Rgb, RgbImage, imageops};
use koharu_torch::{Device, IndexOp, Kind, Tensor};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    config::{KoharuLayoutRFDetrSeg2XLConfig, KoharuLayoutThresholds},
    model::Output,
};

/// Where a page is placed inside the fixed square model input.
///
/// This is an inference policy rather than a checkpoint property: RF-DETR's
/// backbone asserts a square input, so every page geometry has to be mapped
/// onto one, and the mapping changes the input distribution the checkpoint
/// sees. It therefore travels with the inference call instead of arriving
/// through the checkpoint's configuration.
///
/// The variant names are the single spelling shared by the TOML configuration,
/// the generated TypeScript protocol and the command line, so a user never has
/// to learn two words for one setting.
#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize, Type, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum InputFit {
    /// Scale by a single isotropic gain and centre the content, padding the
    /// remainder. Preserves panel and speech-bubble proportions, at the cost of
    /// spending model input on margins.
    #[default]
    #[value(name = "letter_box")]
    LetterBox,
    /// Scale each axis independently to fill the square. Spends every model
    /// pixel on the page but distorts geometry, which is what the checkpoint
    /// was trained on.
    #[value(name = "stretch")]
    Stretch,
}

/// Letterbox margin value, shared with the YOLO layout path.
///
/// 114 is the Ultralytics pad constant and normalizes to roughly the ImageNet
/// mean, so margins reach the backbone as neutral pixels instead of the
/// out-of-distribution black or white frame a page-derived colour would create.
const PADDING_VALUE: u8 = 114;

/// The single mapping between model-input pixels and source-image pixels.
///
/// RF-DETR predicts boxes normalized against its fixed square input, so the fit
/// geometry is derived once during preprocessing and inverted exactly once
/// during postprocessing. Deriving it independently on both sides is what
/// silently misplaces every box once a gain and an offset exist.
#[derive(Debug, Clone, Copy)]
pub(super) struct FitTransform {
    resolution: u32,
    /// Model-input pixels per source-image pixel, per axis.
    gain_x: f64,
    gain_y: f64,
    /// Model-input pixels preceding the first source-image column and row.
    offset_x: u32,
    offset_y: u32,
    content_width: u32,
    content_height: u32,
    original_width: u32,
    original_height: u32,
}

impl FitTransform {
    fn new(
        original_width: u32,
        original_height: u32,
        resolution: i64,
        input_fit: InputFit,
    ) -> Result<Self> {
        if original_width == 0 || original_height == 0 {
            bail!("cannot segment an empty image");
        }
        let resolution = u32::try_from(resolution)?;
        if resolution == 0 {
            bail!("RF-DETR resolution must be positive");
        }
        let (content_width, content_height, offset_x, offset_y) = match input_fit {
            InputFit::LetterBox => {
                let gain = f64::from(resolution) / f64::from(original_width.max(original_height));
                let content_width =
                    python_round(f64::from(original_width) * gain).clamp(1, i64::from(resolution));
                let content_height =
                    python_round(f64::from(original_height) * gain).clamp(1, i64::from(resolution));
                let content_width = content_width as u32;
                let content_height = content_height as u32;
                // Centre the page so it sits inside the receptive field instead
                // of hugging two borders of the square input.
                let offset_x = python_round(f64::from(resolution - content_width) / 2.0) as u32;
                let offset_y = python_round(f64::from(resolution - content_height) / 2.0) as u32;
                (content_width, content_height, offset_x, offset_y)
            }
            InputFit::Stretch => (resolution, resolution, 0, 0),
        };
        Ok(Self {
            resolution,
            gain_x: f64::from(content_width) / f64::from(original_width),
            gain_y: f64::from(content_height) / f64::from(original_height),
            offset_x,
            offset_y,
            content_width,
            content_height,
            original_width,
            original_height,
        })
    }

    fn page_axis(&self, normalized: f32, offset: u32, gain: f64, limit: u32) -> f64 {
        ((f64::from(normalized) * f64::from(self.resolution) - f64::from(offset)) / gain)
            .clamp(0.0, f64::from(limit))
    }

    /// Maps one normalized model-space box to clamped page pixels, or `None`
    /// when the box collapses.
    ///
    /// Clamping is what keeps a prediction that fired inside a letterbox margin
    /// from surfacing as a negative page coordinate. Collapse is decided on the
    /// rounded extent because integer page coordinates are the only geometry a
    /// consumer can slice, so a sub-pixel or inverted box has no valid use.
    fn to_image(&self, normalized_xyxy: [f32; 4]) -> Option<[f32; 4]> {
        if normalized_xyxy.iter().any(|value| !value.is_finite()) {
            return None;
        }
        let x1 = self.page_axis(
            normalized_xyxy[0],
            self.offset_x,
            self.gain_x,
            self.original_width,
        );
        let y1 = self.page_axis(
            normalized_xyxy[1],
            self.offset_y,
            self.gain_y,
            self.original_height,
        );
        let x2 = self.page_axis(
            normalized_xyxy[2],
            self.offset_x,
            self.gain_x,
            self.original_width,
        );
        let y2 = self.page_axis(
            normalized_xyxy[3],
            self.offset_y,
            self.gain_y,
            self.original_height,
        );
        let left = python_round(x1).clamp(0, i64::from(self.original_width));
        let top = python_round(y1).clamp(0, i64::from(self.original_height));
        let right = python_round(x2).clamp(0, i64::from(self.original_width));
        let bottom = python_round(y2).clamp(0, i64::from(self.original_height));
        if right <= left || bottom <= top {
            return None;
        }
        Some([x1 as f32, y1 as f32, x2 as f32, y2 as f32])
    }
}

#[derive(Debug, Clone)]
pub struct KoharuLayoutRFDetrImageProcessor {
    resolution: i64,
    num_select: i64,
    class_names: Vec<String>,
    recommended_thresholds: KoharuLayoutThresholds,
}

impl KoharuLayoutRFDetrImageProcessor {
    pub fn new(config: &KoharuLayoutRFDetrSeg2XLConfig) -> Result<Self> {
        ensure!(config.resolution > 0, "RF-DETR resolution must be positive");
        ensure!(
            config.resolution % 24 == 0,
            "RF-DETR resolution must be divisible by patch_size * num_windows"
        );
        ensure!(config.num_select > 0, "RF-DETR num_select must be positive");
        Ok(Self {
            resolution: config.resolution,
            num_select: config.num_select,
            class_names: config.class_names(),
            recommended_thresholds: config.recommended_thresholds,
        })
    }

    pub(super) fn recommended_thresholds(&self) -> KoharuLayoutThresholds {
        self.recommended_thresholds
    }

    pub(super) fn preprocess(
        &self,
        image: &DynamicImage,
        device: Device,
        input_fit: InputFit,
    ) -> Result<(Tensor, FitTransform)> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "cannot segment an empty image"
        );
        let image = image.to_rgb8();
        let transform =
            FitTransform::new(image.width(), image.height(), self.resolution, input_fit)?;
        // Fitting on the host keeps a 14317-pixel webtoon page from uploading
        // tens of megabytes only to be shrunk on the accelerator, and lets both
        // fit strategies share one resize so an A/B comparison isolates the
        // geometry policy instead of confounding it with the interpolation
        // kernel.
        let canvas = fit_canvas(&image, &transform)?;
        let pixel_values = Tensor::from_slice(canvas.as_raw())
            .view([
                1,
                i64::from(transform.resolution),
                i64::from(transform.resolution),
                3,
            ])
            .permute([0, 3, 1, 2])
            .to_device(device)
            .to_kind(Kind::Float)
            / 255.0;
        let mean = Tensor::from_slice(&[0.485f32, 0.456, 0.406])
            .view([1, 3, 1, 1])
            .to_device(device);
        let std = Tensor::from_slice(&[0.229f32, 0.224, 0.225])
            .view([1, 3, 1, 1])
            .to_device(device);
        Ok(((pixel_values - mean) / std, transform))
    }

    pub(super) fn postprocess(
        &self,
        output: &Output,
        transform: &FitTransform,
        thresholds: KoharuLayoutThresholds,
    ) -> Result<KoharuLayoutDetections> {
        let image_width = transform.original_width;
        let image_height = transform.original_height;
        validate_thresholds(thresholds)?;
        let logits_size = output.pred_logits.size();
        ensure!(
            logits_size == [1, 300, 5],
            "unexpected RF-DETR logits shape {logits_size:?}"
        );
        ensure!(
            output.pred_boxes.size() == [1, 300, 4],
            "unexpected RF-DETR box shape {:?}",
            output.pred_boxes.size()
        );
        ensure!(
            output.pred_masks.size() == [1, 300, 288, 288],
            "unexpected RF-DETR mask shape {:?}",
            output.pred_masks.size()
        );

        // PostProcess ranks every query/class pair, including the checkpoint's
        // fifth background logit slot, before applying the caller threshold.
        let (scores, indexes) =
            output
                .pred_logits
                .sigmoid()
                .view([1, -1])
                .topk(self.num_select, 1, true, true);
        let scores = scores.i(0);
        let indexes = indexes.i(0);
        let query_indexes = indexes.floor_divide_scalar(5);
        let labels = indexes.remainder(5);
        let thresholds = Tensor::from_slice(&thresholds.with_background())
            .to_device(scores.device())
            .index_select(0, &labels);
        let keep = scores.gt_tensor(&thresholds);
        let selected = keep.nonzero().view([-1]);

        if selected.size()[0] == 0 {
            return Ok(KoharuLayoutDetections {
                image_width,
                image_height,
                detections: Vec::new(),
            });
        }

        let scores = scores.index_select(0, &selected);
        let labels = labels.index_select(0, &selected);
        let query_indexes = query_indexes.index_select(0, &selected);
        let boxes = output.pred_boxes.i(0).index_select(0, &query_indexes);
        let center = boxes.i((.., 0..2));
        let half_size = boxes.i((.., 2..4)) / 2.0;
        // Boxes stay in normalized model-input space here; `FitTransform` owns
        // the single inversion into page pixels.
        let boxes = Tensor::cat(&[&center - &half_size, &center + &half_size], 1);

        // Gathering before interpolation is output-equivalent to upstream and
        // avoids allocating resized masks for candidates rejected by threshold.
        let masks = output.pred_masks.i(0).index_select(0, &query_indexes);

        let floats = Tensor::cat(&[scores.unsqueeze(1), boxes], 1);
        let floats = tensor_to_vec_f32(&floats)?;
        let labels = tensor_to_vec_i64(&labels)?;

        let mut detections = Vec::with_capacity(labels.len());
        for (index, label) in labels.into_iter().enumerate() {
            let row = index * 5;
            let Some(bbox) = transform.to_image([
                floats[row + 1],
                floats[row + 2],
                floats[row + 3],
                floats[row + 4],
            ]) else {
                continue;
            };
            // RF-DETR defines masks by bilinearly projecting each native mask to
            // the source image and thresholding at zero. Resolve one mask at a
            // time to preserve that exact contract without retaining an
            // N-by-page tensor, then keep only its non-zero page-space extent.
            let mask = masks
                .i(index as i64)
                .unsqueeze(0)
                .unsqueeze(0)
                .upsample_bilinear2d(
                    [i64::from(image_height), i64::from(image_width)],
                    false,
                    None,
                    None,
                )
                .gt(0.0)
                .to_kind(Kind::Uint8);
            let rows = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 3][..])
                .gt(0)
                .to_kind(Kind::Int64);
            let columns = mask
                .count_nonzero_dim_intlist(&[0i64, 1, 2][..])
                .gt(0)
                .to_kind(Kind::Int64);
            let occupied = Tensor::stack(
                &[
                    mask.count_nonzero(None),
                    columns.argmax(None, false),
                    rows.argmax(None, false),
                    i64::from(image_width) - columns.flip([0]).argmax(None, false),
                    i64::from(image_height) - rows.flip([0]).argmax(None, false),
                ],
                0,
            );
            let occupied = tensor_to_vec_i64(&occupied)?;
            let area = occupied[0].clamp(0, i64::from(u32::MAX)) as u32;
            let (x, y, right, bottom) = if area == 0 {
                (0, 0, 0, 0)
            } else {
                (
                    occupied[1] as u32,
                    occupied[2] as u32,
                    occupied[3] as u32,
                    occupied[4] as u32,
                )
            };
            let width = right.saturating_sub(x);
            let height = bottom.saturating_sub(y);
            let pixels = if width == 0 || height == 0 {
                Vec::new()
            } else {
                tensor_to_vec_u8(
                    &(mask.i((
                        0,
                        0,
                        i64::from(y)..i64::from(bottom),
                        i64::from(x)..i64::from(right),
                    )) * 255),
                )?
            };
            let label_id = label as usize;
            detections.push(KoharuLayoutDetection {
                label_id,
                label: self
                    .class_names
                    .get(label_id)
                    .cloned()
                    .unwrap_or_else(|| "__background__".to_owned()),
                score: floats[row],
                bbox,
                area,
                mask: KoharuLayoutMask {
                    x,
                    y,
                    width,
                    height,
                    pixels,
                },
            });
        }

        Ok(KoharuLayoutDetections {
            image_width,
            image_height,
            detections,
        })
    }
}

/// Places the resized page inside the square model input.
///
/// A zero offset means the content already fills the canvas, so the square is
/// returned without allocating a second full-size buffer.
fn fit_canvas(image: &RgbImage, transform: &FitTransform) -> Result<RgbImage> {
    let content = resize(image, transform.content_width, transform.content_height)?;
    if transform.offset_x == 0 && transform.offset_y == 0 {
        return Ok(content);
    }
    let mut canvas = RgbImage::from_pixel(
        transform.resolution,
        transform.resolution,
        Rgb([PADDING_VALUE; 3]),
    );
    imageops::replace(
        &mut canvas,
        &content,
        i64::from(transform.offset_x),
        i64::from(transform.offset_y),
    );
    Ok(canvas)
}

fn resize(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    if image.width() == width && image.height() == height {
        return Ok(image.clone());
    }
    // Downsampling needs a support-scaled kernel. RF-DETR training used a plain
    // bilinear Albumentations resize, but on a webtoon page that resize folds
    // thousands of rows into one and turns panel borders into aliasing noise,
    // so downsampling uses the antialiased triangle kernel that
    // torchvision `antialias=True` and torchvision's PIL `BILINEAR` mode both
    // select. This is a deliberate divergence from the training resize and must
    // be re-validated against detection quality, not assumed to help.
    let algorithm = if width < image.width() || height < image.height() {
        ResizeAlg::Convolution(FilterType::Bilinear)
    } else {
        ResizeAlg::Interpolation(FilterType::Bilinear)
    };
    let mut resized = RgbImage::new(width, height);
    Resizer::new().resize(
        image,
        &mut resized,
        &ResizeOptions::new().resize_alg(algorithm).use_alpha(false),
    )?;
    Ok(resized)
}

/// Rounds half to even, matching the Python `round` the upstream letterbox and
/// scale-box arithmetic is defined against.
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

fn validate_thresholds(thresholds: KoharuLayoutThresholds) -> Result<()> {
    for (class, threshold) in [
        ("text", thresholds.text),
        ("onomatopoeia", thresholds.onomatopoeia),
        ("bubble", thresholds.bubble),
        ("panel", thresholds.panel),
    ] {
        ensure!(
            (0.0..=1.0).contains(&threshold),
            "{class} confidence threshold must be between 0 and 1"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct KoharuLayoutDetections {
    pub image_width: u32,
    pub image_height: u32,
    pub detections: Vec<KoharuLayoutDetection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KoharuLayoutDetection {
    pub label_id: usize,
    pub label: String,
    pub score: f32,
    pub bbox: [f32; 4],
    pub area: u32,
    #[serde(skip_serializing)]
    pub mask: KoharuLayoutMask,
}

#[derive(Debug, Clone)]
pub struct KoharuLayoutMask {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl KoharuLayoutMask {
    #[must_use]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let Some(local_x) = x.checked_sub(self.x) else {
            return false;
        };
        let Some(local_y) = y.checked_sub(self.y) else {
            return false;
        };
        if local_x >= self.width || local_y >= self.height {
            return false;
        }
        self.pixels
            .get(local_y as usize * self.width as usize + local_x as usize)
            .is_some_and(|value| *value != 0)
    }
}

fn tensor_to_vec_f32(tensor: &Tensor) -> Result<Vec<f32>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Float)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0.0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

fn tensor_to_vec_i64(tensor: &Tensor) -> Result<Vec<i64>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Int64)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

fn tensor_to_vec_u8(tensor: &Tensor) -> Result<Vec<u8>> {
    let tensor = tensor
        .to_device(Device::Cpu)
        .to_kind(Kind::Uint8)
        .contiguous();
    let length = tensor.numel();
    let mut values = vec![0; length];
    tensor.f_copy_data(&mut values, length)?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

    use super::{
        FitTransform, InputFit, KoharuLayoutThresholds, python_round, validate_thresholds,
    };

    const RESOLUTION: i64 = 1152;

    fn letterbox(width: u32, height: u32) -> FitTransform {
        FitTransform::new(width, height, RESOLUTION, InputFit::LetterBox).unwrap()
    }

    /// The forward half of the mapping, mirroring how `fit_canvas` places the
    /// page. Written out here rather than exposed so production code keeps a
    /// single direction to get wrong.
    fn to_model(transform: &FitTransform, page_xyxy: [f64; 4]) -> [f32; 4] {
        [
            ((page_xyxy[0] * transform.gain_x + f64::from(transform.offset_x))
                / f64::from(transform.resolution)) as f32,
            ((page_xyxy[1] * transform.gain_y + f64::from(transform.offset_y))
                / f64::from(transform.resolution)) as f32,
            ((page_xyxy[2] * transform.gain_x + f64::from(transform.offset_x))
                / f64::from(transform.resolution)) as f32,
            ((page_xyxy[3] * transform.gain_y + f64::from(transform.offset_y))
                / f64::from(transform.resolution)) as f32,
        ]
    }

    fn assert_close(actual: [f32; 4], expected: [f64; 4], tolerance: f64) {
        for (index, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (f64::from(actual) - expected).abs() <= tolerance,
                "component {index}: {actual} differs from {expected} by more than {tolerance}",
            );
        }
    }

    #[test]
    fn class_thresholds_must_be_probabilities() {
        let thresholds = KoharuLayoutThresholds {
            text: 0.25,
            onomatopoeia: 1.1,
            bubble: 0.5,
            panel: 0.5,
        };

        let error = validate_thresholds(thresholds).unwrap_err();
        assert!(error.to_string().contains("onomatopoeia"));
    }

    #[test]
    fn letterbox_round_trips_boxes_when_shrinking() {
        for (width, height) in [(720, 14317), (14317, 720), (4000, 4000)] {
            let transform = letterbox(width, height);
            let page = [
                f64::from(width) * 0.13,
                f64::from(height) * 0.21,
                f64::from(width) * 0.87,
                f64::from(height) * 0.79,
            ];
            let mapped = to_model(&transform, page);
            let recovered = transform.to_image(mapped).expect("non-degenerate box");
            assert_close(recovered, page, 0.05);
        }
    }

    #[test]
    fn letterbox_round_trips_boxes_when_enlarging() {
        for (width, height) in [(300, 200), (64, 91)] {
            let transform = letterbox(width, height);
            assert!(transform.gain_x > 1.0 && transform.gain_y > 1.0);
            let page = [
                f64::from(width) * 0.25,
                f64::from(height) * 0.4,
                f64::from(width) * 0.75,
                f64::from(height) * 0.9,
            ];
            let mapped = to_model(&transform, page);
            let recovered = transform.to_image(mapped).expect("non-degenerate box");
            assert_close(recovered, page, 0.05);
        }
    }

    #[test]
    fn stretch_preserves_the_anisotropic_mapping() {
        let transform = FitTransform::new(720, 14317, RESOLUTION, InputFit::Stretch).unwrap();
        assert_eq!(
            (transform.offset_x, transform.offset_y),
            (0, 0),
            "stretch must leave the content flush with the square input"
        );
        // Anisotropic by construction: the two axes cannot share one gain.
        assert!((transform.gain_x - RESOLUTION as f64 / 720.0).abs() < 1e-9);
        assert!((transform.gain_y - RESOLUTION as f64 / 14317.0).abs() < 1e-9);
        assert_close(
            transform.to_image([0.1, 0.2, 0.3, 0.4]).unwrap(),
            [72.0, 2863.4, 216.0, 5726.8],
            0.001,
        );
    }

    #[test]
    fn square_pages_letterbox_to_a_pure_rescale() {
        let transform = letterbox(1000, 1000);
        assert_eq!((transform.offset_x, transform.offset_y), (0, 0));
        assert_eq!(
            (transform.content_width, transform.content_height),
            (1152, 1152)
        );
        assert_close(
            transform.to_image([0.25, 0.25, 0.75, 0.75]).unwrap(),
            [250.0, 250.0, 750.0, 750.0],
            0.001,
        );
    }

    #[test]
    fn square_pages_resolve_identically_under_both_fits() {
        // A square page has no padding to add, so letterboxing degenerates into
        // the same canvas the stretch fit builds. This is what lets a
        // letterbox-versus-stretch comparison isolate the geometry policy on
        // already-square input instead of confounding it with a resize.
        for side in [1000, 1152, 1600] {
            let letterboxed =
                FitTransform::new(side, side, RESOLUTION, InputFit::LetterBox).unwrap();
            let stretched = FitTransform::new(side, side, RESOLUTION, InputFit::Stretch).unwrap();
            assert_eq!(
                (
                    letterboxed.content_width,
                    letterboxed.content_height,
                    letterboxed.offset_x,
                    letterboxed.offset_y,
                    letterboxed.original_width,
                    letterboxed.original_height,
                ),
                (
                    stretched.content_width,
                    stretched.content_height,
                    stretched.offset_x,
                    stretched.offset_y,
                    stretched.original_width,
                    stretched.original_height,
                ),
                "{side}x{side} must not depend on the fit strategy",
            );
            let box_in = [0.1, 0.2, 0.9, 0.8];
            assert_eq!(
                letterboxed.to_image(box_in),
                stretched.to_image(box_in),
                "{side}x{side} round trip must not depend on the fit strategy",
            );
        }
    }

    #[test]
    fn letterbox_is_the_default_and_one_spelling_serves_every_surface() {
        assert_eq!(InputFit::default(), InputFit::LetterBox);
        for (spelling, expected) in [
            ("\"letter_box\"", InputFit::LetterBox),
            ("\"stretch\"", InputFit::Stretch),
        ] {
            assert_eq!(
                serde_json::from_str::<InputFit>(spelling).unwrap(),
                expected,
                "{spelling} must round-trip through the user configuration",
            );
        }
        // clap would otherwise spell these `letter-box` by default, leaving one
        // setting with two words depending on where it is written.
        assert_eq!(
            InputFit::value_variants()
                .iter()
                .map(|variant| {
                    variant
                        .to_possible_value()
                        .expect("a variantless enum always names every variant")
                        .get_name()
                        .to_owned()
                })
                .collect::<Vec<String>>(),
            ["letter_box", "stretch"],
        );
    }

    #[test]
    fn extreme_aspect_ratios_stay_finite() {
        for (width, height) in [(720, 14317), (14317, 720), (1000, 1000), (1, 14317)] {
            let transform = letterbox(width, height);
            for box_in in [
                [0.0, 0.0, 1.0, 1.0],
                [0.5, 0.5, 0.5, 0.5],
                [0.4747, 0.0, 0.5, 0.25],
                [0.0, 0.9999, 0.0001, 1.0],
            ] {
                let Some([x1, y1, x2, y2]) = transform.to_image(box_in) else {
                    continue;
                };
                for value in [x1, x2] {
                    assert!(
                        value.is_finite() && (0.0..=width as f32).contains(&value),
                        "{width}x{height} produced column {value}",
                    );
                }
                for value in [y1, y2] {
                    assert!(
                        value.is_finite() && (0.0..=height as f32).contains(&value),
                        "{width}x{height} produced row {value}",
                    );
                }
            }
        }
    }

    #[test]
    fn model_space_extremes_clamp_into_the_page() {
        for (width, height) in [(720, 14317), (14317, 720), (1000, 1000)] {
            let transform = letterbox(width, height);
            let mapped = transform.to_image([0.0, 0.0, 1.0, 1.0]).unwrap();
            assert_close(
                mapped,
                [0.0, 0.0, f64::from(width), f64::from(height)],
                0.001,
            );
            assert_close(
                transform.to_image([0.5, 0.5, 1.0, 1.0]).unwrap(),
                [
                    f64::from(width) / 2.0,
                    f64::from(height) / 2.0,
                    f64::from(width),
                    f64::from(height),
                ],
                0.001,
            );
            // The far corner is a zero-area box once clamped, so it is dropped
            // rather than reported as a degenerate point.
            assert!(transform.to_image([1.0, 1.0, 1.0, 1.0]).is_none());
        }
    }

    #[test]
    fn padding_predictions_never_leak_negative_coordinates() {
        // 720x14317 letterboxes to a 58-pixel-wide column, so most of the model
        // input is margin. Predictions fired there must still land on the page.
        let transform = letterbox(720, 14317);
        assert_eq!(transform.content_width, 58);
        assert_eq!(transform.offset_x, 547);
        let padding_ratio = f64::from(transform.offset_x) / f64::from(transform.resolution);
        assert!(padding_ratio > 0.4, "the margin must dominate this page");

        // Both column edges land in the margin, so the box collapses onto the
        // page edge and is dropped instead of surfacing a negative column.
        assert!(
            transform
                .to_image([0.0, 0.1, padding_ratio as f32, 0.9])
                .is_none()
        );

        // The span between the two margin edges is exactly the page: the inner
        // margin edge is column zero and the content's outer edge is the full
        // page width, which is what makes the round trip land back on the page.
        let content_edge = f64::from(transform.offset_x + transform.content_width)
            / f64::from(transform.resolution);
        assert_close(
            transform
                .to_image([padding_ratio as f32, 0.0, content_edge as f32, 1.0])
                .unwrap(),
            [0.0, 0.0, 720.0, 14317.0],
            1.0,
        );

        // A box starting inside the margin gains a clamped left edge rather than
        // a negative one, and keeps its content-side columns.
        let straddling = transform
            .to_image([0.0, 0.25, (padding_ratio + 0.2) as f32, 0.75])
            .unwrap();
        assert_eq!(straddling[0], 0.0, "the margin edge must clamp to column 0");
        assert!(straddling[2] > 0.0 && straddling[2] <= 720.0);
        for value in [straddling[1], straddling[3]] {
            assert!(value.is_finite() && (0.0..=14317.0).contains(&value));
        }
    }

    #[test]
    fn collapsed_and_inverted_boxes_are_dropped() {
        let transform = letterbox(1000, 1000);
        // Zero extent on each axis.
        assert!(transform.to_image([0.5, 0.2, 0.5, 0.8]).is_none());
        assert!(transform.to_image([0.2, 0.5, 0.8, 0.5]).is_none());
        // Sub-pixel extent rounds away.
        assert!(transform.to_image([0.5, 0.5, 0.5001, 0.9]).is_none());
        // Inverted corners cannot be salvaged by clamping.
        assert!(transform.to_image([0.8, 0.2, 0.4, 0.8]).is_none());
        assert!(transform.to_image([0.2, 0.8, 0.8, 0.4]).is_none());
        // Non-finite model output must not escape as a NaN page coordinate.
        assert!(transform.to_image([f32::NAN, 0.2, 0.8, 0.8]).is_none());
        assert!(transform.to_image([f32::INFINITY, 0.2, 0.8, 0.8]).is_none());
    }

    #[test]
    fn empty_pages_are_rejected_before_any_division() {
        assert!(FitTransform::new(0, 100, RESOLUTION, InputFit::LetterBox).is_err());
        assert!(FitTransform::new(100, 0, RESOLUTION, InputFit::Stretch).is_err());
    }

    #[test]
    fn python_round_matches_python_semantics() {
        assert_eq!(python_round(0.5), 0);
        assert_eq!(python_round(1.5), 2);
        assert_eq!(python_round(2.5), 2);
        assert_eq!(python_round(-0.5), 0);
        assert_eq!(python_round(4.4), 4);
        assert_eq!(python_round(4.6), 5);
    }
}
