//! Scene-to-PSD document projection.
//!
//! GIMP reference: layer traversal and reverse layer-record ordering:
//! https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-export.c#L1520-L1880
//! GIMP reference: `TySh` transforms and descriptors consumed by the importer:
//! https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-layer-res-load.c#L1345-L1438

use std::sync::Arc;

use image::{DynamicImage, GrayImage, Rgba, RgbaImage};
use koharu_rasterizer::{Raster, RasterOptions, Rasterizer};
use koharu_renderer::{
    Frame, ImageKind, LayerKind, TextAlign, TextMetadata as RenderedText, WritingMode,
};
use koharu_scene::{AssetRole, Snapshot};

use crate::{
    engine_data::{TextJustification, TextOrientation},
    error::PsdExportError,
};

const SOURCE_ROLE: &str = "source";
const TEXT_MASK_ROLE: &str = "text-mask";
const COO_MASK_ROLE: &str = "coo-mask";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLayerMode {
    Rasterized,
    Editable,
}

#[derive(Debug, Clone)]
pub struct PsdExportOptions {
    pub include_source: bool,
    pub include_raster_layers: bool,
    pub include_removal_mask: bool,
    pub text_layer_mode: TextLayerMode,
}

// GIMP reference: default layer visibility and normal-layer flags are serialized here:
// https://github.com/GNOME/gimp/blob/758fb4ed995bbb339282d3f777089a33f0a391b8/plug-ins/file-psd/psd-export.c#L1689-L1721
impl Default for PsdExportOptions {
    fn default() -> Self {
        Self {
            include_source: true,
            include_raster_layers: true,
            include_removal_mask: true,
            text_layer_mode: TextLayerMode::Editable,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Document {
    pub width: u32,
    pub height: u32,
    pub merged: RgbaImage,
    pub layers: Vec<Layer>,
}

#[derive(Debug, Clone)]
pub(crate) struct Layer {
    pub id: i32,
    pub name: String,
    pub left: i32,
    pub top: i32,
    pub pixels: RgbaImage,
    pub hidden: bool,
    pub text: Option<TextMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct TextMetadata {
    pub index: i32,
    pub text: String,
    pub bounds: [f64; 4],
    pub transform: [f64; 6],
    pub orientation: TextOrientation,
    pub justification: TextJustification,
    pub font_index: usize,
    pub font_set: Vec<String>,
    pub font_size: f64,
    pub color: [u8; 4],
    pub box_width: f64,
    pub box_height: f64,
}

pub(crate) async fn build(
    snapshot: &Snapshot,
    frame: &Frame,
    rasterizer: Arc<Rasterizer>,
    raster_options: RasterOptions,
    options: &PsdExportOptions,
) -> Result<Document, PsdExportError> {
    let merged = rasterize(Arc::clone(&rasterizer), frame, raster_options).await?;
    let width = merged.image.width();
    let height = merged.image.height();
    validate_dimensions(width, height)?;

    let source = read_image(snapshot, frame.page(), SOURCE_ROLE).await?;
    if options.include_source && source.is_none() {
        return Err(PsdExportError::MissingAsset {
            page: frame.page(),
            role: SOURCE_ROLE,
        });
    }
    let has_raster_layers = frame.layers().iter().any(|layer| {
        matches!(
            layer.kind(),
            LayerKind::Image(metadata)
                if matches!(metadata.kind, ImageKind::Cleanup | ImageKind::Paint)
        )
    });
    let mut layers = Vec::new();
    if let Some(source) = source.filter(|_| options.include_source) {
        push_raster_layer(
            &mut layers,
            "Original Image",
            source.to_rgba8(),
            options.include_raster_layers && has_raster_layers,
        )?;
    }
    if options.include_removal_mask {
        let mask = combine_masks(
            read_image(snapshot, frame.page(), TEXT_MASK_ROLE).await?,
            read_image(snapshot, frame.page(), COO_MASK_ROLE).await?,
        )?;
        if let Some(mask) = mask {
            push_raster_layer(
                &mut layers,
                "Text Removal Mask",
                grayscale_mask_rgba(&mask),
                true,
            )?;
        }
    }
    let text_entities = frame
        .layers()
        .iter()
        .filter_map(|layer| match layer.kind() {
            LayerKind::Text(text) => Some((layer.entity(), text)),
            LayerKind::Image(_) => None,
        })
        .filter(|(_, text)| !text.text.trim().is_empty())
        .collect::<Vec<_>>();
    let font_set = collect_fonts(text_entities.iter().map(|(_, text)| *text));
    // GIMP writes the application layer list in reverse so PSD records remain bottom-to-top.
    // Reversing the complete visual list preserves interleaving between text and raster layers.
    for visual in frame.layers().iter().rev() {
        let image = match visual.kind() {
            LayerKind::Image(image) => Some(image),
            LayerKind::Text(_) => None,
        };
        let raster =
            image.is_some_and(|image| matches!(image.kind, ImageKind::Cleanup | ImageKind::Paint));
        let ordinary_image = image.is_some_and(|image| image.kind == ImageKind::Embedded);
        if (raster && options.include_raster_layers) || ordinary_image {
            let cropped = frame
                .cropped(visual.entity())?
                .ok_or(PsdExportError::MissingRenderedEntity(visual.entity()))?;
            let rendered = rasterize(Arc::clone(&rasterizer), &cropped, raster_options).await?;
            let image = image.expect("image branch was selected above");
            let name = image.name.clone().unwrap_or_else(|| match image.kind {
                ImageKind::Source | ImageKind::Embedded => "Image".to_owned(),
                ImageKind::Cleanup => "Cleanup".to_owned(),
                ImageKind::Paint => "Paint".to_owned(),
            });
            validate_pixels(&name, &rendered.image)?;
            layers.push(Layer {
                id: 0,
                name,
                left: rendered.left,
                top: rendered.top,
                pixels: rendered.image,
                hidden: false,
                text: None,
            });
        } else if let Some(text) = match visual.kind() {
            LayerKind::Text(text) if !text.text.trim().is_empty() => Some(text),
            LayerKind::Text(_) | LayerKind::Image(_) => None,
        } {
            let offset = text_entities
                .iter()
                .position(|(entity, _)| *entity == visual.entity())
                .expect("nonempty visual text was collected before layer projection");
            let index =
                i32::try_from(offset + 1).map_err(|_| PsdExportError::TooManyLayers(offset + 1))?;
            let cropped = frame
                .cropped(visual.entity())?
                .ok_or(PsdExportError::MissingRenderedEntity(visual.entity()))?;
            let rendered = rasterize(Arc::clone(&rasterizer), &cropped, raster_options).await?;
            validate_pixels(&visual.entity().to_string(), &rendered.image)?;
            layers.push(Layer {
                id: 0,
                name: format!("TL {index:03} {}", visual.entity()),
                left: rendered.left,
                top: rendered.top,
                pixels: rendered.image,
                hidden: false,
                text: match options.text_layer_mode {
                    TextLayerMode::Rasterized => None,
                    TextLayerMode::Editable => Some(text_metadata(index, text, &font_set)),
                },
            });
        }
    }
    let layer_count = layers.len();
    if layer_count > i16::MAX as usize {
        return Err(PsdExportError::TooManyLayers(layer_count));
    }
    for (offset, layer) in layers.iter_mut().enumerate() {
        layer.id =
            i32::try_from(offset + 1).map_err(|_| PsdExportError::TooManyLayers(layer_count))?;
    }

    Ok(Document {
        width,
        height,
        merged: merged.image,
        layers,
    })
}

async fn rasterize(
    rasterizer: Arc<Rasterizer>,
    frame: &Frame,
    options: RasterOptions,
) -> Result<Raster, PsdExportError> {
    let frame = frame.raster_frame()?;
    tokio_rayon::spawn(move || rasterizer.rasterize(&frame, options))
        .await
        .map_err(PsdExportError::Rasterizer)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), PsdExportError> {
    if width == 0 || height == 0 || width > 30_000 || height > 30_000 {
        return Err(PsdExportError::UnsupportedDimensions { width, height });
    }
    Ok(())
}

async fn read_image(
    snapshot: &Snapshot,
    page: koharu_scene::EntityId,
    role: &'static str,
) -> Result<Option<DynamicImage>, PsdExportError> {
    let Some(asset) = snapshot.asset(page, &AssetRole::new(role)?)? else {
        return Ok(None);
    };
    let bytes = snapshot.read_blob(asset.blob).await?;
    Ok(Some(
        tokio_rayon::spawn(move || image::load_from_memory(&bytes)).await?,
    ))
}

fn combine_masks(
    left: Option<DynamicImage>,
    right: Option<DynamicImage>,
) -> Result<Option<DynamicImage>, PsdExportError> {
    let result = match (left, right) {
        (None, None) => None,
        (Some(mask), None) | (None, Some(mask)) => Some(mask),
        (Some(left), Some(right)) => {
            let mut left = left.into_luma8();
            let right = right.into_luma8();
            if left.dimensions() != right.dimensions() {
                return Err(PsdExportError::MismatchedMaskDimensions {
                    left_width: left.width(),
                    left_height: left.height(),
                    right_width: right.width(),
                    right_height: right.height(),
                });
            }
            for (target, source) in left.pixels_mut().zip(right.pixels()) {
                target.0[0] = target.0[0].max(source.0[0]);
            }
            Some(DynamicImage::ImageLuma8(left))
        }
    };
    Ok(result)
}

fn grayscale_mask_rgba(image: &DynamicImage) -> RgbaImage {
    let mask: GrayImage = image.to_luma8();
    let mut rgba = RgbaImage::new(mask.width(), mask.height());
    for (x, y, pixel) in mask.enumerate_pixels() {
        rgba.put_pixel(x, y, Rgba([pixel[0], pixel[0], pixel[0], 255]));
    }
    rgba
}

fn push_raster_layer(
    layers: &mut Vec<Layer>,
    name: &str,
    pixels: RgbaImage,
    hidden: bool,
) -> Result<(), PsdExportError> {
    validate_pixels(name, &pixels)?;
    layers.push(Layer {
        id: 0,
        name: name.to_owned(),
        left: 0,
        top: 0,
        pixels,
        hidden,
        text: None,
    });
    Ok(())
}

fn validate_pixels(layer: &str, pixels: &RgbaImage) -> Result<(), PsdExportError> {
    let width = pixels.width() as i32;
    let height = pixels.height() as i32;
    if width <= 0 || height <= 0 {
        return Err(PsdExportError::InvalidLayerBounds {
            layer: layer.to_owned(),
            width,
            height,
        });
    }
    Ok(())
}

fn collect_fonts<'a>(texts: impl Iterator<Item = &'a RenderedText>) -> Vec<String> {
    let mut fonts = Vec::new();
    for font in texts.flat_map(|text| &text.post_script_fonts) {
        if !fonts.iter().any(|candidate| candidate == font) {
            fonts.push(font.clone());
        }
    }
    fonts
}

fn text_metadata(index: i32, text: &RenderedText, font_set: &[String]) -> TextMetadata {
    let angle = f64::from(text.angle_degrees).to_radians();
    let bounds = text.layout_bounds;
    let primary_font = text.post_script_fonts.first();
    let font_index = primary_font
        .and_then(|font| font_set.iter().position(|candidate| candidate == font))
        .unwrap_or(0);
    TextMetadata {
        index,
        text: text.text.clone(),
        bounds: [
            f64::from(bounds.x),
            f64::from(bounds.y),
            f64::from(bounds.x + bounds.width),
            f64::from(bounds.y + bounds.height),
        ],
        transform: [
            angle.cos(),
            angle.sin(),
            -angle.sin(),
            angle.cos(),
            f64::from(bounds.x),
            f64::from(bounds.y),
        ],
        orientation: match text.writing_mode {
            WritingMode::Horizontal => TextOrientation::Horizontal,
            WritingMode::VerticalRl => TextOrientation::Vertical,
        },
        justification: match text.alignment {
            TextAlign::Left | TextAlign::Justify => TextJustification::Left,
            TextAlign::Center => TextJustification::Center,
            TextAlign::Right => TextJustification::Right,
        },
        font_index,
        font_set: font_set.to_vec(),
        font_size: f64::from(text.font_size),
        color: text.color,
        box_width: f64::from(bounds.width.max(1.0)),
        box_height: f64::from(bounds.height.max(1.0)),
    }
}

#[cfg(test)]
mod tests {
    use koharu_renderer::{RenderBounds, TextMetadata as RenderedText};

    use super::*;

    fn rendered_text(fonts: &[&str]) -> RenderedText {
        RenderedText {
            text: "Hello".to_owned(),
            language: None,
            rendered_bounds: RenderBounds {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 24.0,
            },
            layout_bounds: RenderBounds {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            },
            post_script_fonts: fonts.iter().map(|font| (*font).to_owned()).collect(),
            font_size: 24.0,
            color: [1, 2, 3, 255],
            alignment: TextAlign::Center,
            writing_mode: WritingMode::Horizontal,
            angle_degrees: 0.0,
        }
    }

    #[test]
    fn fonts_keep_first_resolved_order_without_duplicates() {
        let first = rendered_text(&["Primary", "Fallback"]);
        let second = rendered_text(&["Fallback", "Other"]);
        assert_eq!(
            collect_fonts([&first, &second].into_iter()),
            ["Primary", "Fallback", "Other"]
        );
    }

    #[test]
    fn text_metadata_uses_renderer_resolved_presentation() {
        let mut text = rendered_text(&["Primary"]);
        text.writing_mode = WritingMode::VerticalRl;
        text.alignment = TextAlign::Right;
        text.angle_degrees = 90.0;
        let metadata = text_metadata(3, &text, &["Primary".to_owned()]);
        assert_eq!(metadata.orientation, TextOrientation::Vertical);
        assert_eq!(metadata.justification, TextJustification::Right);
        assert_eq!(metadata.font_size, 24.0);
        assert!((metadata.transform[0]).abs() < 1e-12);
        assert!((metadata.transform[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn combine_masks_rejects_different_scene_asset_sizes() {
        let error = combine_masks(
            Some(DynamicImage::ImageLuma8(GrayImage::new(2, 2))),
            Some(DynamicImage::ImageLuma8(GrayImage::new(3, 2))),
        )
        .expect_err("mismatched masks must fail");
        assert!(matches!(
            error,
            PsdExportError::MismatchedMaskDimensions { .. }
        ));
    }
}
